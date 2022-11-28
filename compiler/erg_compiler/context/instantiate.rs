use std::fmt;
use std::mem;
use std::option::Option; // conflicting to Type::Option

use erg_common::dict;
use erg_common::dict::Dict;
use erg_common::error::Location;
#[allow(unused)]
use erg_common::log;
use erg_common::traits::{Locational, Stream};
use erg_common::Str;
use erg_common::{assume_unreachable, enum_unwrap, set, try_map_mut};

use ast::{
    NonDefaultParamSignature, ParamTySpec, PreDeclTypeSpec, SimpleTypeSpec, TypeBoundSpec,
    TypeBoundSpecs, TypeSpec,
};
use erg_parser::ast;
use erg_parser::token::TokenKind;
use erg_parser::Parser;

use crate::ty::constructors::*;
use crate::ty::free::CanbeFree;
use crate::ty::free::{Constraint, HasLevel};
use crate::ty::typaram::{IntervalOp, TyParam, TyParamOrdering};
use crate::ty::value::ValueObj;
use crate::ty::{HasType, ParamTy, Predicate, SubrKind, Type};
use TyParamOrdering::*;
use Type::*;

use crate::context::{Context, RegistrationMode};
use crate::error::{TyCheckError, TyCheckErrors, TyCheckResult};
use crate::hir;
use crate::AccessKind;
use RegistrationMode::*;

/// Context for instantiating a quantified type
/// 量化型をインスタンス化するための文脈
/// e.g. Array -> [("T": ?T(: Type)), ("N": ?N(: Nat))]
/// FIXME: current implementation is wrong
/// It will not work unless the type variable is used with the same name as the definition.
#[derive(Debug, Clone)]
pub struct TyVarCache {
    _level: usize,
    pub(crate) tyvar_instances: Dict<Str, Type>,
    pub(crate) typaram_instances: Dict<Str, TyParam>,
}

impl fmt::Display for TyVarCache {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "TyVarInstContext {{ tyvar_instances: {}, typaram_instances: {} }}",
            self.tyvar_instances, self.typaram_instances,
        )
    }
}

impl TyVarCache {
    pub fn new(level: usize, _ctx: &Context) -> Self {
        Self {
            _level: level,
            tyvar_instances: Dict::new(),
            typaram_instances: Dict::new(),
        }
    }

    pub fn merge(&mut self, outer: &Self) {
        for (name, ty) in outer.tyvar_instances.iter() {
            if self.tyvar_instances.contains_key(name) {
                continue;
            } else {
                self.tyvar_instances.insert(name.clone(), ty.clone());
            }
        }
        for (name, ty) in outer.typaram_instances.iter() {
            if self.typaram_instances.contains_key(name) {
                continue;
            } else {
                self.typaram_instances.insert(name.clone(), ty.clone());
            }
        }
    }

    fn instantiate_constraint(
        &mut self,
        constr: Constraint,
        ctx: &Context,
        loc: Location,
    ) -> TyCheckResult<Constraint> {
        match constr {
            Constraint::Sandwiched { sub, sup } => Ok(Constraint::new_sandwiched(
                ctx.instantiate_t_inner(sub, self, loc)?,
                ctx.instantiate_t_inner(sup, self, loc)?,
            )),
            Constraint::TypeOf(t) => Ok(Constraint::new_type_of(
                ctx.instantiate_t_inner(t, self, loc)?,
            )),
            Constraint::Uninited => Ok(Constraint::Uninited),
        }
    }

    fn _instantiate_pred(&self, _pred: Predicate) -> Predicate {
        todo!()
    }

    pub(crate) fn push_or_init_tyvar(&mut self, name: &Str, tv: &Type) {
        if let Some(inst) = self.tyvar_instances.get(name) {
            // T<tv> <: Eq(T<inst>)
            // T<inst> is uninitialized
            // T<inst>.link(T<tv>);
            // T <: Eq(T <: Eq(T <: ...))
            let inst = enum_unwrap!(inst, Type::FreeVar);
            inst.link(tv);
        } else if let Some(inst) = self.typaram_instances.get(name) {
            if let TyParam::Type(inst) = inst {
                let fv_inst = enum_unwrap!(inst.as_ref(), Type::FreeVar);
                fv_inst.link(tv);
            } else if let TyParam::FreeVar(fv) = inst {
                fv.link(&TyParam::t(tv.clone()));
            } else {
                unreachable!()
            }
        }
        self.tyvar_instances.insert(name.clone(), tv.clone());
    }

    pub(crate) fn push_or_init_typaram(&mut self, name: &Str, tp: &TyParam) {
        // FIXME:
        if let Some(_tp) = self.typaram_instances.get(name) {
            panic!("{_tp} {tp}");
            // return;
        }
        if let Some(_t) = self.tyvar_instances.get(name) {
            panic!("{_t} {tp}");
            // return;
        }
        self.typaram_instances.insert(name.clone(), tp.clone());
    }

    pub(crate) fn get_tyvar(&self, name: &str) -> Option<&Type> {
        self.tyvar_instances.get(name).or_else(|| {
            self.typaram_instances
                .get(name)
                .map(|tp| <&Type>::try_from(tp).unwrap())
        })
    }

    pub(crate) fn get_typaram(&self, name: &str) -> Option<&TyParam> {
        self.typaram_instances.get(name)
    }
}

