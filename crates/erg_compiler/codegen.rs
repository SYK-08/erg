//! generates `CodeObj` (equivalent to PyCodeObject of CPython) from `AST`.
//!
//! ASTからPythonバイトコード(コードオブジェクト)を生成する
use std::fmt;
use std::process;

use crate::ty::codeobj::{CodeObj, CodeObjFlags, MakeFunctionFlags};
use crate::ty::value::GenTypeObj;
use erg_common::cache::CacheSet;
use erg_common::config::ErgConfig;
use erg_common::env::erg_std_path;
use erg_common::error::{ErrorDisplay, Location};
use erg_common::fresh::SharedFreshNameGenerator;
use erg_common::io::Input;
use erg_common::opcode::{CommonOpcode, CompareOp};
use erg_common::opcode308::Opcode308;
use erg_common::opcode310::Opcode310;
use erg_common::opcode311::{BinOpCode, Opcode311};
use erg_common::option_enum_unwrap;
use erg_common::python_util::{env_python_version, PythonVersion};
use erg_common::traits::{Locational, Stream};
use erg_common::Str;
use erg_common::{debug_power_assert, fn_name, fn_name_full, impl_stream, log, switch_unreachable};
use erg_parser::ast::VisModifierSpec;
use erg_parser::ast::{DefId, DefKind};
use CommonOpcode::*;

use erg_parser::ast::{ParamPattern, TypeBoundSpecs, VarName};
use erg_parser::token::DOT;
use erg_parser::token::EQUAL;
use erg_parser::token::{Token, TokenKind};

use crate::compile::{AccessKind, Name, StoreLoadKind};
use crate::error::CompileError;
use crate::hir::{
    Accessor, Args, Array, BinOp, Block, Call, ClassDef, Def, DefBody, Expr, Identifier, Lambda,
    Literal, NonDefaultParamSignature, Params, PatchDef, PosArg, ReDef, Record, Signature,
    SubrSignature, Tuple, UnaryOp, VarSignature, HIR,
};
use crate::ty::value::ValueObj;
use crate::ty::{HasType, Type, TypeCode, TypePair, VisibilityModifier};
use crate::varinfo::VarInfo;
use AccessKind::*;
use Type::*;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlKind {
    If,
    While,
    For,
    Match,
    With,
    Discard,
    Assert,
}

/// patch method -> function
/// patch attr -> variable
fn debind(ident: &Identifier) -> Option<Str> {
    match ident.vi.py_name.as_ref().map(|s| &s[..]) {
        Some(name) if name.starts_with("Function::") => {
            Some(Str::from(name.replace("Function::", "")))
        }
        Some(patch_method) if patch_method.contains("::") || patch_method.contains('.') => {
            Some(Str::rc(patch_method))
        }
        _ => None,
    }
}

fn escape_name(name: &str, vis: &VisibilityModifier, def_line: u32, def_col: u32) -> Str {
    let name = name.replace('!', "__erg_proc__");
    let name = name.replace('$', "__erg_shared__");
    if vis.is_private() {
        let line_mangling = match (def_line, def_col) {
            (0, 0) => "".to_string(),
            (0, _) => format!("_C{def_col}"),
            (_, 0) => format!("_L{def_line}"),
            (_, _) => format!("_L{def_line}_C{def_col}"),
        };
        Str::from(format!("::{name}{line_mangling}"))
    } else {
        Str::from(name)
    }
}

fn escape_ident(ident: Identifier) -> Str {
    let vis = ident.vis();
    if &ident.inspect()[..] == "Self" {
        let Ok(ty) = <&Type>::try_from(ident.vi.t.singleton_value().unwrap()) else { unreachable!() };
        escape_name(
            &ty.local_name(),
            &ident.vi.vis.modifier,
            ident.vi.def_loc.loc.ln_begin().unwrap_or(0),
            ident.vi.def_loc.loc.col_begin().unwrap_or(0),
        )
    } else if let Some(py_name) = ident.vi.py_name {
        py_name
    } else {
        escape_name(
            ident.inspect(),
            vis,
            ident.vi.def_loc.loc.ln_begin().unwrap_or(0),
            ident.vi.def_loc.loc.col_begin().unwrap_or(0),
        )
    }
}

#[derive(Debug, Clone)]
pub struct PyCodeGenUnit {
    pub(crate) id: usize,
    pub(crate) py_version: PythonVersion,
    pub(crate) codeobj: CodeObj,
    pub(crate) tmp_codes: Vec<Vec<u8>>,
    pub(crate) stack_len: u32, // the maximum stack size
    pub(crate) prev_lineno: u32,
    pub(crate) lasti: usize,
    pub(crate) prev_lasti: usize,
    pub(crate) _refs: Vec<ValueObj>, // ref-counted objects
}

impl PartialEq for PyCodeGenUnit {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl fmt::Display for PyCodeGenUnit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "CompilerUnit{{\nid: {}\ncode:\n{}\n}}",
            self.id,
            self.codeobj.code_info(Some(self.py_version))
        )
    }
}

impl PyCodeGenUnit {
    pub fn new<S: Into<Str>, T: Into<Str>>(
        id: usize,
        py_version: PythonVersion,
        params: Vec<Str>,
        filename: S,
        name: T,
        firstlineno: u32,
        flags: u32,
    ) -> Self {
        Self {
            id,
            py_version,
            codeobj: CodeObj::empty(params, filename, name, firstlineno, flags),
            stack_len: 0,
            prev_lineno: firstlineno,
            lasti: 0,
            prev_lasti: 0,
            _refs: vec![],
            tmp_codes: vec![],
        }
    }
}

#[derive(Debug, Clone)]
pub struct PyCodeGenStack(Vec<PyCodeGenUnit>);

impl_stream!(PyCodeGenStack, PyCodeGenUnit);

#[derive(Debug, Default)]
pub struct PyCodeGenerator {
    cfg: ErgConfig,
    pub(crate) py_version: PythonVersion,
    str_cache: CacheSet<str>,
    prelude_loaded: bool,
    mutate_op_loaded: bool,
    in_op_loaded: bool,
    record_type_loaded: bool,
    module_type_loaded: bool,
    control_loaded: bool,
    convertors_loaded: bool,
    abc_loaded: bool,
    unit_size: usize,
    units: PyCodeGenStack,
    fresh_gen: SharedFreshNameGenerator,
}

impl PyCodeGenerator {
    pub fn new(cfg: ErgConfig) -> Self {
        Self {
            py_version: cfg.target_version.unwrap_or_else(env_python_version),
            cfg,
            str_cache: CacheSet::new(),
            prelude_loaded: false,
            mutate_op_loaded: false,
            in_op_loaded: false,
            record_type_loaded: false,
            module_type_loaded: false,
            control_loaded: false,
            convertors_loaded: false,
            abc_loaded: false,
            unit_size: 0,
            units: PyCodeGenStack::empty(),
            fresh_gen: SharedFreshNameGenerator::new("codegen"),
        }
    }

    pub fn inherit(&self) -> Self {
        Self {
            cfg: self.cfg.clone(),
            py_version: self.py_version,
            str_cache: self.str_cache.clone(),
            prelude_loaded: false,
            mutate_op_loaded: false,
            in_op_loaded: false,
            record_type_loaded: false,
            module_type_loaded: false,
            control_loaded: false,
            convertors_loaded: false,
            abc_loaded: false,
            unit_size: 0,
            units: PyCodeGenStack::empty(),
            fresh_gen: self.fresh_gen.clone(),
        }
    }

    pub fn clear(&mut self) {
        self.units.clear();
    }

    pub fn initialize(&mut self) {
        self.prelude_loaded = false;
        self.mutate_op_loaded = false;
        self.in_op_loaded = false;
        self.record_type_loaded = false;
        self.module_type_loaded = false;
        self.control_loaded = false;
        self.convertors_loaded = false;
        self.abc_loaded = false;
    }

    #[inline]
    fn input(&self) -> &Input {
        &self.cfg.input
    }

    fn get_cached(&self, s: &str) -> Str {
        self.str_cache.get(s)
    }

    #[inline]
    fn toplevel_block(&self) -> &PyCodeGenUnit {
        self.units.first().unwrap()
    }

    #[inline]
    fn cur_block(&self) -> &PyCodeGenUnit {
        self.units.last().unwrap()
    }

    #[inline]
    fn mut_cur_block(&mut self) -> &mut PyCodeGenUnit {
        self.units.last_mut().unwrap()
    }

    #[inline]
    fn cur_block_codeobj(&self) -> &CodeObj {
        &self.cur_block().codeobj
    }

    #[inline]
    fn mut_cur_block_codeobj(&mut self) -> &mut CodeObj {
        &mut self.mut_cur_block().codeobj
    }

    #[inline]
    fn toplevel_block_codeobj(&self) -> &CodeObj {
        &self.toplevel_block().codeobj
    }

    #[inline]
    fn stack_len(&self) -> u32 {
        self.cur_block().stack_len
    }

    #[inline]
    fn lasti(&self) -> usize {
        self.cur_block().lasti
    }

    #[inline]
    #[allow(dead_code)]
    fn emit_print_expr(&mut self) {
        self.write(Opcode311::PRINT_EXPR, 0);
        self.stack_dec();
    }

    fn _emit_compare_op(&mut self, op: CompareOp) {
        self.write(Opcode311::COMPARE_OP, op as usize);
        self.stack_dec();
        if self.py_version.minor >= Some(11) {
            self.write_bytes(&[0; 4]);
        }
    }

    /// shut down the interpreter
    #[allow(dead_code)]
    fn terminate(&mut self) {
        self.emit_push_null();
        self.emit_load_name_instr(Identifier::public("exit"));
        self.emit_load_const(1);
        if self.py_version.minor >= Some(11) {
            self.emit_precall_and_call(1);
        } else {
            self.write(Opcode310::CALL_FUNCTION, 1);
        }
        self.stack_dec();
    }

    /// swap TOS and TOS1
    fn rot2(&mut self) {
        if self.py_version.minor >= Some(11) {
            self.write(Opcode311::SWAP, 2);
        } else {
            self.write(Opcode310::ROT_TWO, 0);
        }
    }

    fn dup_top(&mut self) {
        if self.py_version.minor >= Some(11) {
            self.write(Opcode311::COPY, 1);
        } else {
            self.write(Opcode310::DUP_TOP, 0);
        }
        self.stack_inc();
    }

    /// COPY(1) == DUP_TOP
    fn copy(&mut self, i: usize) {
        debug_power_assert!(i, >, 0);
        if self.py_version.minor >= Some(11) {
            self.write(Opcode311::COPY, i);
        } else if i == 1 {
            self.write(Opcode310::DUP_TOP, 0);
        } else {
            todo!()
        }
        self.stack_inc();
    }

    /// 0 origin
    #[allow(dead_code)]
    fn peek_stack(&mut self, i: usize) {
        self.copy(i + 1);
        self.emit_print_expr();
    }

    #[inline]
    fn jump_delta(&self, jump_to: usize) -> usize {
        if self.py_version.minor >= Some(10) {
            if self.lasti() <= jump_to * 2 {
                3
            } else {
                0
            }
        } else if self.lasti() <= jump_to {
            6
        } else {
            0
        }
    }

    /// returns: shift bytes
    #[inline]
    fn edit_code(&mut self, idx: usize, arg: usize) -> usize {
        log!(err "editing: {idx} {arg}");
        match u8::try_from(arg) {
            Ok(u8code) => {
                self.mut_code()[idx] = u8code;
                0
            }
            Err(_e) => {
                // TODO: use u16 as long as possible
                // see write_arg's comment
                let delta = self.jump_delta(arg);
                let bytes = u32::try_from(arg + delta).unwrap().to_be_bytes();
                let before_instr = idx.saturating_sub(1);
                self.mut_code()[idx] = bytes[3];
                self.extend_arg(before_instr, &bytes)
            }
        }
    }

    // e.g. JUMP_ABSOLUTE 264, lasti: 100
    // 6 more instructions will be added after this, so 264 + 6 => 270
    // this is greater than u8::MAX, so we need to extend the arg
    // first, split `code + delta` into 4 u8s (as __Big__ endian)
    // 270.to_be_bytes() == [0, 0, 1, 14]
    // then, write the bytes in reverse order
    // [..., EXTENDED_ARG 0, EXTENDED_ARG 0, EXTENDED_ARG 1, JUMP_ABSOLUTE 14]
    /// returns: shift bytes
    #[inline]
    fn extend_arg(&mut self, before_instr: usize, bytes: &[u8]) -> usize {
        let mut shift_bytes = 0;
        for byte in bytes.iter().rev().skip(1) {
            self.mut_code().insert(before_instr, *byte);
            self.mut_code()
                .insert(before_instr, CommonOpcode::EXTENDED_ARG as u8);
            self.mut_cur_block().lasti += 2;
            shift_bytes += 2;
        }
        shift_bytes
    }

    fn stash_code(&mut self) -> usize {
        self.mut_cur_block().tmp_codes.push(vec![]);
        self.cur_block().tmp_codes.len() - 1
    }

    fn pop_code(&mut self) -> Vec<u8> {
        self.mut_cur_block().tmp_codes.pop().unwrap()
    }

    fn mut_code(&mut self) -> &mut Vec<u8> {
        let cur_block = self.mut_cur_block();
        if let Some(tmp_code) = cur_block.tmp_codes.last_mut() {
            tmp_code
        } else {
            &mut cur_block.codeobj.code
        }
    }

    fn push_code<C: Into<u8>>(&mut self, code: C) {
        self.mut_code().push(code.into());
    }

