//! バイトコードからオブジェクトを復元する
use std::string::FromUtf8Error;

use erg_common::cache::CacheSet;
use erg_common::config::ErgConfig;
use erg_common::dict::Dict;
use erg_common::error::{ErrorCore, ErrorKind, Location, SubMessage};
use erg_common::python_util::PythonVersion;
use erg_common::serialize::DataTypePrefix;
use erg_common::traits::ExitStatus;
use erg_common::{fn_name, switch_lang};
use erg_common::{RcArray, Str};

use super::codeobj::CodeObj;
use super::constructors::array_t;
use super::typaram::TyParam;
use super::value::ValueObj;
use super::{HasType, Type};

#[derive(Debug)]
pub struct DeserializeError {
    pub errno: usize,
    pub caused_by: String,
    pub desc: String,
}

impl From<std::io::Error> for DeserializeError {
    fn from(err: std::io::Error) -> Self {
        Self::new(0, "io::Error::into", err.to_string())
    }
}

impl From<FromUtf8Error> for DeserializeError {
    fn from(err: FromUtf8Error) -> Self {
        Self::new(0, "Str::try_from", err.to_string())
    }
}

impl From<DeserializeError> for ErrorCore {
    fn from(err: DeserializeError) -> Self {
        ErrorCore::new(
            vec![SubMessage::only_loc(Location::Unknown)],
            err.desc,
            err.errno,
            ErrorKind::ImportError,
            Location::Unknown,
        )
    }
}

impl DeserializeError {
    pub fn new<S: Into<String>, T: Into<String>>(errno: usize, caused_by: S, desc: T) -> Self {
        Self {
            errno,
            caused_by: caused_by.into(),
            desc: desc.into(),
        }
    }

    pub fn file_broken_error() -> Self {
        Self::new(
            0,
            fn_name!(),
            switch_lang!(
                "japanese" => "読み込んだ.pycファイルは破損しています",
                "simplified_chinese" => "加载的.pyc文件已损坏",
                "traditional_chinese" => "加載的.pyc文件已損壞",
                "english" => "the loaded .pyc file is broken",
            ),
        )
    }

    pub fn type_error(expect: &Type, found: &Type) -> Self {
        Self::new(
            0,
            fn_name!(),
            switch_lang!(
                "japanese" => format!(
                    "{expect}型オブジェクトを予期しましたが、 読み込んだオブジェクトは{found}型です",
                ),
                "simplified_chinese" => format!(
                    "期望一个{expect}对象，但反序列化的对象是{found}",
                ),
                "traditional_chinese" => format!(
                    "期望一個{expect}對象，但反序列化的對像是{found}",
                ),
                "english" => format!(
                    "expect a {expect} object, but the deserialized object is {found}",
                ),
            ),
        )
    }
}

pub type DeserializeResult<T> = Result<T, DeserializeError>;

#[derive(Default)]
pub struct Deserializer {
    str_cache: CacheSet<str>,
    arr_cache: CacheSet<[ValueObj]>,
    _dict_cache: CacheSet<Dict<ValueObj, ValueObj>>,
}

impl Deserializer {
    pub fn new() -> Self {
        Self {
            str_cache: CacheSet::new(),
            arr_cache: CacheSet::new(),
            _dict_cache: CacheSet::new(),
        }
    }

    pub fn run(cfg: ErgConfig) -> ExitStatus {
        let Some(filename) = cfg.input.path() else {
            eprintln!("{:?} is not a filename", cfg.input);
            return ExitStatus::ERR1;
        };
        match CodeObj::from_pyc(filename) {
            Ok(codeobj) => {
                println!("{}", codeobj.code_info(None));
                ExitStatus::OK
            }
            Err(e) => {
                eprintln!(
                    "failed to deserialize {}: {}",
                    filename.to_string_lossy(),
                    e.desc
                );
                ExitStatus::ERR1
            }
        }
    }