/// TODO: this struct will be removed when const functions are implemented.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConstTemplate {
    Obj(ValueObj),
    App {
        name: Str,
        non_default_args: Vec<Type>,
        default_args: Vec<ConstTemplate>,
    },
}

impl ConstTemplate {
    pub const fn app(
        name: &'static str,
        non_default_args: Vec<Type>,
        default_args: Vec<ConstTemplate>,
    ) -> Self {
        ConstTemplate::App {
            name: Str::ever(name),
            non_default_args,
            default_args,
        }
    }
}

impl Context {
    pub(crate) fn instantiate_var_sig_t(
        &self,
        t_spec: Option<&TypeSpec>,
        opt_eval_t: Option<Type>,
        mode: RegistrationMode,
    ) -> TyCheckResult<Type> {
        let mut tmp_tv_cache = TyVarCache::new(self.level, self);
        let spec_t = if let Some(s) = t_spec {
            self.instantiate_typespec(s, None, &mut tmp_tv_cache, mode, false)?
        } else {
            free_var(self.level, Constraint::new_type_of(Type))
        };
        if let Some(eval_t) = opt_eval_t {
            self.sub_unify(
                &eval_t,
                &spec_t,
                t_spec.map(|s| s.loc()).unwrap_or(Location::Unknown),
                None,
            )?;
        }
        Ok(spec_t)
    }

    pub(crate) fn instantiate_sub_sig_t(
        &self,
        sig: &ast::SubrSignature,
        default_ts: Vec<Type>,
        mode: RegistrationMode,
    ) -> Result<Type, (TyCheckErrors, Type)> {
        let mut errs = TyCheckErrors::empty();
        // -> Result<Type, (Type, TyCheckErrors)> {
        let opt_decl_sig_t = self
            .rec_get_decl_info(&sig.ident, AccessKind::Name, &self.cfg.input, &self.name)
            .ok()
            .map(|vi| enum_unwrap!(vi.t, Type::Subr));
        let mut tmp_tv_cache = self
            .instantiate_ty_bounds(&sig.bounds, PreRegister)
            .map_err(|errs| (errs, Type::Failure))?;
        let mut non_defaults = vec![];
        for (n, param) in sig.params.non_defaults.iter().enumerate() {
            let opt_decl_t = opt_decl_sig_t
                .as_ref()
                .and_then(|subr| subr.non_default_params.get(n));
            match self.instantiate_param_ty(param, None, opt_decl_t, &mut tmp_tv_cache, mode) {
                Ok(t) => non_defaults.push(t),
                Err(es) => {
                    errs.extend(es);
                    non_defaults.push(ParamTy::pos(param.inspect().cloned(), Type::Failure));
                }
            }
        }
        let var_args = if let Some(var_args) = sig.params.var_args.as_ref() {
            let opt_decl_t = opt_decl_sig_t
                .as_ref()
                .and_then(|subr| subr.var_params.as_ref().map(|v| v.as_ref()));
            let pt = match self.instantiate_param_ty(
                var_args,
                None,
                opt_decl_t,
                &mut tmp_tv_cache,
                mode,
            ) {
                Ok(pt) => pt,
                Err(es) => {
                    errs.extend(es);
                    ParamTy::pos(var_args.inspect().cloned(), Type::Failure)
                }
            };
            Some(pt)
        } else {
            None
        };
        let mut defaults = vec![];
        for ((n, p), default_t) in sig.params.defaults.iter().enumerate().zip(default_ts) {
            let opt_decl_t = opt_decl_sig_t
                .as_ref()
                .and_then(|subr| subr.default_params.get(n));
            match self.instantiate_param_ty(
                &p.sig,
                Some(default_t),
                opt_decl_t,
                &mut tmp_tv_cache,
                mode,
            ) {
                Ok(t) => defaults.push(t),
                Err(es) => {
                    errs.extend(es);
                    defaults.push(ParamTy::pos(p.sig.inspect().cloned(), Type::Failure));
                }
            }
        }
        let spec_return_t = if let Some(s) = sig.return_t_spec.as_ref() {
            let opt_decl_t = opt_decl_sig_t
                .as_ref()
                .map(|subr| ParamTy::anonymous(subr.return_t.as_ref().clone()));
            match self.instantiate_typespec(s, opt_decl_t.as_ref(), &mut tmp_tv_cache, mode, false)
            {
                Ok(ty) => ty,
                Err(es) => {
                    errs.extend(es);
                    Type::Failure
                }
            }
        } else {
            // preregisterならouter scopeで型宣言(see inference.md)
            let level = if mode == PreRegister {
                self.level
            } else {
                self.level + 1
            };
            free_var(level, Constraint::new_type_of(Type))
        };
        let typ = if sig.ident.is_procedural() {
            proc(non_defaults, var_args, defaults, spec_return_t)
        } else {
            func(non_defaults, var_args, defaults, spec_return_t)
        };
        if errs.is_empty() {
            Ok(typ)
        } else {
            Err((errs, typ))
        }
    }