    fn write<C: Into<u8>>(&mut self, op: C, mut code: usize) {
        let op = op.into();
        if CommonOpcode::is_relative_jump_op(self.py_version.minor.unwrap_or(11), op) {
            code = self.lasti() - code;
        }
        if CommonOpcode::is_jump_op(op) {
            code /= 2;
        }
        match u8::try_from(code) {
            Ok(u8code) => {
                self.push_code(op);
                self.push_code(u8code);
                self.mut_cur_block().lasti += 2;
            }
            Err(_) => match u16::try_from(code) {
                Ok(_) => {
                    let delta =
                        if CommonOpcode::is_jump_op(*self.cur_block_codeobj().code.last().unwrap())
                        {
                            let shift_bytes = 2;
                            self.jump_delta(code) + shift_bytes
                        } else {
                            0
                        };
                    let arg = code + delta;
                    let bytes = u16::try_from(arg).unwrap().to_be_bytes(); // [u8; 2]
                    self.push_code(EXTENDED_ARG);
                    self.push_code(bytes[0]);
                    self.push_code(op);
                    self.push_code(bytes[1]);
                    self.mut_cur_block().lasti += 4;
                }
                Err(_) => {
                    let delta =
                        if CommonOpcode::is_jump_op(*self.cur_block_codeobj().code.last().unwrap())
                        {
                            let shift_bytes = 6;
                            self.jump_delta(code) + shift_bytes
                        } else {
                            0
                        };
                    let arg = code + delta;
                    let bytes = u32::try_from(arg).unwrap().to_be_bytes(); // [u8; 4]
                    self.push_code(EXTENDED_ARG);
                    self.push_code(bytes[0]);
                    self.push_code(EXTENDED_ARG);
                    self.push_code(bytes[1]);
                    self.push_code(EXTENDED_ARG);
                    self.push_code(bytes[2]);
                    self.push_code(op);
                    self.push_code(bytes[3]);
                    self.mut_cur_block().lasti += 8;
                }
            },
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        self.mut_code().extend_from_slice(bytes);
        self.mut_cur_block().lasti += bytes.len();
    }

    fn stack_inc(&mut self) {
        self.mut_cur_block().stack_len += 1;
        if self.stack_len() > self.cur_block_codeobj().stacksize {
            self.mut_cur_block_codeobj().stacksize = self.stack_len();
        }
    }

    #[track_caller]
    fn stack_dec(&mut self) {
        if self.stack_len() == 0 {
            let lasti = self.lasti();
            let last = self.cur_block_codeobj().code.last().unwrap();
            self.crash(&format!(
                "the stack size becomes -1\nlasti: {lasti}\nlast code: {last}"
            ));
        } else {
            self.mut_cur_block().stack_len -= 1;
        }
    }

    /// NOTE: For example, an operation that increases the stack by 2 and decreases it by 1 should be `stack_inc_n(2); stack_dec();` not `stack_inc(1);`.
    /// This is because the stack size will not increase correctly.
    fn stack_inc_n(&mut self, n: usize) {
        self.mut_cur_block().stack_len += n as u32;
        if self.stack_len() > self.cur_block_codeobj().stacksize {
            self.mut_cur_block_codeobj().stacksize = self.stack_len();
        }
    }

    #[track_caller]
    fn stack_dec_n(&mut self, n: usize) {
        if n > 0 && self.stack_len() == 0 {
            let lasti = self.lasti();
            let last = self.cur_block_codeobj().code.last().unwrap();
            self.crash(&format!(
                "the stack size is less than -1\nlasti: {lasti}\nlast code: {last}"
            ));
        } else {
            self.mut_cur_block().stack_len -= n as u32;
        }
    }

    fn emit_load_const<C: Into<ValueObj>>(&mut self, cons: C) {
        let value: ValueObj = cons.into();
        let idx = self
            .mut_cur_block_codeobj()
            .consts
            .iter()
            .position(|c| c == &value)
            .unwrap_or_else(|| {
                self.mut_cur_block_codeobj().consts.push(value);
                self.mut_cur_block_codeobj().consts.len() - 1
            });
        self.write(LOAD_CONST, idx);
        self.stack_inc();
    }

    fn register_const<C: Into<ValueObj>>(&mut self, cons: C) -> usize {
        let value = cons.into();
        self.mut_cur_block_codeobj()
            .consts
            .iter()
            .position(|c| c == &value)
            .unwrap_or_else(|| {
                self.mut_cur_block_codeobj().consts.push(value);
                self.mut_cur_block_codeobj().consts.len() - 1
            })
    }

    fn local_search(&self, name: &str, _acc_kind: AccessKind) -> Option<Name> {
        let current_is_toplevel = self.cur_block() == self.toplevel_block();
        if let Some(idx) = self
            .cur_block_codeobj()
            .names
            .iter()
            .position(|n| &**n == name)
        {
            Some(Name::local(idx))
        } else if let Some(idx) = self
            .cur_block_codeobj()
            .varnames
            .iter()
            .position(|v| &**v == name)
        {
            if current_is_toplevel {
                Some(Name::local(idx))
            } else {
                Some(Name::fast(idx))
            }
        } else {
            self.cur_block_codeobj()
                .freevars
                .iter()
                .position(|f| &**f == name)
                .map(Name::deref)
        }
    }

    // local_searchで見つからなかった変数を探索する
    fn rec_search(&mut self, name: &str) -> Option<StoreLoadKind> {
        // search_name()を実行した後なのでcur_blockはskipする
        for (nth_from_toplevel, block) in self.units.iter_mut().enumerate().rev().skip(1) {
            let block_is_toplevel = nth_from_toplevel == 0;
            if block.codeobj.cellvars.iter().any(|c| &**c == name) {
                return Some(StoreLoadKind::Deref);
            } else if let Some(idx) = block.codeobj.varnames.iter().position(|v| &**v == name) {
                if block_is_toplevel {
                    return Some(StoreLoadKind::Global);
                } else {
                    // the outer scope variable
                    let cellvar_name = block.codeobj.varnames.get(idx).unwrap().clone();
                    block.codeobj.cellvars.push(cellvar_name);
                    return Some(StoreLoadKind::Deref);
                }
            }
            if block_is_toplevel && block.codeobj.names.iter().any(|n| &**n == name) {
                return Some(StoreLoadKind::Global);
            }
        }
        // 見つからなかった変数(前方参照変数など)はグローバル
        Some(StoreLoadKind::Global)
    }

    fn register_name(&mut self, name: Str) -> Name {
        let current_is_toplevel = self.cur_block() == self.toplevel_block();
        match self.rec_search(&name) {
            Some(st @ (StoreLoadKind::Local | StoreLoadKind::Global)) => {
                let st = if current_is_toplevel {
                    StoreLoadKind::Local
                } else {
                    st
                };
                self.mut_cur_block_codeobj().names.push(name);
                Name::new(st, self.cur_block_codeobj().names.len() - 1)
            }
            Some(StoreLoadKind::Deref) => {
                self.mut_cur_block_codeobj().freevars.push(name.clone());
                if self.py_version.minor >= Some(11) {
                    // in 3.11 freevars are unified with varnames
                    self.mut_cur_block_codeobj().varnames.push(name);
                    Name::deref(self.cur_block_codeobj().varnames.len() - 1)
                } else {
                    // cellvarsのpushはrec_search()で行われる
                    Name::deref(self.cur_block_codeobj().freevars.len() - 1)
                }
            }
            None => {
                // new variable
                if current_is_toplevel {
                    self.mut_cur_block_codeobj().names.push(name);
                    Name::local(self.cur_block_codeobj().names.len() - 1)
                } else {
                    self.mut_cur_block_codeobj().varnames.push(name);
                    Name::fast(self.cur_block_codeobj().varnames.len() - 1)
                }
            }
            Some(_) => {
                switch_unreachable!()
            }
        }
    }

    fn register_attr(&mut self, name: Str) -> Name {
        self.mut_cur_block_codeobj().names.push(name);
        Name::local(self.cur_block_codeobj().names.len() - 1)
    }

    fn register_method(&mut self, name: Str) -> Name {
        self.mut_cur_block_codeobj().names.push(name);
        Name::local(self.cur_block_codeobj().names.len() - 1)
    }

    fn select_load_instr(&self, kind: StoreLoadKind, acc_kind: AccessKind) -> u8 {
        match kind {
            StoreLoadKind::Fast | StoreLoadKind::FastConst => LOAD_FAST as u8,
            StoreLoadKind::Global | StoreLoadKind::GlobalConst => LOAD_NAME as u8, //LOAD_GLOBAL as u8,
            StoreLoadKind::Deref | StoreLoadKind::DerefConst => {
                if self.py_version.minor >= Some(11) {
                    Opcode311::LOAD_DEREF as u8
                } else {
                    Opcode310::LOAD_DEREF as u8
                }
            }
            StoreLoadKind::Local | StoreLoadKind::LocalConst => match acc_kind {
                Name => LOAD_NAME as u8,
                UnboundAttr => LOAD_ATTR as u8,
                BoundAttr => LOAD_METHOD as u8,
            },
        }
    }

    fn select_store_instr(&self, kind: StoreLoadKind, acc_kind: AccessKind) -> u8 {
        match kind {
            StoreLoadKind::Fast => STORE_FAST as u8,
            StoreLoadKind::FastConst => STORE_FAST as u8, // ERG_STORE_FAST_IMMUT,
            // NOTE: First-time variables are treated as GLOBAL, but they are always first-time variables when assigned, so they are just NAME
            // NOTE: 初見の変数はGLOBAL扱いになるが、代入時は必ず初見であるので単なるNAME
            StoreLoadKind::Global | StoreLoadKind::GlobalConst => STORE_NAME as u8,
            StoreLoadKind::Deref | StoreLoadKind::DerefConst => {
                if self.py_version.minor >= Some(11) {
                    Opcode311::STORE_DEREF as u8
                } else {
                    Opcode310::STORE_DEREF as u8
                }
            }
            StoreLoadKind::Local | StoreLoadKind::LocalConst => {
                match acc_kind {
                    Name => STORE_NAME as u8,
                    UnboundAttr => STORE_ATTR as u8,
                    // cannot overwrite methods directly
                    BoundAttr => STORE_ATTR as u8,
                }
            }
        }
    }

    fn emit_load_name_instr(&mut self, ident: Identifier) {
        log!(info "entered {}({ident})", fn_name!());
        if &ident.inspect()[..] == "#ModuleType" && !self.module_type_loaded {
            self.load_module_type();
            self.module_type_loaded = true;
        }
        let escaped = escape_ident(ident);
        match &escaped[..] {
            "if__" | "for__" | "while__" | "with__" | "discard__" => {
                self.load_control();
            }
            "int__" | "nat__" | "str__" | "float__" => {
                self.load_convertors();
            }
            // NoneType is not defined in the global scope, use `type(None)` instead
            "NoneType" => {
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::public("type"));
                self.emit_load_const(ValueObj::None);
                self.emit_precall_and_call(1);
                self.stack_dec();
                return;
            }
            _ => {}
        }
        let name = self
            .local_search(&escaped, Name)
            .unwrap_or_else(|| self.register_name(escaped));
        let instr = self.select_load_instr(name.kind, Name);
        self.write(instr, name.idx);
        self.stack_inc();
        if instr == LOAD_GLOBAL as u8 && self.py_version.minor >= Some(11) {
            self.write_bytes(&[0; 2]);
            self.write_bytes(&[0; 8]);
        }
    }