    fn get_cached_str(&mut self, s: &str) -> ValueObj {
        ValueObj::Str(self.str_cache.get(s))
    }

    fn get_cached_arr(&mut self, arr: &[ValueObj]) -> ValueObj {
        ValueObj::Array(self.arr_cache.get(arr))
    }

    pub fn vec_to_bytes<const LEN: usize>(vector: Vec<u8>) -> [u8; LEN] {
        let mut arr = [0u8; LEN];
        for (arr_elem, vec_elem) in arr.iter_mut().zip(vector.iter()) {
            *arr_elem = *vec_elem;
        }
        arr
    }

    pub fn consume<const LEN: usize>(v: &mut Vec<u8>) -> [u8; LEN] {
        Self::vec_to_bytes::<LEN>(v.drain(..LEN).collect::<Vec<_>>())
    }

    pub fn deserialize_u32(v: &mut Vec<u8>) -> u32 {
        u32::from_le_bytes(Self::consume::<4>(v))
    }

    pub fn deserialize_const(
        &mut self,
        v: &mut Vec<u8>,
        python_ver: PythonVersion,
    ) -> DeserializeResult<ValueObj> {
        match DataTypePrefix::from(v.remove(0)) {
            DataTypePrefix::Int32 => {
                let bytes = Self::consume::<4>(v);
                Ok(ValueObj::Int(i32::from_le_bytes(bytes)))
            }
            DataTypePrefix::BinFloat => {
                let bytes = Self::consume::<8>(v);
                Ok(ValueObj::Float(f64::from_le_bytes(bytes)))
            }
            DataTypePrefix::ShortAscii | DataTypePrefix::ShortAsciiInterned => {
                let len = v.remove(0);
                let bytes = v.drain(..len as usize).collect();
                Ok(self.get_cached_str(&String::from_utf8(bytes)?))
            }
            DataTypePrefix::Str | DataTypePrefix::Unicode => {
                let len = Self::deserialize_u32(v);
                let bytes = v.drain(..len as usize).collect();
                Ok(self.get_cached_str(&String::from_utf8(bytes)?))
            }
            DataTypePrefix::True => Ok(ValueObj::Bool(true)),
            DataTypePrefix::False => Ok(ValueObj::Bool(false)),
            DataTypePrefix::SmallTuple => {
                let len = v.remove(0);
                let mut arr = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    arr.push(self.deserialize_const(v, python_ver)?);
                }
                Ok(self.get_cached_arr(&arr))
            }
            DataTypePrefix::Tuple => {
                let len = Self::deserialize_u32(v);
                let mut arr = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    arr.push(self.deserialize_const(v, python_ver)?);
                }
                Ok(self.get_cached_arr(&arr))
            }
            DataTypePrefix::Code => {
                let argcount = Self::deserialize_u32(v);
                let posonlyargcount = if python_ver.minor >= Some(8) {
                    Self::deserialize_u32(v)
                } else {
                    0
                };
                let kwonlyargcount = Self::deserialize_u32(v);
                let nlocals = if python_ver.minor < Some(11) {
                    Self::deserialize_u32(v)
                } else {
                    0
                };
                let stacksize = Self::deserialize_u32(v);
                let flags = Self::deserialize_u32(v);
                let code = self.deserialize_bytes(v)?;
                let consts = self.deserialize_const_vec(v, python_ver)?;
                let names = self.deserialize_str_vec(v, python_ver)?;
                let varnames = self.deserialize_str_vec(v, python_ver)?;
                let freevars = self.deserialize_str_vec(v, python_ver)?;
                let cellvars = self.deserialize_str_vec(v, python_ver)?;
                let filename = self.deserialize_str(v, python_ver)?;
                let name = self.deserialize_str(v, python_ver)?;
                let qualname = if python_ver.minor >= Some(11) {
                    self.deserialize_str(v, python_ver)?
                } else {
                    name.clone()
                };
                let firstlineno = Self::deserialize_u32(v);
                let lnotab = self.deserialize_bytes(v)?;
                let exceptiontable = if python_ver.minor >= Some(11) {
                    self.deserialize_bytes(v)?
                } else {
                    vec![]
                };
                Ok(ValueObj::from(CodeObj {
                    argcount,
                    posonlyargcount,
                    kwonlyargcount,
                    nlocals,
                    stacksize,
                    flags,
                    code,
                    consts,
                    names,
                    varnames,
                    freevars,
                    cellvars,
                    filename,
                    name,
                    qualname,
                    firstlineno,
                    lnotab,
                    exceptiontable,
                }))
            }
            DataTypePrefix::None => Ok(ValueObj::None),
            other => Err(DeserializeError::new(
                0,
                fn_name!(),
                switch_lang!(
                    "japanese" => format!("このオブジェクトは復元できません: {other}"),
                    "simplified_chinese" => format!("无法反序列化此对象: {other}"),
                    "traditional_chinese" => format!("無法反序列化此對象: {other}"),
                    "english" => format!("cannot deserialize this object: {other}"),
                ),
            )),
        }
    }

    pub fn deserialize_const_vec(
        &mut self,
        v: &mut Vec<u8>,
        python_ver: PythonVersion,
    ) -> DeserializeResult<Vec<ValueObj>> {
        match self.deserialize_const(v, python_ver)? {
            ValueObj::Array(arr) => Ok(arr.to_vec()),
            other => Err(DeserializeError::type_error(&Type::Str, other.ref_t())),
        }
    }

    pub fn deserialize_const_array(
        &mut self,
        v: &mut Vec<u8>,
        python_ver: PythonVersion,
    ) -> DeserializeResult<RcArray<ValueObj>> {
        match self.deserialize_const(v, python_ver)? {
            ValueObj::Array(arr) => Ok(arr),
            other => Err(DeserializeError::type_error(&Type::Str, other.ref_t())),
        }
    }

    pub fn array_into_const(&mut self, arr: &[ValueObj]) -> ValueObj {
        self.get_cached_arr(arr)
    }

    pub fn try_into_str(&mut self, c: ValueObj) -> DeserializeResult<Str> {
        match c {
            ValueObj::Str(s) => Ok(s),
            other => Err(DeserializeError::type_error(&Type::Str, other.ref_t())),
        }
    }

    pub fn deserialize_str_vec(
        &mut self,
        v: &mut Vec<u8>,
        python_ver: PythonVersion,
    ) -> DeserializeResult<Vec<Str>> {
        match self.deserialize_const(v, python_ver)? {
            ValueObj::Array(arr) => {
                let mut strs = Vec::with_capacity(arr.len());
                for c in arr.iter().cloned() {
                    strs.push(self.try_into_str(c)?);
                }
                Ok(strs)
            }
            other => Err(DeserializeError::type_error(
                &array_t(Type::Str, TyParam::erased(Type::Nat)),
                other.ref_t(),
            )),
        }
    }

    pub fn deserialize_str(
        &mut self,
        v: &mut Vec<u8>,
        python_ver: PythonVersion,
    ) -> DeserializeResult<Str> {
        match self.deserialize_const(v, python_ver)? {
            ValueObj::Str(s) => Ok(s),
            other => Err(DeserializeError::type_error(&Type::Str, other.ref_t())),
        }
    }

    pub fn deserialize_bytes(&self, v: &mut Vec<u8>) -> DeserializeResult<Vec<u8>> {
        if DataTypePrefix::from(v.remove(0)) != DataTypePrefix::Str {
            return Err(DeserializeError::new(
                0,
                fn_name!(),
                switch_lang!(
                    "japanese" => "バイト列の読み込みに失敗しました",
                    "simplified_chinese" => "未能加载字节",
                    "traditional_chinese" => "未能加載字節",
                    "english" => "failed to load bytes",
                ),
            ));
        }
        let len = Self::deserialize_u32(v);
        Ok(v.drain(0..len as usize).collect())
    }
}