    /// spec_t == Noneかつリテラル推論が不可能なら型変数を発行する
    pub(crate) fn instantiate_param_sig_t(
        &self,
        sig: &NonDefaultParamSignature,
        opt_decl_t: Option<&ParamTy>,
        tmp_tv_cache: &mut TyVarCache,
        mode: RegistrationMode,
    ) -> TyCheckResult<Type> {
        let spec_t = if let Some(spec_with_op) = &sig.t_spec {
            self.instantiate_typespec(&spec_with_op.t_spec, opt_decl_t, tmp_tv_cache, mode, false)?
        } else {
            match &sig.pat {
                ast::ParamPattern::Lit(lit) => v_enum(set![self.eval_lit(lit)?]),
                ast::ParamPattern::Discard(_) => Type::Obj,
                // ast::ParamPattern::VarName(name) if &name.inspect()[..] == "_" => Type::Obj,
                // TODO: Array<Lit>
                _ => {
                    let level = if mode == PreRegister {
                        self.level
                    } else {
                        self.level + 1
                    };
                    free_var(level, Constraint::new_type_of(Type))
                }
            }
        };
        if let Some(decl_pt) = opt_decl_t {
            self.sub_unify(
                decl_pt.typ(),
                &spec_t,
                sig.t_spec
                    .as_ref()
                    .map(|s| s.loc())
                    .unwrap_or_else(|| sig.loc()),
                None,
            )?;
        }
        Ok(spec_t)
    }

    pub(crate) fn instantiate_param_ty(
        &self,
        sig: &NonDefaultParamSignature,
        opt_default_t: Option<Type>,
        opt_decl_t: Option<&ParamTy>,
        tmp_tv_cache: &mut TyVarCache,
        mode: RegistrationMode,
    ) -> TyCheckResult<ParamTy> {
        let t = self.instantiate_param_sig_t(sig, opt_decl_t, tmp_tv_cache, mode)?;
        match (sig.inspect(), opt_default_t) {
            (Some(name), Some(default_t)) => Ok(ParamTy::kw_default(name.clone(), t, default_t)),
            (Some(name), None) => Ok(ParamTy::kw(name.clone(), t)),
            (None, None) => Ok(ParamTy::anonymous(t)),
            _ => unreachable!(),
        }
    }

