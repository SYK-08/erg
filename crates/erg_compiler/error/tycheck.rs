use std::fmt::Display;

use erg_common::config::Input;
use erg_common::error::{ErrorCore, ErrorKind::*, Location, SubMessage};
use erg_common::set::Set;
use erg_common::style::{StyledStr, StyledString, StyledStrings, Stylize};
use erg_common::traits::Locational;
use erg_common::{fmt_iter, fmt_option_map, fmt_vec, switch_lang, Str};

use crate::error::*;
use crate::ty::{Predicate, Type};

pub type TyCheckError = CompileError;
pub type TyCheckErrors = CompileErrors;
pub type TyCheckResult<T> = CompileResult<T>;
pub type SingleTyCheckResult<T> = SingleCompileResult<T>;

impl TyCheckError {
    pub fn dummy(input: Input, errno: usize) -> Self {
        Self::new(ErrorCore::dummy(errno), input, "".to_string())
    }

    pub fn unreachable(input: Input, fn_name: &str, line: u32) -> Self {
        Self::new(ErrorCore::unreachable(fn_name, line), input, "".to_string())
    }

    pub fn checker_bug(
        input: Input,
        errno: usize,
        loc: Location,
        fn_name: &str,
        line: u32,
    ) -> Self {
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => format!("これはErg compilerのバグです、開発者に報告して下さい ({URL})\n\n{fn_name}:{line}より発生"),
                    "simplified_chinese" => format!("这是Erg编译器的错误，请报告给{URL}\n\n原因来自: {fn_name}:{line}"),
                    "traditional_chinese" => format!("這是Erg編譯器的錯誤，請報告給{URL}\n\n原因來自: {fn_name}:{line}"),
                    "english" => format!("this is a bug of the Erg compiler, please report it to {URL}\n\ncaused from: {fn_name}:{line}"),
                ),
                errno,
                CompilerSystemError,
                loc,
            ),
            input,
            "".to_string(),
        )
    }

    pub fn no_type_spec_error(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        name: &str,
    ) -> Self {
        let name = readable_name(name);
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => format!("{name}の型が指定されていません"),
                    "simplified_chinese" => format!("{name}的类型未指定"),
                    "traditional_chinese" => format!("{name}的類型未指定"),
                    "english" => format!("the type of {name} is not specified"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn callable_impl_error<'a, C: Locational + Display>(
        input: Input,
        errno: usize,
        callee: &C,
        param_ts: impl Iterator<Item = &'a Type>,
        caused_by: String,
    ) -> Self {
        let param_ts = fmt_iter(param_ts);
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(callee.loc())],
                switch_lang!(
                    "japanese" => format!(
                        "{callee}は{param_ts}を引数に取る呼び出し可能オブジェクトではありません"
                    ),
                    "simplified_chinese" => format!("{callee}不是以{param_ts}作为参数的可调用对象"),
                    "traditional_chinese" => format!("{callee}不是以{param_ts}作為參數的可調用對象"),
                    "english" => format!(
                        "{callee} is not a Callable object that takes {param_ts} as an argument"
                    ),
                ),
                errno,
                NotImplementedError,
                callee.loc(),
            ),
            input,
            caused_by,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn type_mismatch_error(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        name: &str,
        nth_param: Option<usize>,
        expect: &Type,
        found: &Type,
        candidates: Option<Set<Type>>,
        hint: Option<String>,
    ) -> Self {
        let ord = match nth_param {
            Some(pos) => switch_lang!(
                "japanese" => format!("({pos}番目の引数)"),
                "simplified_chinese" => format!("(第{pos}个参数)"),
                "traditional_chinese" => format!("(第{pos}個參數)"),
                "english" => format!(" (the {} argument)", ordinal_num(pos)),
            ),
            None => "".to_owned(),
        };
        let name = format!("{name}{ord}").with_color_and_attr(WARN, ATTR);
        let mut expct = StyledStrings::default();
        switch_lang!(
            "japanese" => expct.push_str("予期した型: "),
            "simplified_chinese" =>expct.push_str("预期: "),
            "traditional_chinese" => expct.push_str("預期: "),
            "english" => expct.push_str("expected: "),
        );
        expct.push_str_with_color_and_attr(format!("{expect}"), HINT, ATTR);

        let mut fnd = StyledStrings::default();
        switch_lang!(
            "japanese" => fnd.push_str("与えられた型: "),
            "simplified_chinese" => fnd.push_str("但找到: "),
            "traditional_chinese" => fnd.push_str("但找到: "),
            "english" =>fnd.push_str("but found: "),
        );
        fnd.push_str_with_color_and_attr(format!("{found}"), ERR, ATTR);
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(
                    loc,
                    vec![expct.to_string(), fnd.to_string()],
                    hint,
                )],
                switch_lang!(
                    "japanese" => format!("{name}の型が違います{}", fmt_option_map!(pre "\n与えられた型の単一化候補: ", candidates, |x: &Set<Type>| x.folded_display())),
                    "simplified_chinese" => format!("{name}的类型不匹配{}", fmt_option_map!(pre "\n某一类型的统一候选: ", candidates, |x: &Set<Type>| x.folded_display())),
                    "traditional_chinese" => format!("{name}的類型不匹配{}", fmt_option_map!(pre "\n某一類型的統一候選: ", candidates, |x: &Set<Type>| x.folded_display())),
                    "english" => format!("the type of {name} is mismatched{}", fmt_option_map!(pre "\nunification candidates of a given type: ", candidates, |x: &Set<Type>| x.folded_display())),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn return_type_error(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        name: &str,
        expect: &Type,
        found: &Type,
    ) -> Self {
        let mut expct = StyledStrings::default();
        switch_lang!(
            "japanese" => expct.push_str("予期した型: "),
            "simplified_chinese" =>expct.push_str("预期: "),
            "traditional_chinese" => expct.push_str("預期: "),
            "english" => expct.push_str("expected: "),
        );
        expct.push_str_with_color_and_attr(format!("{expect}"), HINT, ATTR);

        let mut fnd = StyledStrings::default();
        switch_lang!(
            "japanese" => fnd.push_str("与えられた型: "),
            "simplified_chinese" => fnd.push_str("但找到: "),
            "traditional_chinese" => fnd.push_str("但找到: "),
            "english" =>fnd.push_str("but found: "),
        );
        fnd.push_str_with_color_and_attr(format!("{found}"), ERR, ATTR);

        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(
                    loc,
                    vec![expct.to_string(), fnd.to_string()],
                    None,
                )],
                switch_lang!(
                    "japanese" => format!("{name}の戻り値の型が違います"),
                    "simplified_chinese" => format!("{name}的返回类型不匹配"),
                    "traditional_chinese" => format!("{name}的返回類型不匹配"),
                    "english" => format!("the return type of {name} is mismatched"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn uninitialized_error(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        name: &str,
        t: &Type,
    ) -> Self {
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => format!("{name}: {t}は宣言されましたが初期化されていません"),
                    "simplified_chinese" => format!("{name}: {t}已声明但未初始化"),
                    "traditional_chinese" => format!("{name}: {t}已宣告但未初始化"),
                    "english" => format!("{name}: {t} is declared but not initialized"),
                ),
                errno,
                NameError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn argument_error(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        expect: usize,
        found: usize,
    ) -> Self {
        let mut expct = StyledStrings::default();
        switch_lang!(
            "japanese" => expct.push_str("予期した個数: "),
            "simplified_chinese" =>expct.push_str("预期: "),
            "traditional_chinese" => expct.push_str("預期: "),
            "english" => expct.push_str("expected: "),
        );
        expct.push_str_with_color_and_attr(format!("{expect}"), HINT, ATTR);

        let mut fnd = StyledStrings::default();
        switch_lang!(
            "japanese" => fnd.push_str("与えられた個数: "),
            "simplified_chinese" => fnd.push_str("但找到: "),
            "traditional_chinese" => fnd.push_str("但找到: "),
            "english" =>fnd.push_str("but found: "),
        );
        fnd.push_str_with_color_and_attr(format!("{found}"), ERR, ATTR);

        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(
                    loc,
                    vec![expct.to_string(), fnd.to_string()],
                    None,
                )],
                switch_lang!(
                    "japanese" => format!("ポジショナル引数の数が違います"),
                    "simplified_chinese" => format!("正则参数的数量不匹配"),
                    "traditional_chinese" => format!("正則參數的數量不匹配"),
                    "english" => format!("the number of positional arguments is mismatched"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn param_error(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        expect: usize,
        found: usize,
    ) -> Self {
        let mut expct = StyledStrings::default();
        switch_lang!(
            "japanese" => expct.push_str("予期した個数: "),
            "simplified_chinese" =>expct.push_str("预期: "),
            "traditional_chinese" => expct.push_str("預期: "),
            "english" => expct.push_str("expected: "),
        );
        expct.push_str_with_color_and_attr(format!("{expect}"), HINT, ATTR);

        let mut fnd = StyledStrings::default();
        switch_lang!(
            "japanese" => fnd.push_str("与えられた個数: "),
            "simplified_chinese" => fnd.push_str("但找到: "),
            "traditional_chinese" => fnd.push_str("但找到: "),
            "english" =>fnd.push_str("but found: "),
        );
        fnd.push_str_with_color_and_attr(format!("{found}"), ERR, ATTR);

        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(
                    loc,
                    vec![expct.to_string(), fnd.to_string()],
                    None,
                )],
                switch_lang!(
                    "japanese" => format!("引数の数が違います"),
                    "simplified_chinese" => format!("参数的数量不匹配"),
                    "traditional_chinese" => format!("參數的數量不匹配"),
                    "english" => format!("the number of parameters is mismatched"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn default_param_error(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        name: &str,
    ) -> Self {
        Self::new(
            ErrorCore::new(
                vec![],
                switch_lang!(
                    "japanese" => format!("{name}はデフォルト引数を受け取りません"),
                    "simplified_chinese" => format!("{name}不接受默认参数"),
                    "traditional_chinese" => format!("{name}不接受預設參數"),
                    "english" => format!("{name} does not accept default parameters"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn default_param_not_found_error(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        param_name: &str,
        similar_name: Option<&str>,
    ) -> Self {
        let hint = match similar_name {
            Some(name) => {
                let mut s = StyledStrings::default();
                switch_lang!(
                    "japanese" => s.push_str("似た名前の引数があります: "),
                    "simplified_chinese" => s.push_str("相似的参数: "),
                    "traditional_chinese" => s.push_str("相似的參數: "),
                    "english" => s.push_str("exists a similar name parameter: "),
                );
                s.push_str_with_color_and_attr(name, HINT, ATTR);
                Some(s.to_string())
            }
            None => None,
        };
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(loc, vec![], hint)],
                switch_lang!(
                    "japanese" => format!("{param_name}という名前のデフォルト引数はありません"),
                    "simplified_chinese" => format!("没有名为{param_name}的默认参数"),
                    "traditional_chinese" => format!("沒有名為{param_name}的預設參數"),
                    "english" => format!("there is no default parameter named {param_name}"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn match_error(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        expr_t: &Type,
    ) -> Self {
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => format!("{expr_t}型の全パターンを網羅していません"),
                    "simplified_chinese" => format!("并非所有{expr_t}类型的模式都被涵盖"),
                    "traditional_chinese" => format!("並非所有{expr_t}類型的模式都被涵蓋"),
                    "english" => format!("not all patterns of type {expr_t} are covered"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn infer_error(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        expr: &str,
    ) -> Self {
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => format!("{expr}の型が推論できません"),
                    "simplified_chinese" => format!("无法推断{expr}的类型"),
                    "traditional_chinese" => format!("無法推斷{expr}的類型"),
                    "english" => format!("failed to infer the type of {expr}"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn dummy_infer_error(input: Input, fn_name: &str, line: u32) -> Self {
        Self::new(ErrorCore::unreachable(fn_name, line), input, "".to_owned())
    }

    pub fn not_relation(input: Input, fn_name: &str, line: u32) -> Self {
        Self::new(ErrorCore::unreachable(fn_name, line), input, "".to_owned())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn too_many_args_error(
        input: Input,
        errno: usize,
        loc: Location,
        callee_name: &str,
        caused_by: String,
        params_len: usize,
        pos_args_len: usize,
        kw_args_len: usize,
    ) -> Self {
        let name = readable_name(callee_name);
        let expect = format!("{params_len}").with_color_and_attr(HINT, ATTR);
        let pos_args_len = format!("{pos_args_len}").with_color_and_attr(ERR, ATTR);
        let kw_args_len = format!("{kw_args_len}").with_color_and_attr(ERR, ATTR);
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => format!(
                        "{name}に渡された引数の数が多すぎます

必要な引数の合計数: {expect}個
渡された引数の数:   {pos_args_len}個
キーワード引数の数: {kw_args_len}個"
                    ),
                    "simplified_chinese" => format!("传递给{name}的参数过多

总的预期参数: {expect}
通过的位置参数: {pos_args_len}
通过了关键字参数: {kw_args_len}"
                    ),
                    "traditional_chinese" => format!("傳遞給{name}的參數過多

所需參數總數: {expect}
遞的參數數量: {pos_args_len}
字參數的數量: {kw_args_len}"
                    ),
                    "english" => format!(
                        "too many arguments for {name}

total expected params:  {expect}
passed positional args: {pos_args_len}
passed keyword args:    {kw_args_len}"
                    ),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn args_missing_error(
        input: Input,
        errno: usize,
        loc: Location,
        callee_name: &str,
        caused_by: String,
        missing_params: Vec<Str>,
    ) -> Self {
        let name = StyledStr::new(readable_name(callee_name), Some(WARN), Some(ATTR));
        let vec_cxt = StyledString::new(fmt_vec(&missing_params), Some(WARN), Some(ATTR));
        let missing_len = missing_params.len();
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => format!("{name}に渡された引数が{missing_len}個足りません\n不足している引数: {vec_cxt}" ),
                    "simplified_chinese" => format!("{name}的{missing_len}个位置参数不被传递\n缺少的参数: {vec_cxt}" ),
                    "traditional_chinese" => format!("{name}的{missing_len}個位置參數不被傳遞\n缺少的參數: {vec_cxt}" ),
                    "english" => format!("missing {missing_len} positional argument(s) for {name}\nmissing: {vec_cxt}"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn multiple_args_error(
        input: Input,
        errno: usize,
        loc: Location,
        callee_name: &str,
        caused_by: String,
        arg_name: &str,
    ) -> Self {
        let name = StyledStr::new(readable_name(callee_name), Some(WARN), Some(ATTR));
        let found = StyledString::new(arg_name, Some(ERR), Some(ATTR));
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => format!("{name}の引数{found}が複数回渡されています"),
                    "simplified_chinese" => format!("{name}的参数{found}被多次传递"),
                    "traditional_chinese" => format!("{name}的參數{found}被多次傳遞"),
                    "english" => format!("{name}'s argument {found} is passed multiple times"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn unexpected_kw_arg_error(
        input: Input,
        errno: usize,
        loc: Location,
        callee_name: &str,
        caused_by: String,
        param_name: &str,
        similar_name: Option<&str>,
    ) -> Self {
        let name = StyledStr::new(readable_name(callee_name), Some(WARN), Some(ATTR));
        let found = param_name.with_color_and_attr(ERR, ATTR);
        let hint = match similar_name {
            Some(similar_name) => {
                let similar_name = similar_name.with_color_and_attr(HINT, ATTR);
                Some(switch_lang!(
                    "japanese" => format!("似た名前の引数があります: {similar_name}"),
                    "simplified_chinese" => format!("有相似的关参数: {similar_name}"),
                    "traditional_chinese" => format!("有相似的關參數: {similar_name}"),
                    "english" => format!("exists a similar name parameter: {similar_name}"),
                ))
            }
            None => None,
        };
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(loc, vec![], hint)],
                switch_lang!(
                    "japanese" => format!("{name}に予期しないキーワード引数{found}が渡されています"),
                    "simplified_chinese" => format!("{name}得到了意外的关键字参数{found}"),
                    "traditional_chinese" => format!("{name}得到了意外的關鍵字參數{found}"),
                    "english" => format!("{name} got unexpected keyword argument {found}"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn unification_error(
        input: Input,
        errno: usize,
        lhs_t: &Type,
        rhs_t: &Type,
        loc: Location,
        caused_by: String,
    ) -> Self {
        let mut lhs_typ = StyledStrings::default();
        switch_lang!(
            "japanese" => lhs_typ.push_str("左辺: "),
            "simplified_chinese" => lhs_typ.push_str("左边: "),
            "traditional_chinese" => lhs_typ.push_str("左邊: "),
            "english" => lhs_typ.push_str("lhs: "),
        );
        lhs_typ.push_str_with_color_and_attr(format!("{lhs_t}"), WARN, ATTR);
        let mut rhs_typ = StyledStrings::default();
        switch_lang!(
            "japanese" => rhs_typ.push_str("右辺: "),
            "simplified_chinese" => rhs_typ.push_str("右边: "),
            "traditional_chinese" => rhs_typ.push_str("右邊: "),
            "english" => rhs_typ.push_str("rhs: "),
        );
        rhs_typ.push_str_with_color_and_attr(format!("{rhs_t}"), WARN, ATTR);
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(
                    loc,
                    vec![lhs_typ.to_string(), rhs_typ.to_string()],
                    None,
                )],
                switch_lang!(
                    "japanese" => format!("型の単一化に失敗しました"),
                    "simplified_chinese" => format!("类型统一失败"),
                    "traditional_chinese" => format!("類型統一失敗"),
                    "english" => format!("unification failed"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn re_unification_error(
        input: Input,
        errno: usize,
        lhs_t: &Type,
        rhs_t: &Type,
        loc: Location,
        caused_by: String,
    ) -> Self {
        let mut lhs_typ = StyledStrings::default();
        switch_lang!(
            "japanese" => lhs_typ.push_str("左辺: "),
            "simplified_chinese" => lhs_typ.push_str("左边: "),
            "traditional_chinese" => lhs_typ.push_str("左邊: "),
            "english" => lhs_typ.push_str("lhs: "),
        );
        lhs_typ.push_str_with_color_and_attr(format!("{lhs_t}"), WARN, ATTR);
        let mut rhs_typ = StyledStrings::default();
        switch_lang!(
            "japanese" => rhs_typ.push_str("右辺: "),
            "simplified_chinese" => rhs_typ.push_str("右边: "),
            "traditional_chinese" => rhs_typ.push_str("右邊: "),
            "english" => rhs_typ.push_str("rhs: "),
        );
        rhs_typ.push_str_with_color_and_attr(format!("{rhs_t}"), WARN, ATTR);
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(
                    loc,
                    vec![lhs_typ.to_string(), rhs_typ.to_string()],
                    None,
                )],
                switch_lang!(
                    "japanese" => format!("型の再単一化に失敗しました"),
                    "simplified_chinese" => format!("重新统一类型失败"),
                    "traditional_chinese" => format!("重新統一類型失敗"),
                    "english" => format!("re-unification failed"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn subtyping_error(
        input: Input,
        errno: usize,
        sub_t: &Type,
        sup_t: &Type,
        loc: Location,
        caused_by: String,
    ) -> Self {
        let mut sub_type = StyledStrings::default();
        switch_lang!(
            "japanese" => sub_type.push_str("部分型: "),
            "simplified_chinese" => sub_type.push_str("子类型: "),
            "simplified_chinese" =>sub_type.push_str("子類型:"),
            "english" => sub_type.push_str("subtype: "),
        );
        sub_type.push_str_with_color_and_attr(format!("{sub_t}"), HINT, ATTR);

        let mut sup_type = StyledStrings::default();
        switch_lang!(
            "japanese" => sup_type.push_str("汎化型: "),
            "simplified_chinese" => sup_type.push_str("父类型: "),
            "simplified_chinese" => sup_type.push_str("父類型: "),
            "english" =>sup_type.push_str("supertype: "),
        );
        sup_type.push_str_with_color_and_attr(format!("{sup_t}"), ERR, ATTR);
        let hint = switch_lang!(
            "japanese" => "型推論が失敗している可能性があります。型を明示的に指定してみてください。",
            "simplified_chinese" => "可能是编译器推断失败。请尝试明确指定类型。",
            "traditional_chinese" => "可能是編譯器推斷失敗。請嘗試明確指定類型。",
            "english" => "The type checker may fail to inference types. Please try to explicitly specify the type.",
        );
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(
                    loc,
                    vec![sub_type.to_string(), sup_type.to_string()],
                    Some(hint.to_string()),
                )],
                switch_lang!(
                    "japanese" => format!("この式の部分型制約を満たせません"),
                    "simplified_chinese" => format!("无法满足此表达式中的子类型约束"),
                    "traditional_chinese" => format!("無法滿足此表達式中的子類型約束"),
                    "english" => format!("the subtype constraint in this expression cannot be satisfied"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn pred_unification_error(
        input: Input,
        errno: usize,
        lhs: &Predicate,
        rhs: &Predicate,
        caused_by: String,
    ) -> Self {
        let mut lhs_uni = StyledStrings::default();
        switch_lang!(
            "japanese" => lhs_uni.push_str("左辺: "),
            "simplified_chinese" => lhs_uni.push_str("左边: "),
            "traditional_chinese" => lhs_uni.push_str("左邊: "),
            "english" => lhs_uni.push_str("lhs: "),
        );
        lhs_uni.push_str_with_color_and_attr(format!("{lhs}"), HINT, ATTR);
        let mut rhs_uni = StyledStrings::default();
        switch_lang!(
            "japanese" => rhs_uni.push_str("右辺: "),
            "simplified_chinese" => rhs_uni.push_str("右边: "),
            "traditional_chinese" => rhs_uni.push_str("右邊: "),
            "english" => rhs_uni.push_str("rhs: "),
        );
        rhs_uni.push_str_with_color_and_attr(format!("{rhs}"), ERR, ATTR);
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(
                    Location::Unknown,
                    vec![lhs_uni.to_string(), rhs_uni.to_string()],
                    None,
                )],
                switch_lang!(
                    "japanese" => format!("述語式の単一化に失敗しました"),
                    "simplified_chinese" => format!("无法统一谓词表达式"),
                    "traditional_chinese" => format!("無法統一謂詞表達式"),
                    "english" => format!("predicate unification failed"),
                ),
                errno,
                TypeError,
                Location::Unknown,
            ),
            input,
            caused_by,
        )
    }

    pub fn no_candidate_error(
        input: Input,
        errno: usize,
        proj: &Type,
        loc: Location,
        caused_by: String,
        hint: Option<String>,
    ) -> Self {
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(loc, vec![], hint)],
                switch_lang!(
                    "japanese" => format!("{proj}の候補がありません"),
                    "simplified_chinese" => format!("{proj}没有候选项"),
                    "traditional_chinese" => format!("{proj}沒有候選項"),
                    "english" => format!("no candidate for {proj}"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn no_trait_impl_error(
        input: Input,
        errno: usize,
        class: &Type,
        trait_: &Type,
        loc: Location,
        caused_by: String,
        hint: Option<String>,
    ) -> Self {
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(loc, vec![], hint)],
                switch_lang!(
                    "japanese" => format!("{class}は{trait_}を実装していません"),
                    "simplified_chinese" => format!("{class}没有实现{trait_}"),
                    "traditional_chinese" => format!("{class}沒有實現{trait_}"),
                    "english" => format!("{class} does not implement {trait_}"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn method_definition_error(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        name: &str,
        hint: Option<String>,
    ) -> Self {
        let found = name.with_color_and_attr(ERR, ATTR);
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(loc, vec![], hint)],
                switch_lang!(
                    "japanese" => format!(
                        "{found}にメソッドを定義することはできません",
                    ),
                    "simplified_chinese" => format!(
                        "{found}不可定义方法",
                    ),
                    "traditional_chinese" => format!(
                        "{found}不可定義方法",
                    ),
                    "english" => format!(
                        "cannot define methods for {found}",
                    ),
                ),
                errno,
                MethodError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn specialization_error(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        member_name: &str,
        class_type: &Type,
        base_trait_type: &Type,
        expect: &Type,
        found: &Type,
    ) -> Self {
        let expct = StyledStrings::default();
        let expct = switch_lang!(
            "japanese" => expct.concat_str("期待される型: ").concat_str_with_color(&format!("{expect}"), WARN).concat_str("の部分型"),
            "simplified_chinese" => expct.concat_str("期望: ").concat_str_with_color(&format!("{expect}"), WARN).concat_str("的子类型"),
            "traditional_chinese" => expct.concat_str("期望: ").concat_str_with_color(&format!("{expect}"), WARN).concat_str("的子類型"),
            "english" => expct.concat_str("expected: subtype of ").concat_str_with_color(&format!("{expect}"), WARN),
        );
        let fnd = StyledStrings::default();
        let fnd = switch_lang!(
            "japanese" => fnd.concat_str("実際の型: ").concat_str_with_color(&format!("{found}"), ERR),
            "simplified_chinese" => fnd.concat_str("实际的类型: ").concat_str_with_color(&format!("{found}"), ERR),
            "traditional_chinese" => fnd.concat_str("實際的類型: ").concat_str_with_color(&format!("{found}"), ERR),
            "english" => fnd.concat_str("but found: ").concat_str_with_color(&format!("{found}"), ERR),
        );
        // TODO: specify base implementation location
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(
                    loc,
                    vec![expct.to_string(), fnd.to_string()],
                    None,
                )],
                switch_lang!(
                    "japanese" => format!("{class_type}には既に{base_trait_type}が実装されています。
                特殊化を行う場合は、{member_name}の型が{expect}の部分型である必要がありますが、{found}型となっています"),
                    "simplified_chinese" => format!("{class_type}已经实现了{base_trait_type}。
                特殊化时，{member_name}的类型必须是{expect}的子类型，但是现在是{found}类型"),
                    "traditional_chinese" => format!("{class_type}已經實現了{base_trait_type}。
                特殊化時，{member_name}的類型必須是{expect}的子類型，但是現在是{found}類型"),
                    "english" => format!("{class_type} already implements {base_trait_type}.
                To specialize, {member_name} must be a subtype of {expect}, but found {found}"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn trait_member_type_error(
        input: Input,
        errno: usize,
        loc: Location,
        caused_by: String,
        member_name: &str,
        trait_type: &Type,
        expect: &Type,
        found: &Type,
        hint: Option<String>,
    ) -> Self {
        let expct = StyledStrings::default();
        let mut expct = switch_lang!(
            "japanese" => {
                expct.concat(format!("{trait_type}").with_color_and_attr(ACCENT, ATTR)).concat_str("で宣言された型: ")
            },
            "simplified_chinese" => {
                expct.concat(format!("{trait_type}").with_color_and_attr(ACCENT, ATTR)).concat_str("中声明的类型: ")
            },
            "traditional_chinese" => {
                expct.concat(format!("{trait_type}").with_color_and_attr(ACCENT, ATTR)).concat_str("中聲明的類型: ")
            },
            "english" => {
                expct.concat_str("declared in ").concat(format!("{trait_type}: ").with_color(ACCENT))
            },
        );
        expct.push(format!("{expect}").with_color(HINT));
        let mut fnd = switch_lang!(
            "japanese" => StyledStrings::single("与えられた型: ".stylize()),
            "simplified_chinese" => StyledStrings::single("但找到: ".stylize()),
            "traditional_chinese" => StyledStrings::single("但找到: ".stylize()),
            "english" => StyledStrings::single("but found: ".stylize()),
        );
        fnd.push_str_with_color(format!("{found}"), ERR);
        let member_name = StyledStr::new(member_name, Some(WARN), Some(ATTR));
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(
                    loc,
                    vec![expct.to_string(), fnd.to_string()],
                    hint,
                )],
                switch_lang!(
                    "japanese" => format!("{member_name}の型が違います"),
                    "simplified_chinese" => format!("{member_name}的类型不匹配"),
                    "traditional_chinese" => format!("{member_name}的類型不匹配"),
                    "english" => format!("the type of {member_name} is mismatched"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn trait_member_not_defined_error(
        input: Input,
        errno: usize,
        caused_by: String,
        member_name: &str,
        trait_type: &Type,
        class_type: &Type,
        hint: Option<String>,
        loc: Location,
    ) -> Self {
        let member_name = member_name.with_color_and_attr(WARN, ATTR);
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(loc, vec![], hint)],
                switch_lang!(
                    "japanese" => format!("{trait_type}の{member_name}が{class_type}で実装されていません"),
                    "simplified_chinese" => format!("{trait_type}中的{member_name}没有在{class_type}中实现"),
                    "traditional_chinese" => format!("{trait_type}中的{member_name}沒有在{class_type}中實現"),
                    "english" => format!("{member_name} of {trait_type} is not implemented in {class_type}"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn not_in_trait_error(
        input: Input,
        errno: usize,
        caused_by: String,
        member_name: &str,
        trait_type: &Type,
        class_type: &Type,
        hint: Option<String>,
        loc: Location,
    ) -> Self {
        let member_name = member_name.with_color_and_attr(WARN, ATTR);
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(loc, vec![], hint)],
                switch_lang!(
                    "japanese" => format!("{class_type}の{member_name}は{trait_type}で宣言されていません"),
                    "simplified_chinese" => format!("{class_type}中的{member_name}没有在{trait_type}中声明"),
                    "traditional_chinese" => format!("{class_type}中的{member_name}沒有在{trait_type}中聲明"),
                    "english" => format!("{member_name} of {class_type} is not declared in {trait_type}"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn tyvar_not_defined_error(
        input: Input,
        errno: usize,
        name: &str,
        loc: Location,
        caused_by: String,
    ) -> Self {
        let found = name.with_color_and_attr(ERR, ATTR);
        Self::new(
            ErrorCore::new(
                vec![SubMessage::only_loc(loc)],
                switch_lang!(
                    "japanese" => format!("型変数{found}が定義されていません"),
                    "simplified_chinese" => format!("类型变量{found}没有定义"),
                    "traditional_chinese" => format!("類型變量{found}沒有定義"),
                    "english" => format!("type variable {found} is not defined"),
                ),
                errno,
                TypeError,
                loc,
            ),
            input,
            caused_by,
        )
    }

    pub fn ambiguous_type_error(
        input: Input,
        errno: usize,
        expr: &(impl Locational + Display),
        candidates: &[Type],
        caused_by: String,
    ) -> Self {
        let hint = Some(
            switch_lang!(
            "japanese" => {
                let mut s = StyledStrings::default();
                s.push_str("多相関数の場合は");
                s.push_str_with_color_and_attr("f|T := Int|", ACCENT, ATTR);
                s.push_str(", \n型属性の場合は");
                s.push_str_with_color_and_attr("f|T := Trait|.X", ACCENT, ATTR);
                s
            },
            "simplified_chinese" => {
                let mut s = StyledStrings::default();
                s.push_str("如果是多态函数，请使用");
                s.push_str_with_color_and_attr("f|T := Int|", ACCENT, ATTR);
                s.push_str("，\n如果是类型属性，请使用");
                s.push_str_with_color_and_attr("f|T := Trait|.X", ACCENT, ATTR);
                s
            },
            "traditional_chinese" => {
                let mut s = StyledStrings::default();
                s.push_str("如果是多型函數，請使用");
                s.push_str_with_color_and_attr("f|T := Int|", ACCENT, ATTR);
                s.push_str("，\n如果是類型屬性，請使用");
                s.push_str_with_color_and_attr("f|T := Trait|.X", ACCENT, ATTR);
                s
            },
            "english" => {
                let mut s = StyledStrings::default();
                s.push_str("if it is a polymorphic function, like ");
                s.push_str_with_color_and_attr("f|T := Int|", ACCENT, ATTR);
                s.push_str("\nif it is a type attribute, like ");
                s.push_str_with_color_and_attr("f|T := Trait|.X ", ACCENT, ATTR);
                s
            },
                    )
            .to_string(),
        );
        let sub_msg = switch_lang!(
            "japanese" => "型を指定してください",
            "simplified_chinese" => "请指定类型",
            "traditional_chinese" => "請指定類型",
            "english" => "specify the type",
        );
        let mut candidate = StyledStrings::default();
        switch_lang!(
            "japanese" => candidate.push_str("候補: "),
            "simplified_chinese" => candidate.push_str("候选: "),
            "traditional_chinese" => candidate.push_str("候選: "),
            "english" => candidate.push_str("candidates: "),
        );
        candidate.push_str_with_color_and_attr(&fmt_vec(candidates), WARN, ATTR);
        Self::new(
            ErrorCore::new(
                vec![SubMessage::ambiguous_new(
                    expr.loc(),
                    vec![sub_msg.to_string(), candidate.to_string()],
                    hint,
                )],
                switch_lang!(
                    "japanese" => format!("{expr}の型を一意に決定できませんでした"),
                    "simplified_chinese" => format!("无法确定{expr}的类型"),
                    "traditional_chinese" => format!("無法確定{expr}的類型"),
                    "english" => format!("cannot determine the type of {expr}"),
                ),
                errno,
                TypeError,
                expr.loc(),
            ),
            input,
            caused_by,
        )
    }
}