    fn emit_load_global_instr(&mut self, ident: Identifier) {
        log!(info "entered {} ({ident})", fn_name!());
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, Name)
            .unwrap_or_else(|| self.register_name(escaped));
        let instr = LOAD_GLOBAL;
        self.write(instr, name.idx);
        self.stack_inc();
    }

    fn emit_import_name_instr(&mut self, ident: Identifier, items_len: usize) {
        log!(info "entered {}({ident})", fn_name!());
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, Name)
            .unwrap_or_else(|| self.register_name(escaped));
        self.write(IMPORT_NAME, name.idx);
        self.stack_inc_n(items_len);
        self.stack_dec(); // (level + from_list) -> module object
    }

    fn emit_import_from_instr(&mut self, ident: Identifier) {
        log!(info "entered {}", fn_name!());
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, Name)
            .unwrap_or_else(|| self.register_name(escaped));
        self.write(IMPORT_FROM, name.idx);
        // self.stack_inc(); (module object) -> attribute
    }

    fn emit_import_all_instr(&mut self, ident: Identifier) {
        log!(info "entered {}", fn_name!());
        self.emit_load_const(0i32); // escaping to call access `Nat` before importing `Nat`
        self.emit_load_const([Str::ever("*")]);
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, Name)
            .unwrap_or_else(|| self.register_name(escaped));
        self.write(IMPORT_NAME, name.idx);
        self.stack_inc();
        self.write(IMPORT_STAR, 0);
        self.stack_dec_n(3);
    }

    /// item: (name, renamed)
    fn emit_global_import_items(
        &mut self,
        module: Identifier,
        items: Vec<(Identifier, Option<Identifier>)>,
    ) {
        self.emit_load_const(0);
        let item_name_tuple = items
            .iter()
            .map(|ident| ValueObj::Str(ident.0.inspect().clone()))
            .collect::<Vec<_>>();
        let items_len = item_name_tuple.len();
        self.emit_load_const(item_name_tuple);
        self.emit_import_name_instr(module, items_len);
        for (item, renamed) in items.into_iter() {
            if let Some(renamed) = renamed {
                self.emit_import_from_instr(item);
                self.emit_store_global_instr(renamed);
            } else {
                self.emit_import_from_instr(item.clone());
                self.emit_store_global_instr(item);
            }
        }
        self.emit_pop_top(); // discard IMPORT_FROM object
    }

    fn emit_load_attr_instr(&mut self, ident: Identifier) {
        log!(info "entered {} ({ident})", fn_name!());
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, UnboundAttr)
            .unwrap_or_else(|| self.register_attr(escaped));
        let instr = self.select_load_instr(name.kind, UnboundAttr);
        self.write(instr, name.idx);
        if self.py_version.minor >= Some(11) {
            self.write_bytes(&[0; 8]);
        }
    }

    fn emit_load_method_instr(&mut self, ident: Identifier) {
        log!(info "entered {} ({ident})", fn_name!());
        if &ident.inspect()[..] == "__new__" {
            log!("{:?}", ident.vi);
        }
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, BoundAttr)
            .unwrap_or_else(|| self.register_method(escaped));
        let instr = self.select_load_instr(name.kind, BoundAttr);
        self.write(instr, name.idx);
        if self.py_version.minor >= Some(11) {
            self.stack_inc(); // instead of PUSH_NULL
            self.write_bytes(&[0; 20]);
        }
    }

    fn emit_store_instr(&mut self, ident: Identifier, acc_kind: AccessKind) {
        log!(info "entered {} ({ident})", fn_name!());
        let escaped = escape_ident(ident);
        let name = self.local_search(&escaped, acc_kind).unwrap_or_else(|| {
            if acc_kind.is_local() {
                self.register_name(escaped)
            } else {
                self.register_attr(escaped)
            }
        });
        let instr = self.select_store_instr(name.kind, acc_kind);
        self.write(instr, name.idx);
        self.stack_dec();
        if instr == STORE_ATTR as u8 {
            if self.py_version.minor >= Some(11) {
                self.write_bytes(&[0; 8]);
            }
            self.stack_dec();
        }
    }

    /// used for importing Erg builtin objects, etc. normally, this is not used
    // Ergの組み込みオブジェクトをimportするときなどに使う、通常は使わない
    fn emit_store_global_instr(&mut self, ident: Identifier) {
        log!(info "entered {} ({ident})", fn_name!());
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, Name)
            .unwrap_or_else(|| self.register_name(escaped));
        let instr = STORE_GLOBAL;
        self.write(instr, name.idx);
        self.stack_dec();
    }

    /// Ergの文法として、属性への代入は存在しない(必ずオブジェクトはすべての属性を初期化しなくてはならないため)
    /// この関数はPythonへ落とし込むときに使う
    fn store_acc(&mut self, acc: Accessor) {
        log!(info "entered {} ({acc})", fn_name!());
        match acc {
            Accessor::Ident(ident) => {
                self.emit_store_instr(ident, Name);
            }
            Accessor::Attr(attr) => {
                self.emit_expr(*attr.obj);
                self.emit_store_instr(attr.ident, UnboundAttr);
            }
        }
    }

    fn emit_pop_top(&mut self) {
        self.write(POP_TOP, 0);
        self.stack_dec();
    }

    fn cancel_if_pop_top(&mut self) {
        if self.mut_code().len() < 2 {
            return;
        }
        let lasop_t_idx = self.mut_code().len() - 2;
        if self.mut_code().get(lasop_t_idx) == Some(&(POP_TOP as u8)) {
            self.mut_code().pop();
            self.mut_code().pop();
            self.mut_cur_block().lasti -= 2;
            self.stack_inc();
        }
    }

    /// Compileが継続不能になった際呼び出す
    /// 極力使わないこと
    #[track_caller]
    fn crash(&mut self, description: &str) -> ! {
        if cfg!(feature = "debug") {
            println!("current block: {}", self.cur_block());
            panic!("internal error: {description}");
        } else {
            let err = CompileError::compiler_bug(
                0,
                self.input().clone(),
                Location::Unknown,
                std::panic::Location::caller().file(),
                std::panic::Location::caller().line(),
                Some(description.to_string()),
            );
            err.write_to_stderr();
            process::exit(1);
        }
    }

    fn gen_param_names(&self, params: &Params) -> Vec<Str> {
        params
            .non_defaults
            .iter()
            .map(|p| (p.inspect().map(|s| &s[..]).unwrap_or("_"), &p.vi))
            .chain(if let Some(var_args) = &params.var_params {
                vec![(
                    var_args.inspect().map(|s| &s[..]).unwrap_or("_"),
                    &var_args.vi,
                )]
            } else {
                vec![]
            })
            .chain(
                params
                    .defaults
                    .iter()
                    .map(|p| (p.inspect().map(|s| &s[..]).unwrap_or("_"), &p.sig.vi)),
            )
            .enumerate()
            .map(|(i, (s, vi))| {
                if s == "_" {
                    format!("_{i}")
                } else {
                    escape_name(
                        s,
                        &VisibilityModifier::Private,
                        vi.def_loc.loc.ln_begin().unwrap_or(0),
                        vi.def_loc.loc.col_begin().unwrap_or(0),
                    )
                    .to_string()
                }
            })
            .map(|s| self.get_cached(&s))
            .collect()
    }

    fn emit_acc(&mut self, acc: Accessor) {
        log!(info "entered {} ({acc})", fn_name!());
        match acc {
            Accessor::Ident(ident) => {
                self.emit_load_name_instr(ident);
            }
            Accessor::Attr(mut a) => {
                // Python's namedtuple, a representation of Record, does not allow attribute names such as `::x`.
                // Since Erg does not allow the coexistence of private and public variables with the same name, there is no problem in this trick.
                let is_record = a.obj.ref_t().is_record();
                if is_record {
                    a.ident.raw.vis = VisModifierSpec::Public(DOT);
                }
                if let Some(varname) = debind(&a.ident) {
                    a.ident.raw.vis = VisModifierSpec::Private;
                    a.ident.raw.name = VarName::from_str(varname);
                    self.emit_load_name_instr(a.ident);
                } else {
                    self.emit_expr(*a.obj);
                    self.emit_load_attr_instr(a.ident);
                }
            }
        }
    }

    fn emit_def(&mut self, def: Def) {
        log!(info "entered {} ({})", fn_name!(), def.sig);
        if def.def_kind().is_trait() {
            return self.emit_trait_def(def);
        }
        match def.sig {
            Signature::Subr(sig) => self.emit_subr_def(None, sig, def.body),
            Signature::Var(sig) => self.emit_var_def(sig, def.body),
        }
    }

    fn emit_push_null(&mut self) {
        if self.py_version.minor >= Some(11) {
            self.write(Opcode311::PUSH_NULL, 0);
            self.stack_inc();
        }
    }

    fn emit_precall_and_call(&mut self, argc: usize) {
        self.write(Opcode311::PRECALL, argc);
        self.write(0, 0);
        self.write(Opcode311::CALL, argc);
        self.write_bytes(&[0; 8]);
        self.stack_dec();
    }

    fn emit_call_instr(&mut self, argc: usize, kind: AccessKind) {
        if self.py_version.minor >= Some(11) {
            self.emit_precall_and_call(argc);
        } else {
            match kind {
                AccessKind::BoundAttr => self.write(Opcode310::CALL_METHOD, argc),
                _ => self.write(Opcode310::CALL_FUNCTION, argc),
            }
        }
    }

    fn emit_call_kw_instr(&mut self, argc: usize, kws: Vec<ValueObj>) {
        if self.py_version.minor >= Some(11) {
            let idx = self.register_const(kws);
            self.write(Opcode311::KW_NAMES, idx);
            self.emit_precall_and_call(argc);
        } else {
            self.emit_load_const(kws);
            self.write(Opcode310::CALL_FUNCTION_KW, argc);
        }
    }

    fn emit_trait_def(&mut self, def: Def) {
        if !self.abc_loaded {
            self.load_abc();
            self.abc_loaded = true;
        }
        self.emit_push_null();
        self.write(LOAD_BUILD_CLASS, 0);
        self.stack_inc();
        let code = self.emit_trait_block(def.def_kind(), &def.sig, def.body.block);
        self.emit_load_const(code);
        if self.py_version.minor < Some(11) {
            self.emit_load_const(def.sig.ident().inspect().clone());
        } else {
            self.stack_inc();
        }
        self.write(MAKE_FUNCTION, 0);
        self.emit_load_const(def.sig.ident().inspect().clone());
        self.emit_load_name_instr(Identifier::private("#ABCMeta"));
        let subclasses_len = 1;
        self.emit_call_kw_instr(2 + subclasses_len, vec![ValueObj::from("metaclass")]);
        let sum = if self.py_version.minor >= Some(11) {
            1 + 2 + subclasses_len
        } else {
            1 + 2 + 1 + subclasses_len
        };
        self.stack_dec_n(sum - 1);
        self.emit_store_instr(def.sig.into_ident(), Name);
        self.stack_dec();
    }

    // trait variables will be removed
    // T = Trait {
    //     x = Int
    //     f = (self: Self) -> Int
    // }
    // ↓
    // class T(metaclass=ABCMeta):
    //    def f(): pass
    fn emit_trait_block(&mut self, kind: DefKind, sig: &Signature, mut block: Block) -> CodeObj {
        debug_assert_eq!(kind, DefKind::Trait);
        let name = sig.ident().inspect().clone();
        let Expr::Call(mut trait_call) = block.remove(0) else { unreachable!() };
        let req = if let Some(Expr::Record(req)) = trait_call.args.remove_left_or_key("Requirement")
        {
            req.attrs.into_iter()
        } else {
            vec![].into_iter()
        };
        self.unit_size += 1;
        let firstlineno = block
            .get(0)
            .and_then(|def| def.ln_begin())
            .unwrap_or_else(|| sig.ln_begin().unwrap());
        self.units.push(PyCodeGenUnit::new(
            self.unit_size,
            self.py_version,
            vec![],
            Str::rc(self.cfg.input.enclosed_name()),
            &name,
            firstlineno,
            0,
        ));
        let mod_name = self.toplevel_block_codeobj().name.clone();
        self.emit_load_const(mod_name);
        self.emit_store_instr(Identifier::public("__module__"), Name);
        self.emit_load_const(name);
        self.emit_store_instr(Identifier::public("__qualname__"), Name);
        for def in req {
            self.emit_empty_func(
                Some(sig.ident().inspect()),
                def.sig.into_ident(),
                Some(Identifier::private("#abstractmethod")),
            );
        }
        self.emit_load_const(ValueObj::None);
        self.write(RETURN_VALUE, 0);
        if self.stack_len() > 1 {
            let block_id = self.cur_block().id;
            let stack_len = self.stack_len();
            CompileError::stack_bug(
                self.input().clone(),
                Location::Unknown,
                stack_len,
                block_id,
                fn_name_full!(),
            )
            .write_to_stderr();
            self.crash("error in emit_trait_block: invalid stack size");
        }
        // flagging
        if !self.cur_block_codeobj().varnames.is_empty() {
            self.mut_cur_block_codeobj().flags += CodeObjFlags::NewLocals as u32;
        }
        // end of flagging
        let unit = self.units.pop().unwrap();
        if !self.units.is_empty() {
            let ld = unit.prev_lineno - self.cur_block().prev_lineno;
            if ld != 0 {
                if let Some(l) = self.mut_cur_block_codeobj().lnotab.last_mut() {
                    *l += ld as u8;
                }
                self.mut_cur_block().prev_lineno += ld;
            }
        }
        unit.codeobj
    }

    fn emit_empty_func(
        &mut self,
        class_name: Option<&str>,
        ident: Identifier,
        deco: Option<Identifier>,
    ) {
        log!(info "entered {} ({ident})", fn_name!());
        self.emit_push_null();
        let deco_is_some = deco.is_some();
        if let Some(deco) = deco {
            self.emit_load_name_instr(deco);
        }
        let code = {
            self.unit_size += 1;
            self.units.push(PyCodeGenUnit::new(
                self.unit_size,
                self.py_version,
                vec![],
                Str::rc(self.cfg.input.enclosed_name()),
                ident.inspect(),
                ident.ln_begin().unwrap_or(0),
                0,
            ));
            self.emit_load_const(ValueObj::None);
            self.write(RETURN_VALUE, 0);
            let unit = self.units.pop().unwrap();
            if !self.units.is_empty() {
                let ld = unit
                    .prev_lineno
                    .saturating_sub(self.cur_block().prev_lineno);
                if ld != 0 {
                    if let Some(l) = self.mut_cur_block_codeobj().lnotab.last_mut() {
                        *l += ld as u8;
                    }
                    self.mut_cur_block().prev_lineno += ld;
                }
            }
            unit.codeobj
        };
        self.emit_load_const(code);
        if self.py_version.minor < Some(11) {
            if let Some(class) = class_name {
                self.emit_load_const(Str::from(format!("{class}.{}", ident.inspect())));
            } else {
                self.emit_load_const(ident.inspect().clone());
            }
        } else {
            self.stack_inc();
        }
        self.write(MAKE_FUNCTION, 0);
        if deco_is_some {
            self.emit_call_instr(1, Name);
            self.stack_dec();
        }
        // stack_dec: (<abstractmethod>) + <code obj> + <name> -> <function>
        self.stack_dec();
        self.emit_store_instr(ident, Name);
    }

    fn emit_class_def(&mut self, class_def: ClassDef) {
        log!(info "entered {} ({})", fn_name!(), class_def.sig);
        self.emit_push_null();
        let ident = class_def.sig.ident().clone();
        let require_or_sup = class_def.require_or_sup.clone().map(|x| *x);
        let obj = class_def.obj.clone();
        self.write(LOAD_BUILD_CLASS, 0);
        self.stack_inc();
        let code = self.emit_class_block(class_def);
        self.emit_load_const(code);
        if self.py_version.minor < Some(11) {
            self.emit_load_const(ident.inspect().clone());
        } else {
            self.stack_inc();
        }
        self.write(MAKE_FUNCTION, 0);
        self.emit_load_const(ident.inspect().clone());
        // LOAD subclasses
        let subclasses_len = self.emit_require_type(obj, require_or_sup);
        self.emit_call_instr(2 + subclasses_len, Name);
        self.stack_dec_n((1 + 2 + subclasses_len) - 1);
        self.emit_store_instr(ident, Name);
        self.stack_dec();
    }

    fn emit_patch_def(&mut self, patch_def: PatchDef) {
        log!(info "entered {} ({})", fn_name!(), patch_def.sig);
        for def in patch_def.methods {
            // Invert.
            //     invert self = ...
            // ↓
            // def Invert::invert(self): ...
            let Expr::Def(mut def) = def else { todo!() };
            let namespace = self.cur_block_codeobj().name.trim_start_matches("::");
            let name = format!(
                "{}{}{}",
                namespace,
                patch_def.sig.ident().to_string_notype(),
                def.sig.ident().to_string_notype()
            );
            def.sig.ident_mut().raw.name = VarName::from_str(Str::from(name));
            def.sig.ident_mut().raw.vis = VisModifierSpec::Private;
            self.emit_def(def);
        }
    }

    // NOTE: use `TypeVar`, `Generic` in `typing` module
    // fn emit_poly_type_def(&mut self, sig: SubrSignature, body: DefBody) {}
    /// Y = Inherit X => class Y(X): ...
    fn emit_require_type(&mut self, obj: GenTypeObj, require_or_sup: Option<Expr>) -> usize {
        log!(info "entered {} ({obj}, {require_or_sup:?})", fn_name!());
        match obj {
            GenTypeObj::Class(_) => 0,
            GenTypeObj::Subclass(_) => {
                self.emit_expr(require_or_sup.unwrap());
                1 // TODO: not always 1
            }
            _ => todo!(),
        }
    }

    fn emit_redef(&mut self, redef: ReDef) {
        log!(info "entered {} ({redef})", fn_name!());
        self.emit_frameless_block(redef.block, vec![]);
        self.store_acc(redef.attr);
    }

    fn emit_var_def(&mut self, sig: VarSignature, mut body: DefBody) {
        log!(info "entered {} ({sig} = {})", fn_name!(), body.block);
        if body.block.len() == 1 {
            self.emit_expr(body.block.remove(0));
        } else {
            self.emit_frameless_block(body.block, vec![]);
        }
        if sig.global {
            self.emit_store_global_instr(sig.ident);
        } else {
            self.emit_store_instr(sig.ident, Name);
        }
    }

    fn emit_subr_def(&mut self, class_name: Option<&str>, sig: SubrSignature, body: DefBody) {
        log!(info "entered {} ({sig} = {})", fn_name!(), body.block);
        let name = sig.ident.inspect().clone();
        let mut make_function_flag = 0;
        let params = self.gen_param_names(&sig.params);
        if !sig.params.defaults.is_empty() {
            let defaults_len = sig.params.defaults.len();
            sig.params
                .defaults
                .into_iter()
                .for_each(|default| self.emit_expr(default.default_val));
            self.write(BUILD_TUPLE, defaults_len);
            self.stack_dec_n(defaults_len - 1);
            make_function_flag += MakeFunctionFlags::Defaults as usize;
        }
        let flags = if sig.params.var_params.is_some() {
            CodeObjFlags::VarArgs as u32
        } else {
            0
        };
        let code = self.emit_block(body.block, Some(name.clone()), params, flags);
        // code.flags += CodeObjFlags::Optimized as u32;
        self.register_cellvars(&mut make_function_flag);
        self.emit_load_const(code);
        if self.py_version.minor < Some(11) {
            if let Some(class) = class_name {
                self.emit_load_const(Str::from(format!("{class}.{name}")));
            } else {
                self.emit_load_const(name);
            }
        } else {
            self.stack_inc();
        }
        self.write(MAKE_FUNCTION, make_function_flag);
        // stack_dec: <code obj> + <name> -> <function>
        self.stack_dec();
        if make_function_flag & MakeFunctionFlags::Defaults as usize != 0 {
            self.stack_dec();
        }
        self.emit_store_instr(sig.ident, Name);
    }

    fn emit_lambda(&mut self, lambda: Lambda) {
        log!(info "entered {} ({lambda})", fn_name!());
        let mut make_function_flag = 0;
        let params = self.gen_param_names(&lambda.params);
        if !lambda.params.defaults.is_empty() {
            let defaults_len = lambda.params.defaults.len();
            lambda
                .params
                .defaults
                .into_iter()
                .for_each(|default| self.emit_expr(default.default_val));
            self.write(BUILD_TUPLE, defaults_len);
            self.stack_dec_n(defaults_len - 1);
            make_function_flag += MakeFunctionFlags::Defaults as usize;
        }
        let flags = if lambda.params.var_params.is_some() {
            CodeObjFlags::VarArgs as u32
        } else {
            0
        };
        let code = self.emit_block(lambda.body, Some("<lambda>".into()), params, flags);
        self.register_cellvars(&mut make_function_flag);
        self.emit_load_const(code);
        if self.py_version.minor < Some(11) {
            self.emit_load_const("<lambda>");
        } else {
            self.stack_inc();
        }
        self.write(MAKE_FUNCTION, make_function_flag);
        // stack_dec: <lambda code obj> + <name "<lambda>"> -> <function>
        self.stack_dec();
        if make_function_flag & MakeFunctionFlags::Defaults as usize != 0 {
            self.stack_dec();
        }
    }

    fn register_cellvars(&mut self, flag: &mut usize) {
        if !self.cur_block_codeobj().cellvars.is_empty() {
            let cellvars_len = self.cur_block_codeobj().cellvars.len();
            for i in 0..cellvars_len {
                if self.py_version.minor >= Some(11) {
                    self.write(Opcode311::MAKE_CELL, i);
                    self.write(Opcode311::LOAD_CLOSURE, i);
                } else {
                    self.write(Opcode310::LOAD_CLOSURE, i);
                }
            }
            self.write(BUILD_TUPLE, cellvars_len);
            *flag += MakeFunctionFlags::Closure as usize;
        }
    }

    fn emit_unaryop(&mut self, unary: UnaryOp) {
        log!(info "entered {} ({unary})", fn_name!());
        let val_t = unary
            .info
            .t
            .non_default_params()
            .and_then(|tys| tys.get(0).map(|pt| pt.typ()))
            .unwrap_or(Type::FAILURE);
        let tycode = TypeCode::from(val_t);
        let instr = match &unary.op.kind {
            // TODO:
            TokenKind::PrePlus => UNARY_POSITIVE,
            TokenKind::PreMinus => UNARY_NEGATIVE,
            TokenKind::PreBitNot => UNARY_INVERT,
            TokenKind::Mutate => {
                if !self.mutate_op_loaded {
                    self.load_mutate_op();
                }
                if self.py_version.minor >= Some(11) {
                    self.emit_push_null();
                }
                self.emit_load_name_instr(Identifier::private("#mutate_operator"));
                NOP // ERG_MUTATE,
            }
            _ => {
                CompileError::feature_error(
                    self.cfg.input.clone(),
                    unary.op.loc(),
                    &unary.op.inspect().clone(),
                    String::from(unary.op.content),
                )
                .write_to_stderr();
                NOT_IMPLEMENTED
            }
        };
        self.emit_expr(*unary.expr);
        if instr != NOP {
            self.write(instr, tycode as usize);
        } else {
            if self.py_version.minor >= Some(11) {
                self.emit_precall_and_call(1);
            } else {
                self.write(Opcode310::CALL_FUNCTION, 1);
            }
            self.stack_dec();
        }
    }

    fn emit_binop(&mut self, bin: BinOp) {
        log!(info "entered {} ({bin})", fn_name!());
        // TODO: and/orのプリミティブ命令の実装
        // Range operators are not operators in Python
        match &bin.op.kind {
            // l..<r == range(l, r)
            TokenKind::RightOpen => {
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::public("RightOpenRange"));
            }
            TokenKind::LeftOpen => {
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::public("LeftOpenRange"));
            }
            TokenKind::Closed => {
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::public("ClosedRange"));
            }
            TokenKind::Open => {
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::public("OpenRange"));
            }
            TokenKind::InOp => {
                // if no-std, always `x in y == True`
                if self.cfg.no_std {
                    self.emit_load_const(true);
                    return;
                }
                if !self.in_op_loaded {
                    self.load_in_op();
                }
                self.emit_push_null();
                self.emit_load_name_instr(Identifier::private("#in_operator"));
            }
            _ => {}
        }
        let lhs_t = bin
            .info
            .t
            .non_default_params()
            .and_then(|tys| tys.get(0).map(|pt| pt.typ()))
            .unwrap_or(Type::FAILURE);
        let rhs_t = bin
            .info
            .t
            .non_default_params()
            .and_then(|tys| tys.get(1).map(|pt| pt.typ()))
            .unwrap_or(Type::FAILURE);
        let type_pair = TypePair::new(lhs_t, rhs_t);
        self.emit_expr(*bin.lhs);
        self.emit_expr(*bin.rhs);
        self.emit_binop_instr(bin.op, type_pair);
    }

    fn emit_binop_instr(&mut self, binop: Token, type_pair: TypePair) {
        if self.py_version.minor >= Some(11) {
            self.emit_binop_instr_311(binop, type_pair);
        } else {
            self.emit_binop_instr_310(binop, type_pair);
        }
    }

    fn emit_binop_instr_310(&mut self, binop: Token, type_pair: TypePair) {
        let instr = match &binop.kind {
            TokenKind::Plus => Opcode310::BINARY_ADD,
            TokenKind::Minus => Opcode310::BINARY_SUBTRACT,
            TokenKind::Star => Opcode310::BINARY_MULTIPLY,
            TokenKind::Slash => Opcode310::BINARY_TRUE_DIVIDE,
            TokenKind::FloorDiv => Opcode310::BINARY_FLOOR_DIVIDE,
            TokenKind::Pow => Opcode310::BINARY_POWER,
            TokenKind::Mod => Opcode310::BINARY_MODULO,
            TokenKind::AndOp | TokenKind::BitAnd => Opcode310::BINARY_AND,
            TokenKind::OrOp | TokenKind::BitOr => Opcode310::BINARY_OR,
            TokenKind::BitXor => Opcode310::BINARY_XOR,
            TokenKind::IsOp | TokenKind::IsNotOp => Opcode310::IS_OP,
            TokenKind::Less
            | TokenKind::LessEq
            | TokenKind::DblEq
            | TokenKind::NotEq
            | TokenKind::Gre
            | TokenKind::GreEq => Opcode310::COMPARE_OP,
            TokenKind::LeftOpen
            | TokenKind::RightOpen
            | TokenKind::Closed
            | TokenKind::Open
            | TokenKind::InOp => Opcode310::CALL_FUNCTION, // ERG_BINARY_RANGE,
            _ => {
                CompileError::feature_error(
                    self.cfg.input.clone(),
                    binop.loc(),
                    &binop.inspect().clone(),
                    String::from(binop.content),
                )
                .write_to_stderr();
                Opcode310::NOT_IMPLEMENTED
            }
        };
        let arg = match &binop.kind {
            TokenKind::Less => 0,
            TokenKind::LessEq => 1,
            TokenKind::DblEq => 2,
            TokenKind::NotEq => 3,
            TokenKind::Gre => 4,
            TokenKind::GreEq => 5,
            TokenKind::IsOp => 0,
            TokenKind::IsNotOp => 1,
            TokenKind::LeftOpen
            | TokenKind::RightOpen
            | TokenKind::Closed
            | TokenKind::Open
            | TokenKind::InOp => 2,
            _ => type_pair as usize,
        };
        self.write(instr, arg);
        self.stack_dec();
        match &binop.kind {
            TokenKind::LeftOpen
            | TokenKind::RightOpen
            | TokenKind::Open
            | TokenKind::Closed
            | TokenKind::InOp => {
                self.stack_dec();
            }
            _ => {}
        }
    }

    fn emit_binop_instr_311(&mut self, binop: Token, type_pair: TypePair) {
        let instr = match &binop.kind {
            TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::FloorDiv
            | TokenKind::Pow
            | TokenKind::Mod
            | TokenKind::AndOp
            | TokenKind::OrOp
            | TokenKind::BitAnd
            | TokenKind::BitOr
            | TokenKind::BitXor => Opcode311::BINARY_OP,
            TokenKind::IsOp | TokenKind::IsNotOp => Opcode311::IS_OP,
            TokenKind::Less
            | TokenKind::LessEq
            | TokenKind::DblEq
            | TokenKind::NotEq
            | TokenKind::Gre
            | TokenKind::GreEq => Opcode311::COMPARE_OP,
            TokenKind::LeftOpen
            | TokenKind::RightOpen
            | TokenKind::Closed
            | TokenKind::Open
            | TokenKind::InOp => {
                self.write(Opcode311::PRECALL, 2);
                self.write(0, 0);
                Opcode311::CALL
            }
            _ => {
                CompileError::feature_error(
                    self.cfg.input.clone(),
                    binop.loc(),
                    &binop.inspect().clone(),
                    String::from(binop.content),
                )
                .write_to_stderr();
                Opcode311::NOT_IMPLEMENTED
            }
        };
        let arg = match &binop.kind {
            TokenKind::Plus => BinOpCode::Add as usize,
            TokenKind::Minus => BinOpCode::Subtract as usize,
            TokenKind::Star => BinOpCode::Multiply as usize,
            TokenKind::Slash => BinOpCode::TrueDivide as usize,
            TokenKind::FloorDiv => BinOpCode::FloorDiv as usize,
            TokenKind::Pow => BinOpCode::Power as usize,
            TokenKind::Mod => BinOpCode::Remainder as usize,
            TokenKind::AndOp | TokenKind::BitAnd => BinOpCode::And as usize,
            TokenKind::OrOp | TokenKind::BitOr => BinOpCode::Or as usize,
            TokenKind::BitXor => BinOpCode::Xor as usize,
            TokenKind::Less => 0,
            TokenKind::LessEq => 1,
            TokenKind::DblEq => 2,
            TokenKind::NotEq => 3,
            TokenKind::Gre => 4,
            TokenKind::GreEq => 5,
            TokenKind::IsOp => 0,
            TokenKind::IsNotOp => 1,
            TokenKind::LeftOpen
            | TokenKind::RightOpen
            | TokenKind::Closed
            | TokenKind::Open
            | TokenKind::InOp => 2,
            _ => type_pair as usize,
        };
        self.write(instr, arg);
        match instr {
            Opcode311::CALL => {
                self.write_bytes(&[0; 8]);
            }
            Opcode311::BINARY_OP => {
                self.write_bytes(&[0; 2]);
            }
            Opcode311::COMPARE_OP => {
                self.write_bytes(&[0; 4]);
            }
            _ => {}
        }
        self.stack_dec();
        match &binop.kind {
            TokenKind::LeftOpen
            | TokenKind::RightOpen
            | TokenKind::Open
            | TokenKind::Closed
            | TokenKind::InOp => {
                self.stack_dec();
                if self.py_version.minor >= Some(11) {
                    self.stack_dec();
                }
            }
            _ => {}
        }
    }

    fn emit_del_instr(&mut self, mut args: Args) {
        let Some(Expr::Accessor(Accessor::Ident(ident))) = args.remove_left_or_key("obj") else {
            log!(err "del instruction requires an identifier");
            return;
        };
        log!(info "entered {} ({ident})", fn_name!());
        let escaped = escape_ident(ident);
        let name = self
            .local_search(&escaped, Name)
            .unwrap_or_else(|| self.register_name(escaped));
        self.write(DELETE_NAME, name.idx);
        self.emit_load_const(ValueObj::None);
    }

    fn emit_not_instr(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        let expr = args.remove_left_or_key("b").unwrap();
        self.emit_expr(expr);
        self.write(UNARY_NOT, 0);
    }

    fn emit_discard_instr(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        while let Some(arg) = args.try_remove(0) {
            self.emit_expr(arg);
            self.emit_pop_top();
        }
        self.emit_load_const(ValueObj::None);
    }

    fn deopt_instr(&mut self, kind: ControlKind, args: Args) {
        if !self.control_loaded {
            self.load_control();
        }
        let local = match kind {
            ControlKind::If => Identifier::public("if__"),
            ControlKind::For => Identifier::public("for__"),
            ControlKind::While => Identifier::public("while__"),
            ControlKind::With => Identifier::public("with__"),
            ControlKind::Discard => Identifier::public("discard__"),
            ControlKind::Assert => Identifier::public("assert__"),
            kind => todo!("{kind:?}"),
        };
        self.emit_call_local(local, args);
    }

    fn emit_if_instr(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        let init_stack_len = self.stack_len();
        let cond = args.remove(0);
        self.emit_expr(cond);
        let idx_pop_jump_if_false = self.lasti();
        // Opcode310::POP_JUMP_IF_FALSE == Opcode311::POP_JUMP_FORWARD_IF_FALSE
        self.stash_code();
        match args.remove(0) {
            // then block
            Expr::Lambda(lambda) => {
                // let params = self.gen_param_names(&lambda.params);
                self.emit_frameless_block(lambda.body, vec![]);
            }
            other => {
                self.emit_expr(other);
            }
        }
        if args.get(0).is_some() {
            let idx_jump_forward = self.lasti();
            self.stash_code();
            // else block
            let idx_else_begin = if self.py_version.minor >= Some(11) {
                self.lasti() - idx_pop_jump_if_false + 2
            } else {
                self.lasti()
            };
            match args.remove(0) {
                Expr::Lambda(lambda) => {
                    // let params = self.gen_param_names(&lambda.params);
                    self.emit_frameless_block(lambda.body, vec![]);
                }
                other => {
                    self.emit_expr(other);
                }
            }
            let code = self.pop_code();
            self.write(JUMP_FORWARD, idx_jump_forward);
            self.mut_code().extend(code);
            let code = self.pop_code();
            self.write(Opcode310::POP_JUMP_IF_FALSE, self.lasti() - idx_else_begin);
            self.mut_code().extend(code);
            // FIXME: this is a hack to make sure the stack is balanced
            while self.stack_len() != init_stack_len + 1 {
                self.stack_dec();
            }
        } else {
            // no else block
            self.write(JUMP_FORWARD, self.lasti() - 2);
            let code = self.pop_code();
            self.write(Opcode310::POP_JUMP_IF_FALSE, idx_pop_jump_if_false);
            self.mut_code().extend(code);
            self.emit_load_const(ValueObj::None);
            while self.stack_len() != init_stack_len + 1 {
                self.stack_dec();
            }
        }
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    fn emit_for_instr(&mut self, mut args: Args) {
        log!(info "entered {} ({})", fn_name!(), args);
        if !matches!(args.get(1).unwrap(), Expr::Lambda(_)) {
            return self.deopt_instr(ControlKind::For, args);
        }
        let _init_stack_len = self.stack_len();
        let iterable = args.remove(0);
        self.emit_expr(iterable);
        self.write(GET_ITER, 0);
        let idx_for_iter = self.lasti();
        self.stack_inc();
        self.stash_code();
        let Expr::Lambda(lambda) = args.remove(0) else { unreachable!() };
        // If there is nothing on the stack at the start, init_stack_len == 2 (an iterator and the first iterator value)
        let init_stack_len = self.stack_len();
        let params = self.gen_param_names(&lambda.params);
        // store the iterator value, stack_len == 1 or 2 in the end
        self.emit_frameless_block(lambda.body, params);
        if self.stack_len() > init_stack_len - 1 {
            self.emit_pop_top();
        }
        debug_assert_eq!(self.stack_len(), init_stack_len - 1); // the iterator is remained
        match self.py_version.minor {
            Some(11) => {
                self.write(Opcode311::JUMP_BACKWARD, idx_for_iter - 4);
            }
            _ => {
                self.write(Opcode308::JUMP_ABSOLUTE, idx_for_iter);
            }
        }
        let code = self.pop_code();
        self.write(FOR_ITER, idx_for_iter);
        self.mut_code().extend(code);
        self.stack_dec();
        self.emit_load_const(ValueObj::None);
        debug_assert_eq!(self.stack_len(), _init_stack_len + 1);
    }

    fn emit_while_instr(&mut self, mut args: Args) {
        log!(info "entered {} ({})", fn_name!(), args);
        if !matches!(args.get(1).unwrap(), Expr::Lambda(_)) {
            return self.deopt_instr(ControlKind::While, args);
        }
        let _init_stack_len = self.stack_len();
        // e.g. is_foo!: () => Bool, do!(is_bar)
        let cond_block = args.remove(0);
        let cond = match cond_block {
            Expr::Lambda(mut lambda) => lambda.body.remove(0),
            Expr::Accessor(acc) => Expr::Accessor(acc).call_expr(Args::empty()),
            _ => todo!(),
        };
        self.emit_expr(cond.clone());
        let idx_while = self.lasti();
        self.stash_code();
        self.stack_dec();
        let Expr::Lambda(lambda) = args.remove(0) else { unreachable!() };
        let init_stack_len = self.stack_len();
        let params = self.gen_param_names(&lambda.params);
        self.emit_frameless_block(lambda.body, params);
        if self.stack_len() > init_stack_len {
            self.emit_pop_top();
        }
        self.emit_expr(cond);
        if self.py_version.minor >= Some(11) {
            self.write(Opcode311::POP_JUMP_BACKWARD_IF_TRUE, idx_while - 2);
        } else {
            self.write(Opcode310::POP_JUMP_IF_TRUE, idx_while - 2);
        }
        self.stack_dec();
        let idx_end = if self.py_version.minor >= Some(11) {
            idx_while - 1
        } else {
            0
        };
        let code = self.pop_code();
        self.write(Opcode310::POP_JUMP_IF_FALSE, idx_end);
        self.mut_code().extend(code);
        self.emit_load_const(ValueObj::None);
        debug_assert_eq!(self.stack_len(), _init_stack_len + 1);
    }

    fn emit_match_instr(&mut self, mut args: Args, _use_erg_specific: bool) {
        log!(info "entered {}", fn_name!());
        let init_stack_len = self.stack_len();
        let expr = args.remove(0);
        self.emit_expr(expr);
        let len = args.len();
        let mut jump_forward_points = vec![];
        while let Some(expr) = args.try_remove(0) {
            if len > 1 && !args.is_empty() {
                self.dup_top();
            }
            // compilerで型チェック済み(可読性が下がるため、matchでNamedは使えない)
            let Expr::Lambda(mut lambda) = expr else { unreachable!() };
            debug_power_assert!(lambda.params.len(), ==, 1);
            if !lambda.params.defaults.is_empty() {
                todo!("default values in match expression are not supported yet")
            }
            let param = lambda.params.non_defaults.remove(0);
            let pop_jump_point = self.emit_match_pattern(param, args.is_empty());
            self.emit_frameless_block(lambda.body, Vec::new());
            // If we move on to the next arm, the stack size will increase
            // so `self.stack_dec();` for now (+1 at the end).
            self.stack_dec();
            if let Some(pop_jump_point) = pop_jump_point {
                let idx = if self.py_version.minor >= Some(11) {
                    pop_jump_point - 2
                } else {
                    2
                };
                let code = self.pop_code();
                self.write(Opcode310::POP_JUMP_IF_FALSE, idx);
                self.mut_code().extend(code);
                jump_forward_points.push(self.lasti());
                self.stash_code();
            }
        }
        for jump_point in jump_forward_points.into_iter().rev() {
            let code = self.pop_code();
            self.write(JUMP_FORWARD, jump_point);
            self.mut_code().extend(code);
        }
        self.stack_inc();
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    fn emit_match_pattern(
        &mut self,
        param: NonDefaultParamSignature,
        is_last_arm: bool,
    ) -> Option<usize> {
        log!(info "entered {}", fn_name!());
        let mut pop_jump_point = None;
        if let Some(t_spec) = param.t_spec_as_expr {
            // If it's the last arm, there's no need to inspect it
            if !is_last_arm {
                // < v3.11:
                // arg
                // ↓ LOAD_NAME(in_operator)
                // arg in_operator
                // ↓ ROT_TWO
                // in_operator arg
                // ↓ load expr
                // in_operator arg expr
                //
                // in v3.11:
                // arg null
                // ↓ SWAP 1
                // null arg
                // ↓ LOAD_NAME(in_operator)
                // null arg in_operator
                // ↓ SWAP 1
                // null in_operator arg
                // ↓ load expr
                // null in_operator arg expr
                if self.py_version.minor >= Some(11) {
                    self.emit_push_null();
                    self.rot2();
                }
                if !self.in_op_loaded {
                    self.load_in_op();
                }
                self.emit_load_name_instr(Identifier::private("#in_operator"));
                self.rot2();
                self.emit_expr(t_spec);
                if self.py_version.minor >= Some(11) {
                    self.emit_precall_and_call(2);
                } else {
                    self.write(Opcode310::CALL_FUNCTION, 2);
                }
                self.stack_dec();
                pop_jump_point = Some(self.lasti());
                self.stash_code();
                self.stack_dec();
            }
        }
        match param.raw.pat {
            ParamPattern::VarName(name) => {
                let ident = erg_parser::ast::Identifier::private_from_varname(name);
                let ident = Identifier::new(ident, None, param.vi);
                self.emit_store_instr(ident, AccessKind::Name);
            }
            ParamPattern::Discard(_) => {
                self.emit_pop_top();
            }
            _other => unreachable!(),
        }
        pop_jump_point
    }

    fn emit_with_instr_311(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        if !matches!(args.get(1).unwrap(), Expr::Lambda(_)) {
            return self.deopt_instr(ControlKind::With, args);
        }
        let expr = args.remove(0);
        let Expr::Lambda(lambda) = args.remove(0) else { unreachable!() };
        let params = self.gen_param_names(&lambda.params);
        self.emit_expr(expr);
        self.write(Opcode311::BEFORE_WITH, 0);
        // push __exit__, __enter__() to the stack
        self.stack_inc_n(2);
        let lambda_line = lambda.body.last().unwrap().ln_begin().unwrap_or(0);
        self.emit_with_block(lambda.body, params);
        let stash = Identifier::private_with_line(self.fresh_gen.fresh_varname(), lambda_line);
        self.emit_store_instr(stash.clone(), Name);
        self.emit_load_const(ValueObj::None);
        self.emit_load_const(ValueObj::None);
        self.emit_load_const(ValueObj::None);
        self.emit_precall_and_call(2);
        self.emit_pop_top();
        let idx_jump_forward = self.lasti();
        self.stash_code();
        self.write(Opcode311::PUSH_EXC_INFO, 0);
        self.write(Opcode308::WITH_EXCEPT_START, 0);
        self.write(Opcode311::POP_JUMP_FORWARD_IF_TRUE, 4);
        self.write(Opcode311::RERAISE, 0);
        self.write(Opcode311::COPY, 3);
        self.write(Opcode311::POP_EXCEPT, 0);
        self.write(Opcode311::RERAISE, 1);
        self.emit_pop_top();
        self.write(Opcode311::POP_EXCEPT, 0);
        self.emit_pop_top();
        self.emit_pop_top();
        let code = self.pop_code();
        self.write(Opcode311::JUMP_FORWARD, idx_jump_forward);
        self.mut_code().extend(code);
        self.emit_load_name_instr(stash);
    }

    fn emit_with_instr_310(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        if !matches!(args.get(1).unwrap(), Expr::Lambda(_)) {
            return self.deopt_instr(ControlKind::With, args);
        }
        let expr = args.remove(0);
        let Expr::Lambda(lambda) = args.remove(0) else { unreachable!() };
        let params = self.gen_param_names(&lambda.params);
        self.emit_expr(expr);
        let idx_setup_with = self.lasti();
        self.stash_code();
        // push __exit__, __enter__() to the stack
        self.stack_inc_n(2);
        let lambda_line = lambda.body.last().unwrap().ln_begin().unwrap_or(0);
        self.emit_with_block(lambda.body, params);
        let stash = Identifier::private_with_line(self.fresh_gen.fresh_varname(), lambda_line);
        self.emit_store_instr(stash.clone(), Name);
        self.write(POP_BLOCK, 0);
        self.emit_load_const(ValueObj::None);
        self.write(Opcode310::DUP_TOP, 0);
        self.stack_inc();
        self.write(Opcode310::DUP_TOP, 0);
        self.stack_inc();
        self.write(Opcode310::CALL_FUNCTION, 3);
        self.stack_dec_n((1 + 3) - 1);
        self.emit_pop_top();
        let idx_jump_forward = self.lasti();
        self.stash_code();
        let setup_with = self.lasti() - idx_setup_with - 2;
        self.write(Opcode310::WITH_EXCEPT_START, 0);
        self.write(Opcode310::POP_JUMP_IF_TRUE, self.lasti() + 1);
        self.write(Opcode310::RERAISE, 1);
        self.emit_pop_top();
        self.write(Opcode310::POP_EXCEPT, 0);
        let idx_end = self.lasti();
        let code = self.pop_code();
        self.write(JUMP_FORWARD, idx_end - idx_jump_forward - 2);
        self.mut_code().extend(code);
        // self.edit_code(idx_jump_forward + 1, (idx_end - idx_jump_forward - 2) / 2);
        let code = self.pop_code();
        self.write(Opcode310::SETUP_WITH, setup_with);
        self.mut_code().extend(code);
        self.emit_load_name_instr(stash);
    }

    fn emit_with_instr_308(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        if !matches!(args.get(1).unwrap(), Expr::Lambda(_)) {
            return self.deopt_instr(ControlKind::With, args);
        }
        let expr = args.remove(0);
        let Expr::Lambda(lambda) = args.remove(0) else { unreachable!() };
        let params = self.gen_param_names(&lambda.params);
        self.emit_expr(expr);
        let idx_setup_with = self.lasti();
        self.stash_code();
        // push __exit__, __enter__() to the stack
        // self.stack_inc_n(2);
        let lambda_line = lambda.body.last().unwrap().ln_begin().unwrap_or(0);
        self.emit_with_block(lambda.body, params);
        let stash = Identifier::private_with_line(self.fresh_gen.fresh_varname(), lambda_line);
        self.emit_store_instr(stash.clone(), Name);
        self.write(POP_BLOCK, 0);
        self.write(Opcode308::BEGIN_FINALLY, 0);
        self.write(Opcode308::WITH_CLEANUP_START, 0);
        let code = self.pop_code();
        self.write(
            Opcode308::SETUP_WITH,
            (self.lasti() - idx_setup_with - 2) / 2,
        );
        self.mut_code().extend(code);
        self.write(Opcode308::WITH_CLEANUP_FINISH, 0);
        self.write(Opcode308::END_FINALLY, 0);
        self.emit_load_name_instr(stash);
    }

    fn emit_call(&mut self, call: Call) {
        log!(info "entered {} ({call})", fn_name!());
        let init_stack_len = self.stack_len();
        // Python cannot distinguish at compile time between a method call and a attribute call
        if let Some(attr_name) = call.attr_name {
            self.emit_call_method(*call.obj, attr_name, call.args);
        } else {
            match *call.obj {
                Expr::Accessor(Accessor::Ident(ident)) if ident.vis().is_private() => {
                    self.emit_call_local(ident, call.args)
                }
                other => {
                    let is_py_api = other.is_py_api();
                    self.emit_push_null();
                    self.emit_expr(other);
                    self.emit_args_311(call.args, Name, is_py_api);
                }
            }
        }
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    fn emit_call_local(&mut self, local: Identifier, args: Args) {
        log!(info "entered {}", fn_name!());
        match &local.inspect()[..] {
            "assert" => self.emit_assert_instr(args),
            "Del" => self.emit_del_instr(args),
            "not" => self.emit_not_instr(args),
            "discard" => self.emit_discard_instr(args),
            "for" | "for!" => self.emit_for_instr(args),
            "while!" => self.emit_while_instr(args),
            "if" | "if!" => self.emit_if_instr(args),
            "match" | "match!" => self.emit_match_instr(args, true),
            "with!" => match self.py_version.minor {
                Some(11) => self.emit_with_instr_311(args),
                Some(10) => self.emit_with_instr_310(args),
                Some(9 | 8 | 7) => self.emit_with_instr_308(args),
                _ => todo!("not supported Python version"),
            },
            // "pyimport" | "py" are here
            _ => {
                let is_py_api = local.is_py_api();
                self.emit_push_null();
                self.emit_load_name_instr(local);
                self.emit_args_311(args, Name, is_py_api);
            }
        }
    }

    fn emit_call_method(&mut self, obj: Expr, method_name: Identifier, args: Args) {
        log!(info "entered {}", fn_name!());
        match &method_name.inspect()[..] {
            "update!" => {
                if self.py_version.minor >= Some(11) {
                    return self.emit_call_update_311(obj, args);
                } else {
                    return self.emit_call_update_310(obj, args);
                }
            }
            "return" if obj.ref_t().is_callable() => {
                return self.emit_return_instr(args);
            }
            // TODO: create `Generator` type
            "yield" /* if obj.ref_t().is_callable() */ => {
                return self.emit_yield_instr(args);
            }
            _ => {}
        }
        if let Some(func_name) = debind(&method_name) {
            return self.emit_call_fake_method(obj, func_name, method_name, args);
        }
        let is_py_api = method_name.is_py_api();
        self.emit_expr(obj);
        self.emit_load_method_instr(method_name);
        self.emit_args_311(args, BoundAttr, is_py_api);
    }

    fn emit_var_args_311(&mut self, pos_len: usize, var_args: &PosArg) {
        if pos_len > 0 {
            self.write(BUILD_LIST, pos_len);
        }
        self.emit_expr(var_args.expr.clone());
        if pos_len > 0 {
            self.write(Opcode310::LIST_EXTEND, 1);
            self.write(Opcode310::LIST_TO_TUPLE, 0);
        }
    }

    fn emit_var_args_38(&mut self, pos_len: usize, var_args: &PosArg) {
        if pos_len > 0 {
            self.write(BUILD_TUPLE, pos_len);
        }
        self.emit_expr(var_args.expr.clone());
        if pos_len > 0 {
            self.write(Opcode308::BUILD_TUPLE_UNPACK_WITH_CALL, 2);
        }
    }

    fn emit_args_311(&mut self, mut args: Args, kind: AccessKind, is_py_api: bool) {
        let argc = args.len();
        let pos_len = args.pos_args.len();
        let mut kws = Vec::with_capacity(args.kw_len());
        while let Some(arg) = args.try_remove_pos(0) {
            self.emit_expr(arg.expr);
        }
        if let Some(var_args) = &args.var_args {
            if self.py_version.minor >= Some(10) {
                self.emit_var_args_311(pos_len, var_args);
            } else {
                self.emit_var_args_38(pos_len, var_args);
            }
        }
        while let Some(arg) = args.try_remove_kw(0) {
            let kw = if is_py_api {
                arg.keyword.content
            } else {
                escape_name(&arg.keyword.content, &VisibilityModifier::Private, 0, 0)
            };
            kws.push(ValueObj::Str(kw));
            self.emit_expr(arg.expr);
        }
        let kwsc = if !kws.is_empty() {
            self.emit_call_kw_instr(argc, kws);
            #[allow(clippy::bool_to_int_with_if)]
            if self.py_version.minor >= Some(11) {
                0
            } else {
                1
            }
        } else {
            if args.var_args.is_some() {
                let arg = if kws.is_empty() { 0 } else { 1 };
                self.write(CALL_FUNCTION_EX, arg);
            } else {
                self.emit_call_instr(argc, kind);
            }
            0
        };
        // (1 (subroutine) + argc + kwsc) input objects -> 1 return object
        self.stack_dec_n((1 + argc + kwsc) - 1);
    }

    /// X.update! x -> x + 1
    /// => X = mutate_operator((x -> x + 1)(X))
    /// TODO: should be `X = X + 1` in the above case
    fn emit_call_update_311(&mut self, obj: Expr, mut args: Args) {
        log!(info "entered {}", fn_name!());
        let Expr::Accessor(acc) = obj else { unreachable!() };
        let func = args.remove_left_or_key("f").unwrap();
        if !self.mutate_op_loaded {
            self.load_mutate_op();
        }
        self.emit_push_null();
        self.emit_load_name_instr(Identifier::private("#mutate_operator"));
        self.emit_push_null();
        self.emit_expr(func);
        self.emit_acc(acc.clone());
        self.emit_precall_and_call(1);
        // (1 (subroutine) + argc) input objects -> 1 return object
        // self.stack_dec_n((1 + 1) - 1);
        self.stack_dec();
        self.emit_precall_and_call(1);
        self.stack_dec();
        self.store_acc(acc);
        self.emit_load_const(ValueObj::None);
    }

    /// X.update! x -> x + 1
    /// X = mutate_operator((x -> x + 1)(X))
    /// X = X + 1
    fn emit_call_update_310(&mut self, obj: Expr, mut args: Args) {
        log!(info "entered {}", fn_name!());
        let Expr::Accessor(acc) = obj else { unreachable!() };
        let func = args.remove_left_or_key("f").unwrap();
        if !self.mutate_op_loaded {
            self.load_mutate_op();
        }
        self.emit_load_name_instr(Identifier::private("#mutate_operator"));
        self.emit_expr(func);
        self.emit_acc(acc.clone());
        self.write(Opcode310::CALL_FUNCTION, 1);
        // (1 (subroutine) + argc) input objects -> 1 return object
        self.stack_dec_n((1 + 1) - 1);
        self.write(Opcode310::CALL_FUNCTION, 1);
        self.stack_dec();
        self.store_acc(acc);
        self.emit_load_const(ValueObj::None);
    }

    // TODO: use exception
    fn emit_return_instr(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        if args.is_empty() {
            self.emit_load_const(ValueObj::None);
        } else {
            self.emit_expr(args.remove(0));
        }
        self.write(RETURN_VALUE, 0);
    }

    fn emit_yield_instr(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        if args.is_empty() {
            self.emit_load_const(ValueObj::None);
        } else {
            self.emit_expr(args.remove(0));
        }
        self.write(YIELD_VALUE, 0);
    }

    /// 1.abs() => abs(1)
    fn emit_call_fake_method(
        &mut self,
        obj: Expr,
        func_name: Str,
        mut method_name: Identifier,
        mut args: Args,
    ) {
        log!(info "entered {}", fn_name!());
        method_name.raw.vis = VisModifierSpec::Private;
        method_name.vi.py_name = Some(func_name);
        self.emit_push_null();
        self.emit_load_name_instr(method_name);
        args.insert_pos(0, PosArg::new(obj));
        self.emit_args_311(args, Name, true);
    }

    // assert takes 1 or 2 arguments (0: cond, 1: message)
    fn emit_assert_instr(&mut self, mut args: Args) {
        log!(info "entered {}", fn_name!());
        let init_stack_len = self.stack_len();
        self.emit_expr(args.remove(0));
        let pop_jump_point = self.lasti();
        self.stash_code();
        self.stack_dec();
        if self.py_version.minor >= Some(10) {
            self.write(Opcode310::LOAD_ASSERTION_ERROR, 0);
            self.stack_inc();
        } else {
            self.emit_load_global_instr(Identifier::public("AssertionError"));
        }
        if let Some(expr) = args.try_remove(0) {
            self.emit_expr(expr);
            if self.py_version.minor >= Some(11) {
                self.emit_precall_and_call(0);
            } else {
                self.write(Opcode310::CALL_FUNCTION, 1);
            }
        }
        self.write(RAISE_VARARGS, 1);
        self.stack_dec();
        let code = self.pop_code();
        self.write(Opcode310::POP_JUMP_IF_TRUE, pop_jump_point);
        self.mut_code().extend(code);
        self.emit_load_const(ValueObj::None);
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    // TODO: list comprehension
    fn emit_array(&mut self, array: Array) {
        let init_stack_len = self.stack_len();
        if !self.cfg.no_std {
            self.emit_push_null();
            self.emit_load_name_instr(Identifier::public("Array"));
        }
        match array {
            Array::Normal(mut arr) => {
                let len = arr.elems.len();
                while let Some(arg) = arr.elems.try_remove_pos(0) {
                    self.emit_expr(arg.expr);
                }
                self.write(BUILD_LIST, len);
                if len == 0 {
                    self.stack_inc();
                } else {
                    self.stack_dec_n(len - 1);
                }
            }
            Array::WithLength(arr) => {
                self.emit_expr(*arr.elem);
                self.write(BUILD_LIST, 1);
                self.emit_call_instr(1, Name);
                self.stack_dec();
                self.emit_expr(*arr.len);
                self.emit_binop_instr(Token::dummy(TokenKind::Star, "*"), TypePair::ArrayNat);
                return;
            }
            other => todo!("{other}"),
        }
        if !self.cfg.no_std {
            self.emit_call_instr(1, Name);
            self.stack_dec();
        }
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    // TODO: tuple comprehension
    // TODO: tuples can be const
    fn emit_tuple(&mut self, tuple: Tuple) {
        match tuple {
            Tuple::Normal(mut tup) => {
                let len = tup.elems.len();
                while let Some(arg) = tup.elems.try_remove_pos(0) {
                    self.emit_expr(arg.expr);
                }
                self.write(BUILD_TUPLE, len);
                if len == 0 {
                    self.stack_inc();
                } else {
                    self.stack_dec_n(len - 1);
                }
            }
        }
    }

    fn emit_set(&mut self, set: crate::hir::Set) {
        match set {
            crate::hir::Set::Normal(mut set) => {
                let len = set.elems.len();
                while let Some(arg) = set.elems.try_remove_pos(0) {
                    self.emit_expr(arg.expr);
                }
                self.write(BUILD_SET, len);
                if len == 0 {
                    self.stack_inc();
                } else {
                    self.stack_dec_n(len - 1);
                }
            }
            crate::hir::Set::WithLength(st) => {
                self.emit_expr(*st.elem);
                self.write(BUILD_SET, 1);
            }
        }
    }

    fn emit_dict(&mut self, dict: crate::hir::Dict) {
        match dict {
            crate::hir::Dict::Normal(dic) => {
                let len = dic.kvs.len();
                for kv in dic.kvs.into_iter() {
                    self.emit_expr(kv.key);
                    self.emit_expr(kv.value);
                }
                self.write(BUILD_MAP, len);
                if len == 0 {
                    self.stack_inc();
                } else {
                    self.stack_dec_n(2 * len - 1);
                }
            }
            other => todo!("{other}"),
        }
    }

    #[allow(clippy::identity_op)]
    fn emit_record(&mut self, rec: Record) {
        log!(info "entered {} ({rec})", fn_name!());
        let init_stack_len = self.stack_len();
        let attrs_len = rec.attrs.len();
        self.emit_push_null();
        // making record type
        let ident = Identifier::private("#NamedTuple");
        self.emit_load_name_instr(ident);
        // record name, let it be anonymous
        self.emit_load_const("Record");
        for field in rec.attrs.iter() {
            self.emit_load_const(ValueObj::Str(field.sig.ident().inspect().clone()));
        }
        self.write(BUILD_LIST, attrs_len);
        if attrs_len == 0 {
            self.stack_inc();
        } else {
            self.stack_dec_n(attrs_len - 1);
        }
        self.emit_call_instr(2, Name);
        // (1 (subroutine) + argc + kwsc) input objects -> 1 return object
        self.stack_dec_n((1 + 2 + 0) - 1);
        let ident = Identifier::private("#rec");
        self.emit_store_instr(ident, Name);
        // making record instance
        let ident = Identifier::private("#rec");
        self.emit_push_null();
        self.emit_load_name_instr(ident);
        for field in rec.attrs.into_iter() {
            self.emit_frameless_block(field.body.block, vec![]);
        }
        self.emit_call_instr(attrs_len, Name);
        // (1 (subroutine) + argc + kwsc) input objects -> 1 return object
        self.stack_dec_n((1 + attrs_len + 0) - 1);
        debug_assert_eq!(self.stack_len(), init_stack_len + 1);
    }

    /// Emits independent code blocks (e.g., linked other modules)
    fn emit_code(&mut self, code: Block) {
        let mut gen = self.inherit();
        let code = gen.emit_block(code, None, vec![], 0);
        self.emit_load_const(code);
    }

    pub(crate) fn get_root(acc: &Accessor) -> Identifier {
        match acc {
            Accessor::Ident(ident) => ident.clone(),
            Accessor::Attr(attr) => {
                if let Expr::Accessor(acc) = attr.obj.as_ref() {
                    Self::get_root(acc)
                } else {
                    todo!("{:?}", attr.obj)
                }
            }
        }
    }

    fn emit_import(&mut self, acc: Accessor) {
        self.emit_load_const(0i32);
        self.emit_load_const(ValueObj::None);
        let full_name = Str::from(
            acc.qual_name()
                .map_or(acc.show(), |s| s.replace(".__init__", "")),
        );
        let name = self
            .local_search(&full_name, Name)
            .unwrap_or_else(|| self.register_name(full_name));
        self.write(IMPORT_NAME, name.idx);
        let root = Self::get_root(&acc);
        self.emit_store_instr(root, Name);
        self.stack_dec();
    }

    fn emit_compound(&mut self, chunks: Block) {
        let is_module_loading_chunks = chunks
            .get(2)
            .map(|chunk| {
                option_enum_unwrap!(chunk, Expr::Call)
                    .map(|call| call.obj.show_acc().as_ref().map(|s| &s[..]) == Some("exec"))
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        if !self.module_type_loaded && is_module_loading_chunks {
            self.load_module_type();
            self.module_type_loaded = true;
        }
        let init_stack_len = self.stack_len();
        for chunk in chunks.into_iter() {
            self.emit_chunk(chunk);
            if self.stack_len() == init_stack_len + 1 {
                self.emit_pop_top();
            }
        }
        self.cancel_if_pop_top();
    }

    fn push_lnotab(&mut self, expr: &Expr) {
        let ln_begin = expr.ln_begin().unwrap_or(0);
        if ln_begin > self.cur_block().prev_lineno {
            let sd = self.lasti() - self.cur_block().prev_lasti;
            let ld = ln_begin - self.cur_block().prev_lineno;
            if ld != 0 {
                if sd != 0 {
                    self.mut_cur_block_codeobj().lnotab.push(sd as u8);
                    self.mut_cur_block_codeobj().lnotab.push(ld as u8);
                } else {
                    // empty lines
                    if let Some(last_ld) = self.mut_cur_block_codeobj().lnotab.last_mut() {
                        *last_ld += ld as u8;
                    } else {
                        // a block starts with an empty line
                        self.mut_cur_block_codeobj().lnotab.push(0);
                        self.mut_cur_block_codeobj().lnotab.push(ld as u8);
                    }
                }
                self.mut_cur_block().prev_lineno += ld;
                self.mut_cur_block().prev_lasti = self.lasti();
            } else {
                self.crash("codegen failed: invalid bytecode format");
            }
        }
    }

    fn emit_chunk(&mut self, chunk: Expr) {
        log!(info "entered {} ({chunk})", fn_name!());
        self.push_lnotab(&chunk);
        match chunk {
            Expr::Lit(lit) => self.emit_load_const(lit.value),
            Expr::Accessor(acc) => self.emit_acc(acc),
            Expr::Def(def) => self.emit_def(def),
            Expr::ClassDef(class) => self.emit_class_def(class),
            Expr::PatchDef(patch) => self.emit_patch_def(patch),
            Expr::ReDef(attr) => self.emit_redef(attr),
            Expr::Lambda(lambda) => self.emit_lambda(lambda),
            Expr::UnaryOp(unary) => self.emit_unaryop(unary),
            Expr::BinOp(bin) => self.emit_binop(bin),
            Expr::Call(call) => self.emit_call(call),
            Expr::Array(arr) => self.emit_array(arr),
            Expr::Tuple(tup) => self.emit_tuple(tup),
            Expr::Set(set) => self.emit_set(set),
            Expr::Dict(dict) => self.emit_dict(dict),
            Expr::Record(rec) => self.emit_record(rec),
            Expr::Code(code) => self.emit_code(code),
            Expr::Compound(chunks) => self.emit_compound(chunks),
            Expr::Import(acc) => self.emit_import(acc),
            Expr::Dummy(_) | Expr::TypeAsc(_) => {}
        }
    }

    fn emit_expr(&mut self, expr: Expr) {
        log!(info "entered {} ({expr})", fn_name!());
        self.push_lnotab(&expr);
        let mut wrapped = true;
        if !self.cfg.no_std {
            match expr.ref_t().derefine() {
                Bool => {
                    self.emit_push_null();
                    self.emit_load_name_instr(Identifier::public("Bool"));
                }
                Nat => {
                    self.emit_push_null();
                    self.emit_load_name_instr(Identifier::public("Nat"));
                }
                Int => {
                    self.emit_push_null();
                    self.emit_load_name_instr(Identifier::public("Int"));
                }
                Float => {
                    self.emit_push_null();
                    self.emit_load_name_instr(Identifier::public("Float"));
                }
                Str => {
                    self.emit_push_null();
                    self.emit_load_name_instr(Identifier::public("Str"));
                }
                other => match &other.qual_name()[..] {
                    "Bytes" => {
                        self.emit_push_null();
                        self.emit_load_name_instr(Identifier::public("Bytes"));
                    }
                    "Array" => {
                        self.emit_push_null();
                        self.emit_load_name_instr(Identifier::public("Array"));
                    }
                    "Dict" => {
                        self.emit_push_null();
                        self.emit_load_name_instr(Identifier::public("Dict"));
                    }
                    "Set" => {
                        self.emit_push_null();
                        self.emit_load_name_instr(Identifier::public("Set"));
                    }
                    "Tuple" => {
                        self.emit_push_null();
                        self.emit_load_name_instr(Identifier::public("tuple"));
                    }
                    _ => {
                        wrapped = false;
                    }
                },
            }
        }
        match expr {
            Expr::Lit(lit) => self.emit_load_const(lit.value),
            Expr::Accessor(acc) => self.emit_acc(acc),
            Expr::Def(def) => self.emit_def(def),
            Expr::ClassDef(class) => self.emit_class_def(class),
            Expr::PatchDef(patch) => self.emit_patch_def(patch),
            Expr::ReDef(attr) => self.emit_redef(attr),
            Expr::Lambda(lambda) => self.emit_lambda(lambda),
            Expr::UnaryOp(unary) => self.emit_unaryop(unary),
            Expr::BinOp(bin) => self.emit_binop(bin),
            Expr::Call(call) => self.emit_call(call),
            Expr::Array(arr) => self.emit_array(arr),
            Expr::Tuple(tup) => self.emit_tuple(tup),
            Expr::Set(set) => self.emit_set(set),
            Expr::Dict(dict) => self.emit_dict(dict),
            Expr::Record(rec) => self.emit_record(rec),
            Expr::Code(code) => self.emit_code(code),
            Expr::Compound(chunks) => self.emit_compound(chunks),
            Expr::TypeAsc(tasc) => self.emit_expr(*tasc.expr),
            Expr::Import(acc) => self.emit_import(acc),
            Expr::Dummy(_) => {}
        }
        if !self.cfg.no_std && wrapped {
            self.emit_call_instr(1, Name);
            self.stack_dec();
        }
    }

    /// forブロックなどで使う
    fn emit_frameless_block(&mut self, block: Block, params: Vec<Str>) {
        log!(info "entered {}", fn_name!());
        let line = block.ln_begin().unwrap_or(0);
        for param in params {
            self.emit_store_instr(Identifier::public_with_line(DOT, param, line), Name);
        }
        let init_stack_len = self.stack_len();
        for chunk in block.into_iter() {
            self.emit_chunk(chunk);
            if self.stack_len() > init_stack_len {
                self.emit_pop_top();
            }
        }
        self.cancel_if_pop_top();
    }

    fn emit_with_block(&mut self, block: Block, params: Vec<Str>) {
        log!(info "entered {}", fn_name!());
        let line = block.ln_begin().unwrap_or(0);
        for param in params {
            self.emit_store_instr(Identifier::public_with_line(DOT, param, line), Name);
        }
        let init_stack_len = self.stack_len();
        for chunk in block.into_iter() {
            self.emit_chunk(chunk);
            // __exit__, __enter__() are on the stack
            if self.stack_len() > init_stack_len {
                self.emit_pop_top();
            }
        }
        self.cancel_if_pop_top();
    }

    fn emit_class_block(&mut self, class: ClassDef) -> CodeObj {
        log!(info "entered {}", fn_name!());
        let name = class.sig.ident().inspect().clone();
        self.unit_size += 1;
        let firstlineno = match class.methods.get(0).and_then(|def| def.ln_begin()) {
            Some(l) => l,
            None => class.sig.ln_begin().unwrap_or(0),
        };
        self.units.push(PyCodeGenUnit::new(
            self.unit_size,
            self.py_version,
            vec![],
            Str::rc(self.cfg.input.enclosed_name()),
            &name,
            firstlineno,
            0,
        ));
        let init_stack_len = self.stack_len();
        let mod_name = self.toplevel_block_codeobj().name.clone();
        self.emit_load_const(mod_name);
        self.emit_store_instr(Identifier::public("__module__"), Name);
        self.emit_load_const(name);
        self.emit_store_instr(Identifier::public("__qualname__"), Name);
        self.emit_init_method(&class.sig, class.__new__.clone());
        if class.need_to_gen_new {
            self.emit_new_func(&class.sig, class.__new__);
        }
        if !class.methods.is_empty() {
            self.emit_frameless_block(class.methods, vec![]);
        }
        if self.stack_len() == init_stack_len {
            self.emit_load_const(ValueObj::None);
        }
        self.write(RETURN_VALUE, 0);
        if self.stack_len() > 1 {
            let block_id = self.cur_block().id;
            let stack_len = self.stack_len();
            CompileError::stack_bug(
                self.input().clone(),
                Location::Unknown,
                stack_len,
                block_id,
                fn_name_full!(),
            )
            .write_to_stderr();
            self.crash("error in emit_class_block: invalid stack size");
        }
        // flagging
        if !self.cur_block_codeobj().varnames.is_empty() {
            self.mut_cur_block_codeobj().flags += CodeObjFlags::NewLocals as u32;
        }
        // end of flagging
        let unit = self.units.pop().unwrap();
        if !self.units.is_empty() {
            let ld = unit.prev_lineno - self.cur_block().prev_lineno;
            if ld != 0 {
                if let Some(l) = self.mut_cur_block_codeobj().lnotab.last_mut() {
                    *l += ld as u8;
                }
                self.mut_cur_block().prev_lineno += ld;
            }
        }
        unit.codeobj
    }

    fn emit_init_method(&mut self, sig: &Signature, __new__: Type) {
        log!(info "entered {}", fn_name!());
        let new_first_param = __new__.non_default_params().unwrap().first();
        let line = sig.ln_begin().unwrap_or(0);
        let class_name = sig.ident().inspect();
        let mut ident = Identifier::public_with_line(DOT, Str::ever("__init__"), line);
        ident.vi.t = __new__.clone();
        let self_param = VarName::from_str_and_line(Str::ever("self"), line);
        let vi = VarInfo::nd_parameter(
            __new__.return_t().unwrap().clone(),
            ident.vi.def_loc.clone(),
            "?".into(),
        );
        let raw =
            erg_parser::ast::NonDefaultParamSignature::new(ParamPattern::VarName(self_param), None);
        let self_param = NonDefaultParamSignature::new(raw, vi, None);
        let (param_name, params) = if let Some(new_first_param) = new_first_param {
            let param_name = new_first_param
                .name()
                .cloned()
                .unwrap_or_else(|| self.fresh_gen.fresh_varname());
            let param = VarName::from_str_and_line(param_name.clone(), line);
            let raw =
                erg_parser::ast::NonDefaultParamSignature::new(ParamPattern::VarName(param), None);
            let vi = VarInfo::nd_parameter(
                new_first_param.typ().clone(),
                ident.vi.def_loc.clone(),
                "?".into(),
            );
            let param = NonDefaultParamSignature::new(raw, vi, None);
            let params = Params::new(vec![self_param, param], None, vec![], None);
            (param_name, params)
        } else {
            ("_".into(), Params::single(self_param))
        };
        let bounds = TypeBoundSpecs::empty();
        let subr_sig = SubrSignature::new(ident, bounds, params, sig.t_spec_with_op().cloned());
        let mut attrs = vec![];
        match new_first_param.map(|pt| pt.typ()) {
            // namedtupleは仕様上::xなどの名前を使えない
            // {x = Int; y = Int}
            //   => self::x = %x.x; self::y = %x.y
            // {.x = Int; .y = Int}
            //   => self.x = %x.x; self.y = %x.y
            // () => pass
            Some(Type::Record(rec)) => {
                for field in rec.keys() {
                    let obj =
                        Expr::Accessor(Accessor::private_with_line(Str::from(&param_name), line));
                    let ident = erg_parser::ast::Identifier::public(field.symbol.clone());
                    let expr = obj.attr_expr(Identifier::bare(ident));
                    let obj = Expr::Accessor(Accessor::private_with_line(Str::ever("self"), line));
                    let dot = if field.vis.is_private() {
                        VisModifierSpec::Private
                    } else {
                        VisModifierSpec::Public(DOT)
                    };
                    let attr = erg_parser::ast::Identifier::new(
                        dot,
                        VarName::from_str(field.symbol.clone()),
                    );
                    let attr = obj.attr(Identifier::bare(attr));
                    let redef = ReDef::new(attr, Block::new(vec![expr]));
                    attrs.push(Expr::ReDef(redef));
                }
            }
            // self::base = %x
            Some(_) => {
                let expr =
                    Expr::Accessor(Accessor::private_with_line(Str::from(&param_name), line));
                let obj = Expr::Accessor(Accessor::private_with_line(Str::ever("self"), line));
                let attr = obj.attr(Identifier::private_with_line(Str::ever("base"), line));
                let redef = ReDef::new(attr, Block::new(vec![expr]));
                attrs.push(Expr::ReDef(redef));
            }
            None => {}
        }
        let none = Token::new(TokenKind::NoneLit, "None", line, 0);
        attrs.push(Expr::Lit(Literal::new(ValueObj::None, none)));
        let block = Block::new(attrs);
        let body = DefBody::new(EQUAL, block, DefId(0));
        self.emit_subr_def(Some(class_name), subr_sig, body);
    }

    /// ```python
    /// class C:
    ///     # __new__ => __call__
    ///     def new(x): return C.__call__(x)
    /// ```
    fn emit_new_func(&mut self, sig: &Signature, __new__: Type) {
        log!(info "entered {}", fn_name!());
        let class_ident = sig.ident();
        let line = sig.ln_begin().unwrap_or(0);
        let mut ident = Identifier::public_with_line(DOT, Str::ever("new"), line);
        let class = Expr::Accessor(Accessor::Ident(class_ident.clone()));
        let mut new_ident = Identifier::private_with_line(Str::ever("__new__"), line);
        new_ident.vi.py_name = Some(Str::ever("__call__"));
        let class_new = class.attr_expr(new_ident);
        ident.vi.t = __new__;
        if let Some(new_first_param) = ident.vi.t.non_default_params().unwrap().first() {
            let param_name = new_first_param
                .name()
                .cloned()
                .unwrap_or_else(|| self.fresh_gen.fresh_varname());
            let param = VarName::from_str_and_line(param_name.clone(), line);
            let vi = VarInfo::nd_parameter(
                new_first_param.typ().clone(),
                ident.vi.def_loc.clone(),
                "?".into(),
            );
            let raw =
                erg_parser::ast::NonDefaultParamSignature::new(ParamPattern::VarName(param), None);
            let param = NonDefaultParamSignature::new(raw, vi, None);
            let params = Params::single(param);
            let bounds = TypeBoundSpecs::empty();
            let sig = SubrSignature::new(ident, bounds, params, sig.t_spec_with_op().cloned());
            let arg = PosArg::new(Expr::Accessor(Accessor::private_with_line(
                param_name, line,
            )));
            let call = class_new.call_expr(Args::single(arg));
            let block = Block::new(vec![call]);
            let body = DefBody::new(EQUAL, block, DefId(0));
            self.emit_subr_def(Some(class_ident.inspect()), sig, body);
        } else {
            let params = Params::empty();
            let bounds = TypeBoundSpecs::empty();
            let sig = SubrSignature::new(ident, bounds, params, sig.t_spec_with_op().cloned());
            let call = class_new.call_expr(Args::empty());
            let block = Block::new(vec![call]);
            let body = DefBody::new(EQUAL, block, DefId(0));
            self.emit_subr_def(Some(class_ident.inspect()), sig, body);
        }
    }

    fn emit_block(
        &mut self,
        block: Block,
        opt_name: Option<Str>,
        params: Vec<Str>,
        flags: u32,
    ) -> CodeObj {
        log!(info "entered {}", fn_name!());
        self.unit_size += 1;
        let name = if let Some(name) = opt_name {
            name
        } else {
            self.fresh_gen.fresh_varname()
        };
        let firstlineno = block
            .first()
            .and_then(|first| first.ln_begin())
            .unwrap_or(0);
        self.units.push(PyCodeGenUnit::new(
            self.unit_size,
            self.py_version,
            params,
            Str::rc(self.cfg.input.enclosed_name()),
            name,
            firstlineno,
            flags,
        ));
        let idx_copy_free_vars = if self.py_version.minor >= Some(11) {
            let idx_copy_free_vars = self.lasti();
            self.write(Opcode311::COPY_FREE_VARS, 0);
            self.write(Opcode311::RESUME, 0);
            idx_copy_free_vars
        } else {
            0
        };
        let init_stack_len = self.stack_len();
        for chunk in block.into_iter() {
            self.emit_chunk(chunk);
            // NOTE: 各行のトップレベルでは0個または1個のオブジェクトが残っている
            // Pythonの場合使わなかったオブジェクトはそのまま捨てられるが、Ergではdiscardを使う必要がある
            // TODO: discard
            if self.stack_len() > init_stack_len {
                self.emit_pop_top();
            }
        }
        self.cancel_if_pop_top(); // 最後の値は戻り値として取っておく
        if self.stack_len() == init_stack_len {
            self.emit_load_const(ValueObj::None);
        } else if self.stack_len() > init_stack_len + 1 {
            let block_id = self.cur_block().id;
            let stack_len = self.stack_len();
            CompileError::stack_bug(
                self.input().clone(),
                Location::Unknown,
                stack_len,
                block_id,
                fn_name_full!(),
            )
            .write_to_stderr();
            self.crash("error in emit_block: invalid stack size");
        }
        self.write(RETURN_VALUE, 0);
        // flagging
        if !self.cur_block_codeobj().varnames.is_empty() {
            self.mut_cur_block_codeobj().flags += CodeObjFlags::NewLocals as u32;
        }
        let freevars_len = self.cur_block_codeobj().freevars.len();
        if freevars_len > 0 {
            self.mut_cur_block_codeobj().flags += CodeObjFlags::Nested as u32;
            if self.py_version.minor >= Some(11) {
                self.edit_code(idx_copy_free_vars + 1, freevars_len);
            }
        } else if self.py_version.minor >= Some(11) {
            // cancel copying
            let code = self.cur_block_codeobj().code.get(idx_copy_free_vars);
            debug_assert_eq!(code, Some(&(Opcode311::COPY_FREE_VARS as u8)));
            self.edit_code(idx_copy_free_vars, CommonOpcode::NOP as usize);
        }
        // end of flagging
        let unit = self.units.pop().unwrap();
        // increase lineno
        if !self.units.is_empty() {
            let ld = unit
                .prev_lineno
                .saturating_sub(self.cur_block().prev_lineno);
            if ld != 0 {
                if let Some(l) = self.mut_cur_block_codeobj().lnotab.last_mut() {
                    *l += ld as u8;
                }
                self.mut_cur_block().prev_lineno += ld;
            }
        }
        unit.codeobj
    }

    fn load_prelude(&mut self) {
        // NOTE: Integers need to be used in IMPORT_NAME
        // but `Int` are called before importing it, so they need to be no_std mode
        let no_std = self.cfg.no_std;
        self.cfg.no_std = true;
        self.load_record_type();
        self.load_prelude_py();
        self.prelude_loaded = true;
        self.record_type_loaded = true;
        self.cfg.no_std = no_std;
    }

    fn load_in_op(&mut self) {
        let mod_name = Identifier::public("_erg_std_prelude");
        self.emit_global_import_items(
            mod_name,
            vec![(
                Identifier::public("in_operator"),
                Some(Identifier::private("#in_operator")),
            )],
        );
        self.in_op_loaded = true;
    }

    fn load_mutate_op(&mut self) {
        let mod_name = Identifier::public("_erg_std_prelude");
        self.emit_global_import_items(
            mod_name,
            vec![(
                Identifier::public("mutate_operator"),
                Some(Identifier::private("#mutate_operator")),
            )],
        );
        self.mutate_op_loaded = true;
    }

    fn load_control(&mut self) {
        let mod_name = Identifier::public("_erg_control");
        self.emit_import_all_instr(mod_name);
        self.control_loaded = true;
    }

    fn load_convertors(&mut self) {
        let mod_name = Identifier::public("_erg_convertors");
        self.emit_import_all_instr(mod_name);
        self.convertors_loaded = true;
    }

    fn load_prelude_py(&mut self) {
        self.emit_global_import_items(
            Identifier::public("sys"),
            vec![(
                Identifier::public("path"),
                Some(Identifier::private("#path")),
            )],
        );
        self.emit_load_name_instr(Identifier::private("#path"));
        self.emit_load_method_instr(Identifier::public("append"));
        self.emit_load_const(erg_std_path().to_str().unwrap());
        self.emit_call_instr(1, BoundAttr);
        self.stack_dec();
        self.emit_pop_top();
        let erg_std_mod = Identifier::public("_erg_std_prelude");
        // escaping
        self.emit_global_import_items(
            erg_std_mod.clone(),
            vec![(
                Identifier::public("in_operator"),
                Some(Identifier::private("#in_operator")),
            )],
        );
        self.emit_import_all_instr(erg_std_mod);
    }

    fn load_record_type(&mut self) {
        self.emit_global_import_items(
            Identifier::public("collections"),
            vec![(
                Identifier::public("namedtuple"),
                Some(Identifier::private("#NamedTuple")),
            )],
        );
    }

    fn load_abc(&mut self) {
        self.emit_global_import_items(
            Identifier::public("abc"),
            vec![
                (
                    Identifier::public("ABCMeta"),
                    Some(Identifier::private("#ABCMeta")),
                ),
                (
                    Identifier::public("abstractmethod"),
                    Some(Identifier::private("#abstractmethod")),
                ),
            ],
        );
    }

    fn load_module_type(&mut self) {
        self.emit_global_import_items(
            Identifier::public("types"),
            vec![(
                Identifier::public("ModuleType"),
                Some(Identifier::private("#ModuleType")),
            )],
        );
    }

    pub fn emit(&mut self, hir: HIR) -> CodeObj {
        log!(info "the code-generating process has started.{RESET}");
        self.unit_size += 1;
        self.units.push(PyCodeGenUnit::new(
            self.unit_size,
            self.py_version,
            vec![],
            Str::rc(self.cfg.input.enclosed_name()),
            "<module>",
            1,
            0,
        ));
        if self.py_version.minor >= Some(11) {
            self.write(Opcode311::RESUME, 0);
        }
        if !self.cfg.no_std && !self.prelude_loaded {
            self.load_prelude();
        }
        for chunk in hir.module.into_iter() {
            self.emit_chunk(chunk);
            // TODO: discard
            if self.stack_len() == 1 {
                self.emit_pop_top();
            }
        }
        self.cancel_if_pop_top(); // 最後の値は戻り値として取っておく
        if self.input().is_repl() {
            if self.stack_len() == 1 {
                self.emit_print_expr();
            }
            self.stack_dec_n(self.stack_len() as usize);
        }
        if self.stack_len() == 0 {
            self.emit_load_const(ValueObj::None);
        } else if self.stack_len() > 1 {
            let block_id = self.cur_block().id;
            let stack_len = self.stack_len();
            CompileError::stack_bug(
                self.input().clone(),
                Location::Unknown,
                stack_len,
                block_id,
                fn_name_full!(),
            )
            .write_to_stderr();
            self.crash("error in emit: invalid stack size");
        }
        self.write(RETURN_VALUE, 0);
        // flagging
        if !self.cur_block_codeobj().varnames.is_empty() {
            self.mut_cur_block_codeobj().flags += CodeObjFlags::NewLocals as u32;
        }
        // end of flagging
        let unit = self.units.pop().unwrap();
        if !self.units.is_empty() {
            let ld = unit.prev_lineno - self.cur_block().prev_lineno;
            if ld != 0 {
                if let Some(l) = self.mut_cur_block_codeobj().lnotab.last_mut() {
                    *l += ld as u8;
                }
                self.mut_cur_block().prev_lineno += ld;
            }
        }
        log!(info "the code-generating process has completed.{RESET}");
        unit.codeobj
    }
}