    pub(crate) fn instantiate_predecl_t(
        &self,
        predecl: &PreDeclTypeSpec,
        opt_decl_t: Option<&ParamTy>,
        tmp_tv_cache: &mut TyVarCache,
        not_found_is_qvar: bool,
    ) -> TyCheckResult<Type> {
        match predecl {
            ast::PreDeclTypeSpec::Simple(simple) => {
                self.instantiate_simple_t(simple, opt_decl_t, tmp_tv_cache, not_found_is_qvar)
            }
            ast::PreDeclTypeSpec::Attr { namespace, t } => {
                if let Ok(namespace) = Parser::validate_const_expr(namespace.as_ref().clone()) {
                    if let Ok(namespace) =
                        self.instantiate_const_expr_as_type(&namespace, None, tmp_tv_cache)
                    {
                        let rhs = t.ident.inspect();
                        return Ok(proj(namespace, rhs));
                    }
                }
                let ctx = self.get_singular_ctx(namespace.as_ref(), &self.name)?;
                if let Some((typ, _)) = ctx.rec_get_type(t.ident.inspect()) {
                    // TODO: visibility check
                    Ok(typ.clone())
                } else {
                    Err(TyCheckErrors::from(TyCheckError::no_var_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        t.loc(),
                        self.caused_by(),
                        t.ident.inspect(),
                        self.get_similar_name(t.ident.inspect()),
                    )))
                }
            }
            other => todo!("{other}"),
        }
    }

    pub(crate) fn instantiate_simple_t(
        &self,
        simple: &SimpleTypeSpec,
        opt_decl_t: Option<&ParamTy>,
        tmp_tv_cache: &mut TyVarCache,
        not_found_is_qvar: bool,
    ) -> TyCheckResult<Type> {
        match &simple.ident.inspect()[..] {
            "_" | "Obj" => Ok(Type::Obj),
            "Nat" => Ok(Type::Nat),
            "Int" => Ok(Type::Int),
            "Ratio" => Ok(Type::Ratio),
            "Float" => Ok(Type::Float),
            "Str" => Ok(Type::Str),
            "Bool" => Ok(Type::Bool),
            "NoneType" => Ok(Type::NoneType),
            "Ellipsis" => Ok(Type::Ellipsis),
            "NotImplemented" => Ok(Type::NotImplemented),
            "Inf" => Ok(Type::Inf),
            "NegInf" => Ok(Type::NegInf),
            "Never" => Ok(Type::Never),
            "ClassType" => Ok(Type::ClassType),
            "TraitType" => Ok(Type::TraitType),
            "Type" => Ok(Type::Type),
            "Array" => {
                // TODO: kw
                let mut args = simple.args.pos_args();
                if let Some(first) = args.next() {
                    let t = self.instantiate_const_expr_as_type(&first.expr, None, tmp_tv_cache)?;
                    let len = args.next().unwrap();
                    let len = self.instantiate_const_expr(&len.expr, None, tmp_tv_cache)?;
                    Ok(array_t(t, len))
                } else {
                    Ok(mono("GenericArray"))
                }
            }
            "Ref" => {
                let mut args = simple.args.pos_args();
                let first = args.next().unwrap();
                let t = self.instantiate_const_expr_as_type(&first.expr, None, tmp_tv_cache)?;
                Ok(ref_(t))
            }
            "RefMut" => {
                // TODO after
                let mut args = simple.args.pos_args();
                let first = args.next().unwrap();
                let t = self.instantiate_const_expr_as_type(&first.expr, None, tmp_tv_cache)?;
                Ok(ref_mut(t, None))
            }
            "Self" => self.rec_get_self_t().ok_or_else(|| {
                TyCheckErrors::from(TyCheckError::unreachable(
                    self.cfg.input.clone(),
                    erg_common::fn_name_full!(),
                    line!(),
                ))
            }),
            other if simple.args.is_empty() => {
                if let Some(t) = tmp_tv_cache.get_tyvar(other) {
                    return Ok(t.clone());
                } else if let Some(tp) = tmp_tv_cache.get_typaram(other) {
                    let t = enum_unwrap!(tp, TyParam::Type);
                    return Ok(t.as_ref().clone());
                }
                if let Some(tv_cache) = &self.tv_cache {
                    if let Some(t) = tv_cache.get_tyvar(other) {
                        return Ok(t.clone());
                    } else if let Some(tp) = tv_cache.get_typaram(other) {
                        let t = enum_unwrap!(tp, TyParam::Type);
                        return Ok(t.as_ref().clone());
                    }
                }
                if let Some(outer) = &self.outer {
                    if let Ok(t) = outer.instantiate_simple_t(
                        simple,
                        opt_decl_t,
                        tmp_tv_cache,
                        not_found_is_qvar,
                    ) {
                        return Ok(t);
                    }
                }
                if let Some(decl_t) = opt_decl_t {
                    return Ok(decl_t.typ().clone());
                }
                if let Some((typ, _)) = self.rec_get_type(other) {
                    Ok(typ.clone())
                } else if not_found_is_qvar {
                    let tyvar = named_free_var(Str::rc(other), self.level, Constraint::Uninited);
                    tmp_tv_cache.push_or_init_tyvar(&Str::rc(other), &tyvar);
                    Ok(tyvar)
                } else {
                    Err(TyCheckErrors::from(TyCheckError::no_type_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        simple.loc(),
                        self.caused_by(),
                        other,
                        self.get_similar_name(other),
                    )))
                }
            }
            other => {
                let ctx = if let Some((_, ctx)) = self.rec_get_type(other) {
                    ctx
                } else {
                    return Err(TyCheckErrors::from(TyCheckError::no_type_error(
                        self.cfg.input.clone(),
                        line!() as usize,
                        simple.ident.loc(),
                        self.caused_by(),
                        other,
                        self.get_similar_name(other),
                    )));
                };
                // FIXME: kw args
                let mut new_params = vec![];
                for (i, arg) in simple.args.pos_args().enumerate() {
                    let params =
                        self.instantiate_const_expr(&arg.expr, Some((ctx, i)), tmp_tv_cache);
                    let params = params.or_else(|e| {
                        if not_found_is_qvar {
                            let name = Str::from(arg.expr.to_string());
                            let tp = TyParam::named_free_var(
                                name.clone(),
                                self.level,
                                Constraint::Uninited,
                            );
                            tmp_tv_cache.push_or_init_typaram(&name, &tp);
                            Ok(tp)
                        } else {
                            Err(e)
                        }
                    })?;
                    new_params.push(params);
                }
                // FIXME: non-builtin
                Ok(poly(Str::rc(other), new_params))
            }
        }
    }

    pub(crate) fn instantiate_const_expr(
        &self,
        expr: &ast::ConstExpr,
        erased_idx: Option<(&Context, usize)>,
        tmp_tv_cache: &mut TyVarCache,
    ) -> TyCheckResult<TyParam> {
        match expr {
            ast::ConstExpr::Lit(lit) => Ok(TyParam::Value(self.eval_lit(lit)?)),
            ast::ConstExpr::Accessor(ast::ConstAccessor::Local(name)) => {
                if &name.inspect()[..] == "_" {
                    let t = if let Some((ctx, i)) = erased_idx {
                        ctx.params[i].1.t.clone()
                    } else {
                        Type::Uninited
                    };
                    return Ok(TyParam::erased(t));
                }
                if let Some(tp) = tmp_tv_cache.get_typaram(name.inspect()) {
                    return Ok(tp.clone());
                } else if let Some(t) = tmp_tv_cache.get_tyvar(name.inspect()) {
                    return Ok(TyParam::t(t.clone()));
                }
                if let Some(tv_ctx) = &self.tv_cache {
                    if let Some(t) = tv_ctx.get_tyvar(name.inspect()) {
                        return Ok(TyParam::t(t.clone()));
                    } else if let Some(tp) = tv_ctx.get_typaram(name.inspect()) {
                        return Ok(tp.clone());
                    }
                }
                if let Some(value) = self.rec_get_const_obj(name.inspect()) {
                    return Ok(TyParam::Value(value.clone()));
                }
                Err(TyCheckErrors::from(TyCheckError::no_var_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    name.loc(),
                    self.caused_by(),
                    name.inspect(),
                    self.get_similar_name(name.inspect()),
                )))
            }
            _ => todo!(),
        }
    }

    pub(crate) fn instantiate_const_expr_as_type(
        &self,
        expr: &ast::ConstExpr,
        erased_idx: Option<(&Context, usize)>,
        tmp_tv_cache: &mut TyVarCache,
    ) -> TyCheckResult<Type> {
        match self.instantiate_const_expr(expr, erased_idx, tmp_tv_cache)? {
            TyParam::Type(t) => Ok(*t),
            TyParam::Value(ValueObj::Type(t)) => Ok(t.into_typ()),
            other => todo!("{other}"),
        }
    }

    fn instantiate_func_param_spec(
        &self,
        p: &ParamTySpec,
        opt_decl_t: Option<&ParamTy>,
        default_t: Option<&TypeSpec>,
        tmp_tv_cache: &mut TyVarCache,
        mode: RegistrationMode,
    ) -> TyCheckResult<ParamTy> {
        let t = self.instantiate_typespec(&p.ty, opt_decl_t, tmp_tv_cache, mode, false)?;
        if let Some(default_t) = default_t {
            Ok(ParamTy::kw_default(
                p.name.as_ref().unwrap().inspect().to_owned(),
                t,
                self.instantiate_typespec(default_t, opt_decl_t, tmp_tv_cache, mode, false)?,
            ))
        } else {
            Ok(ParamTy::pos(
                p.name.as_ref().map(|t| t.inspect().to_owned()),
                t,
            ))
        }
    }

    pub(crate) fn instantiate_typespec(
        &self,
        spec: &TypeSpec,
        opt_decl_t: Option<&ParamTy>,
        tmp_tv_cache: &mut TyVarCache,
        mode: RegistrationMode,
        not_found_is_qvar: bool,
    ) -> TyCheckResult<Type> {
        match spec {
            TypeSpec::Infer(_) => Ok(free_var(self.level, Constraint::new_type_of(Type))),
            TypeSpec::PreDeclTy(predecl) => Ok(self.instantiate_predecl_t(
                predecl,
                opt_decl_t,
                tmp_tv_cache,
                not_found_is_qvar,
            )?),
            TypeSpec::And(lhs, rhs) => Ok(self.intersection(
                &self.instantiate_typespec(
                    lhs,
                    opt_decl_t,
                    tmp_tv_cache,
                    mode,
                    not_found_is_qvar,
                )?,
                &self.instantiate_typespec(
                    rhs,
                    opt_decl_t,
                    tmp_tv_cache,
                    mode,
                    not_found_is_qvar,
                )?,
            )),
            TypeSpec::Or(lhs, rhs) => Ok(self.union(
                &self.instantiate_typespec(
                    lhs,
                    opt_decl_t,
                    tmp_tv_cache,
                    mode,
                    not_found_is_qvar,
                )?,
                &self.instantiate_typespec(
                    rhs,
                    opt_decl_t,
                    tmp_tv_cache,
                    mode,
                    not_found_is_qvar,
                )?,
            )),
            TypeSpec::Not(lhs, rhs) => Ok(not(
                self.instantiate_typespec(lhs, opt_decl_t, tmp_tv_cache, mode, not_found_is_qvar)?,
                self.instantiate_typespec(rhs, opt_decl_t, tmp_tv_cache, mode, not_found_is_qvar)?,
            )),
            TypeSpec::Array(arr) => {
                let elem_t = self.instantiate_typespec(
                    &arr.ty,
                    opt_decl_t,
                    tmp_tv_cache,
                    mode,
                    not_found_is_qvar,
                )?;
                let mut len = self.instantiate_const_expr(&arr.len, None, tmp_tv_cache)?;
                if let TyParam::Erased(t) = &mut len {
                    *t.as_mut() = Type::Nat;
                }
                Ok(array_t(elem_t, len))
            }
            TypeSpec::SetWithLen(set) => {
                let elem_t = self.instantiate_typespec(
                    &set.ty,
                    opt_decl_t,
                    tmp_tv_cache,
                    mode,
                    not_found_is_qvar,
                )?;
                let mut len = self.instantiate_const_expr(&set.len, None, tmp_tv_cache)?;
                if let TyParam::Erased(t) = &mut len {
                    *t.as_mut() = Type::Nat;
                }
                Ok(set_t(elem_t, len))
            }
            TypeSpec::Tuple(tys) => {
                let mut inst_tys = vec![];
                for spec in tys {
                    inst_tys.push(self.instantiate_typespec(
                        spec,
                        opt_decl_t,
                        tmp_tv_cache,
                        mode,
                        not_found_is_qvar,
                    )?);
                }
                Ok(tuple_t(inst_tys))
            }
            TypeSpec::Dict(dict) => {
                let mut inst_tys = dict! {};
                for (k, v) in dict {
                    inst_tys.insert(
                        self.instantiate_typespec(
                            k,
                            opt_decl_t,
                            tmp_tv_cache,
                            mode,
                            not_found_is_qvar,
                        )?,
                        self.instantiate_typespec(
                            v,
                            opt_decl_t,
                            tmp_tv_cache,
                            mode,
                            not_found_is_qvar,
                        )?,
                    );
                }
                Ok(dict_t(inst_tys.into()))
            }
            TypeSpec::Record(rec) => {
                let mut inst_tys = dict! {};
                for (k, v) in rec {
                    inst_tys.insert(
                        k.into(),
                        self.instantiate_typespec(
                            v,
                            opt_decl_t,
                            tmp_tv_cache,
                            mode,
                            not_found_is_qvar,
                        )?,
                    );
                }
                Ok(Type::Record(inst_tys))
            }
            // TODO: エラー処理(リテラルでない)はパーサーにやらせる
            TypeSpec::Enum(set) => {
                let mut new_set = set! {};
                for arg in set.pos_args() {
                    new_set.insert(self.instantiate_const_expr(&arg.expr, None, tmp_tv_cache)?);
                }
                let ty = new_set.iter().fold(Type::Never, |t, tp| {
                    self.union(&t, &self.get_tp_t(tp).unwrap())
                });
                Ok(tp_enum(ty, new_set))
            }
            TypeSpec::Interval { op, lhs, rhs } => {
                let op = match op.kind {
                    TokenKind::Closed => IntervalOp::Closed,
                    TokenKind::LeftOpen => IntervalOp::LeftOpen,
                    TokenKind::RightOpen => IntervalOp::RightOpen,
                    TokenKind::Open => IntervalOp::Open,
                    _ => assume_unreachable!(),
                };
                let l = self.instantiate_const_expr(lhs, None, tmp_tv_cache)?;
                let l = self.eval_tp(l)?;
                let r = self.instantiate_const_expr(rhs, None, tmp_tv_cache)?;
                let r = self.eval_tp(r)?;
                if let Some(Greater) = self.try_cmp(&l, &r) {
                    panic!("{l}..{r} is not a valid interval type (should be lhs <= rhs)")
                }
                Ok(int_interval(op, l, r))
            }
            TypeSpec::Subr(subr) => {
                let mut inner_tv_ctx = if !subr.bounds.is_empty() {
                    let tv_cache = self.instantiate_ty_bounds(&subr.bounds, mode)?;
                    Some(tv_cache)
                } else {
                    None
                };
                if let Some(inner) = &mut inner_tv_ctx {
                    inner.merge(tmp_tv_cache);
                }
                let tmp_tv_ctx = if let Some(inner) = &mut inner_tv_ctx {
                    inner
                } else {
                    tmp_tv_cache
                };
                let non_defaults = try_map_mut(subr.non_defaults.iter(), |p| {
                    self.instantiate_func_param_spec(p, opt_decl_t, None, tmp_tv_ctx, mode)
                })?;
                let var_args = subr
                    .var_args
                    .as_ref()
                    .map(|p| {
                        self.instantiate_func_param_spec(p, opt_decl_t, None, tmp_tv_ctx, mode)
                    })
                    .transpose()?;
                let defaults = try_map_mut(subr.defaults.iter(), |p| {
                    self.instantiate_func_param_spec(
                        &p.param,
                        opt_decl_t,
                        Some(&p.default),
                        tmp_tv_ctx,
                        mode,
                    )
                })?
                .into_iter()
                .collect();
                let return_t = self.instantiate_typespec(
                    &subr.return_t,
                    opt_decl_t,
                    tmp_tv_ctx,
                    mode,
                    not_found_is_qvar,
                )?;
                Ok(subr_t(
                    SubrKind::from(subr.arrow.kind),
                    non_defaults,
                    var_args,
                    defaults,
                    return_t,
                ))
            }
            TypeSpec::TypeApp { spec, args } => {
                todo!("{spec}{args}")
            }
        }
    }

    pub(crate) fn instantiate_ty_bound(
        &self,
        bound: &TypeBoundSpec,
        tv_cache: &mut TyVarCache,
        mode: RegistrationMode,
    ) -> TyCheckResult<()> {
        // REVIEW: 型境界の左辺に来れるのは型変数だけか?
        // TODO: 高階型変数
        match bound {
            TypeBoundSpec::NonDefault { lhs, spec } => {
                let constr =
                    match spec.op.kind {
                        TokenKind::SubtypeOf => Constraint::new_subtype_of(
                            self.instantiate_typespec(&spec.t_spec, None, tv_cache, mode, true)?,
                        ),
                        TokenKind::SupertypeOf => todo!(),
                        TokenKind::Colon => Constraint::new_type_of(self.instantiate_typespec(
                            &spec.t_spec,
                            None,
                            tv_cache,
                            mode,
                            true,
                        )?),
                        _ => unreachable!(),
                    };
                let tv = named_free_var(lhs.inspect().clone(), self.level, constr);
                tv_cache.push_or_init_tyvar(lhs.inspect(), &tv);
            }
            TypeBoundSpec::WithDefault { .. } => todo!(),
        }
        Ok(())
    }

    pub(crate) fn instantiate_ty_bounds(
        &self,
        bounds: &TypeBoundSpecs,
        mode: RegistrationMode,
    ) -> TyCheckResult<TyVarCache> {
        let mut tv_cache = TyVarCache::new(self.level, self);
        for bound in bounds.iter() {
            self.instantiate_ty_bound(bound, &mut tv_cache, mode)?;
        }
        for tv in tv_cache.tyvar_instances.values() {
            if tv.constraint().unwrap().is_uninited() {
                return Err(TyCheckErrors::from(TyCheckError::no_var_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    bounds.loc(),
                    self.caused_by(),
                    &tv.local_name(),
                    self.get_similar_name(&tv.local_name()),
                )));
            }
        }
        for tp in tv_cache.typaram_instances.values() {
            if tp.constraint().unwrap().is_uninited() {
                return Err(TyCheckErrors::from(TyCheckError::no_var_error(
                    self.cfg.input.clone(),
                    line!() as usize,
                    bounds.loc(),
                    self.caused_by(),
                    &tp.to_string(),
                    self.get_similar_name(&tp.to_string()),
                )));
            }
        }
        Ok(tv_cache)
    }

    fn instantiate_tp(
        &self,
        quantified: TyParam,
        tmp_tv_cache: &mut TyVarCache,
        loc: Location,
    ) -> TyCheckResult<TyParam> {
        match quantified {
            TyParam::FreeVar(fv) if fv.is_generalized() => {
                let (name, constr) = (fv.unbound_name().unwrap(), fv.constraint().unwrap());
                if let Some(tp) = tmp_tv_cache.get_typaram(&name) {
                    Ok(tp.clone())
                } else if let Some(t) = tmp_tv_cache.get_tyvar(&name) {
                    Ok(TyParam::t(t.clone()))
                } else {
                    if let Some(tv_cache) = &self.tv_cache {
                        if let Some(tp) = tv_cache.get_typaram(&name) {
                            return Ok(tp.clone());
                        } else if let Some(t) = tv_cache.get_tyvar(&name) {
                            return Ok(TyParam::t(t.clone()));
                        }
                    }
                    let constr = tmp_tv_cache.instantiate_constraint(constr, self, loc)?;
                    let tp = TyParam::named_free_var(name.clone(), self.level, constr);
                    tmp_tv_cache.push_or_init_typaram(&name, &tp);
                    Ok(tp)
                }
            }
            TyParam::Dict(dict) => {
                let dict = dict
                    .into_iter()
                    .map(|(k, v)| {
                        let k = self.instantiate_tp(k, tmp_tv_cache, loc)?;
                        let v = self.instantiate_tp(v, tmp_tv_cache, loc)?;
                        Ok((k, v))
                    })
                    .collect::<TyCheckResult<_>>()?;
                Ok(TyParam::Dict(dict))
            }
            TyParam::Array(arr) => {
                let arr = arr
                    .into_iter()
                    .map(|v| self.instantiate_tp(v, tmp_tv_cache, loc))
                    .collect::<TyCheckResult<_>>()?;
                Ok(TyParam::Array(arr))
            }
            TyParam::Set(set) => {
                let set = set
                    .into_iter()
                    .map(|v| self.instantiate_tp(v, tmp_tv_cache, loc))
                    .collect::<TyCheckResult<_>>()?;
                Ok(TyParam::Set(set))
            }
            TyParam::Tuple(tup) => {
                let tup = tup
                    .into_iter()
                    .map(|v| self.instantiate_tp(v, tmp_tv_cache, loc))
                    .collect::<TyCheckResult<_>>()?;
                Ok(TyParam::Tuple(tup))
            }
            TyParam::UnaryOp { op, val } => {
                let res = self.instantiate_tp(*val, tmp_tv_cache, loc)?;
                Ok(TyParam::unary(op, res))
            }
            TyParam::BinOp { op, lhs, rhs } => {
                let lhs = self.instantiate_tp(*lhs, tmp_tv_cache, loc)?;
                let rhs = self.instantiate_tp(*rhs, tmp_tv_cache, loc)?;
                Ok(TyParam::bin(op, lhs, rhs))
            }
            TyParam::Type(t) => {
                let t = self.instantiate_t_inner(*t, tmp_tv_cache, loc)?;
                Ok(TyParam::t(t))
            }
            TyParam::FreeVar(fv) if fv.is_linked() => {
                self.instantiate_tp(fv.crack().clone(), tmp_tv_cache, loc)
            }
            p @ (TyParam::Value(_)
            | TyParam::Mono(_)
            | TyParam::FreeVar(_)
            | TyParam::Erased(_)) => Ok(p),
            other => todo!("{other}"),
        }
    }

    /// 'T -> ?T (quantified to free)
    pub(crate) fn instantiate_t_inner(
        &self,
        unbound: Type,
        tmp_tv_cache: &mut TyVarCache,
        loc: Location,
    ) -> TyCheckResult<Type> {
        match unbound {
            FreeVar(fv) if fv.is_generalized() => {
                let (name, constr) = (fv.unbound_name().unwrap(), fv.constraint().unwrap());
                if let Some(t) = tmp_tv_cache.get_tyvar(&name) {
                    Ok(t.clone())
                } else if let Some(tp) = tmp_tv_cache.get_typaram(&name) {
                    if let TyParam::Type(t) = tp {
                        Ok(*t.clone())
                    } else {
                        todo!(
                            "typaram_insts: {}\ntyvar_insts:{}\n{tp}",
                            tmp_tv_cache.typaram_instances,
                            tmp_tv_cache.tyvar_instances,
                        )
                    }
                } else {
                    if let Some(tv_ctx) = &self.tv_cache {
                        if let Some(t) = tv_ctx.get_tyvar(&name) {
                            return Ok(t.clone());
                        } else if let Some(tp) = tv_ctx.get_typaram(&name) {
                            if let TyParam::Type(t) = tp {
                                return Ok(*t.clone());
                            } else {
                                todo!(
                                    "typaram_insts: {}\ntyvar_insts:{}\n{tp}",
                                    tmp_tv_cache.typaram_instances,
                                    tmp_tv_cache.tyvar_instances,
                                )
                            }
                        }
                    }
                    let constr = tmp_tv_cache.instantiate_constraint(constr, self, loc)?;
                    let tyvar = named_free_var(name.clone(), self.level, constr);
                    tmp_tv_cache.push_or_init_tyvar(&name, &tyvar);
                    Ok(tyvar)
                }
            }
            Refinement(mut refine) => {
                refine.t = Box::new(self.instantiate_t_inner(*refine.t, tmp_tv_cache, loc)?);
                let mut new_preds = set! {};
                for mut pred in refine.preds.into_iter() {
                    for tp in pred.typarams_mut() {
                        *tp = self.instantiate_tp(mem::take(tp), tmp_tv_cache, loc)?;
                    }
                    new_preds.insert(pred);
                }
                refine.preds = new_preds;
                Ok(Type::Refinement(refine))
            }
            Subr(mut subr) => {
                for pt in subr.non_default_params.iter_mut() {
                    *pt.typ_mut() =
                        self.instantiate_t_inner(mem::take(pt.typ_mut()), tmp_tv_cache, loc)?;
                }
                if let Some(var_args) = subr.var_params.as_mut() {
                    *var_args.typ_mut() =
                        self.instantiate_t_inner(mem::take(var_args.typ_mut()), tmp_tv_cache, loc)?;
                }
                for pt in subr.default_params.iter_mut() {
                    *pt.typ_mut() =
                        self.instantiate_t_inner(mem::take(pt.typ_mut()), tmp_tv_cache, loc)?;
                }
                let return_t = self.instantiate_t_inner(*subr.return_t, tmp_tv_cache, loc)?;
                let res = subr_t(
                    subr.kind,
                    subr.non_default_params,
                    subr.var_params.map(|p| *p),
                    subr.default_params,
                    return_t,
                );
                Ok(res)
            }
            Record(mut dict) => {
                for v in dict.values_mut() {
                    *v = self.instantiate_t_inner(mem::take(v), tmp_tv_cache, loc)?;
                }
                Ok(Type::Record(dict))
            }
            Ref(t) => {
                let t = self.instantiate_t_inner(*t, tmp_tv_cache, loc)?;
                Ok(ref_(t))
            }
            RefMut { before, after } => {
                let before = self.instantiate_t_inner(*before, tmp_tv_cache, loc)?;
                let after = after
                    .map(|aft| self.instantiate_t_inner(*aft, tmp_tv_cache, loc))
                    .transpose()?;
                Ok(ref_mut(before, after))
            }
            Proj { lhs, rhs } => {
                let lhs = self.instantiate_t_inner(*lhs, tmp_tv_cache, loc)?;
                Ok(proj(lhs, rhs))
            }
            ProjCall {
                lhs,
                attr_name,
                mut args,
            } => {
                let lhs = self.instantiate_tp(*lhs, tmp_tv_cache, loc)?;
                for arg in args.iter_mut() {
                    *arg = self.instantiate_tp(mem::take(arg), tmp_tv_cache, loc)?;
                }
                Ok(proj_call(lhs, attr_name, args))
            }
            Poly { name, mut params } => {
                for param in params.iter_mut() {
                    *param = self.instantiate_tp(mem::take(param), tmp_tv_cache, loc)?;
                }
                Ok(poly(name, params))
            }
            Quantified(_) => {
                panic!("a quantified type should not be instantiated, instantiate the inner type")
            }
            FreeVar(fv) if fv.is_linked() => {
                self.instantiate_t_inner(fv.crack().clone(), tmp_tv_cache, loc)
            }
            FreeVar(fv) => {
                let (sub, sup) = fv.get_subsup().unwrap();
                let sub = self.instantiate_t_inner(sub, tmp_tv_cache, loc)?;
                let sup = self.instantiate_t_inner(sup, tmp_tv_cache, loc)?;
                let new_constraint = Constraint::new_sandwiched(sub, sup);
                fv.update_constraint(new_constraint);
                Ok(FreeVar(fv))
            }
            And(l, r) => {
                let l = self.instantiate_t_inner(*l, tmp_tv_cache, loc)?;
                let r = self.instantiate_t_inner(*r, tmp_tv_cache, loc)?;
                Ok(self.intersection(&l, &r))
            }
            Or(l, r) => {
                let l = self.instantiate_t_inner(*l, tmp_tv_cache, loc)?;
                let r = self.instantiate_t_inner(*r, tmp_tv_cache, loc)?;
                Ok(self.union(&l, &r))
            }
            Not(l, r) => {
                let l = self.instantiate_t_inner(*l, tmp_tv_cache, loc)?;
                let r = self.instantiate_t_inner(*r, tmp_tv_cache, loc)?;
                Ok(not(l, r))
            }
            other if other.is_monomorphic() => Ok(other),
            other => todo!("{other}"),
        }
    }

    pub(crate) fn instantiate(&self, quantified: Type, callee: &hir::Expr) -> TyCheckResult<Type> {
        match quantified {
            Quantified(quant) => {
                let mut tmp_tv_cache = TyVarCache::new(self.level, self);
                let t = self.instantiate_t_inner(*quant, &mut tmp_tv_cache, callee.loc())?;
                match &t {
                    Type::Subr(subr) => {
                        if let Some(self_t) = subr.self_t() {
                            self.sub_unify(
                                callee.ref_t(),
                                self_t,
                                callee.loc(),
                                Some(&Str::ever("self")),
                            )?;
                        }
                    }
                    _ => unreachable!(),
                }
                if cfg!(feature = "debug") && t.has_qvar() {
                    panic!("{t} has qvar")
                }
                Ok(t)
            }
            // HACK: {op: |T|(T -> T) | op == F} => ?T -> ?T
            Refinement(refine) if refine.t.is_quantified() => {
                let quant = enum_unwrap!(*refine.t, Type::Quantified);
                let mut tmp_tv_cache = TyVarCache::new(self.level, self);
                let t = self.instantiate_t_inner(*quant, &mut tmp_tv_cache, callee.loc())?;
                match &t {
                    Type::Subr(subr) => {
                        if let Some(self_t) = subr.self_t() {
                            self.sub_unify(
                                callee.ref_t(),
                                self_t,
                                callee.loc(),
                                Some(&Str::ever("self")),
                            )?;
                        }
                    }
                    _ => unreachable!(),
                }
                Ok(t)
            }
            // rank-1制限により、通常の型(rank-0型)の内側に量化型は存在しない
            other => Ok(other),
        }
    }
}
