use std::fmt;
use std::mem;
use std::option::Option; // conflicting to Type::Option

use erg_common::dict::Dict;
use erg_common::enum_unwrap;
#[allow(unused)]
use erg_common::log;
use erg_common::set::Set;
use erg_common::traits::Locational;
use erg_common::Str;

use crate::feature_error;
use crate::ty::constructors::*;
use crate::ty::free::{Constraint, HasLevel};
use crate::ty::typaram::{TyParam, TyParamLambda};
use crate::ty::{HasType, Predicate, Type};
use crate::{type_feature_error, unreachable_error};
use Type::*;

use crate::context::Context;
use crate::error::{TyCheckError, TyCheckErrors, TyCheckResult};
use crate::hir;

/// Context for instantiating a quantified type
/// For example, cloning each type variable of quantified type `?T -> ?T` would result in `?1 -> ?2`.
/// To avoid this, an environment to store type variables is needed, which is `TyVarCache`.
/// 量化型をインスタンス化するための文脈
/// e.g. Array -> [("T": ?T(: Type)), ("N": ?N(: Nat))]
/// FIXME: current implementation is wrong
/// It will not work unless the type variable is used with the same name as the definition.
#[derive(Debug, Clone)]
pub struct TyVarCache {
    _level: usize,
    pub(crate) already_appeared: Set<Str>,
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
            already_appeared: Set::new(),
            tyvar_instances: Dict::new(),
            typaram_instances: Dict::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tyvar_instances.is_empty() && self.typaram_instances.is_empty()
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
        loc: &impl Locational,
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

    /// Some of the quantified types are circulating.
    /// e.g.
    /// ```erg
    /// add: |T <: Add(T(<: Add(T(<: ...))))|(T, T) -> T.Output
    /// ```
    /// `T` in `Add` should be instantiated as `Constraint::Uninited`.
    /// And with the outer `T`, the Compiler will overwrite the inner `T`'s constraint.
    /// ```erg
    /// T <: Add(?T(: Uninited))
    /// ↓
    /// ?T <: Add(?T(<: Add(?T(<: ...))))
    /// ```
    /// After the instantiation:
    /// ```erg
    /// add: (?T(<: Add(?T)), ?T(<: ...)) -> ?T(<: ...).Output
    /// ```
    /// Therefore, it is necessary to register the type variables that appear inside.
    pub(crate) fn push_appeared(&mut self, name: Str) {
        self.already_appeared.insert(name);
    }

    pub(crate) fn push_or_init_tyvar(&mut self, name: &Str, tv: &Type, ctx: &Context) {
        if let Some(inst) = self.tyvar_instances.get(name) {
            self.update_tv(inst, tv, ctx);
        } else if let Some(inst) = self.typaram_instances.get(name) {
            if let TyParam::Type(inst) = inst {
                self.update_tv(inst, tv, ctx);
            } else if let TyParam::FreeVar(fv) = inst {
                fv.link(&TyParam::t(tv.clone()));
            } else {
                unreachable!()
            }
        } else {
            self.tyvar_instances.insert(name.clone(), tv.clone());
        }
    }

    fn update_tv(&self, inst: &Type, tv: &Type, ctx: &Context) {
        // T<tv> <: Eq(T<inst>)
        // T<inst> is uninitialized
        // T<inst>.link(T<tv>);
        // T <: Eq(T <: Eq(T <: ...))
        let inst = enum_unwrap!(inst, Type::FreeVar);
        if inst.constraint_is_uninited() {
            inst.link(tv);
        } else {
            // inst: ?T(<: Int) => old_sub: Never, old_sup: Int
            // tv: ?T(:> Nat) => new_sub: Nat, new_sup: Obj
            // => ?T(:> Nat, <: Int)
            // inst: ?T(:> Str)
            // tv: ?T(:> Nat)
            // => ?T(:> Nat or Str)
            let (old_sub, old_sup) = inst.get_subsup().unwrap();
            let tv = enum_unwrap!(tv, Type::FreeVar);
            let (new_sub, new_sup) = tv.get_subsup().unwrap();
            let new_constraint = Constraint::new_sandwiched(
                ctx.union(&old_sub, &new_sub),
                ctx.intersection(&old_sup, &new_sup),
            );
            inst.update_constraint(new_constraint, true);
        }
    }

    pub(crate) fn push_or_init_typaram(&mut self, name: &Str, tp: &TyParam) {
        // FIXME:
        if let Some(_tp) = self.typaram_instances.get(name) {
            panic!("{_tp} {tp}");
            // return;
        } else if let Some(_t) = self.tyvar_instances.get(name) {
            panic!("{_t} {tp}");
            // return;
        } else {
            self.typaram_instances.insert(name.clone(), tp.clone());
        }
    }

    pub(crate) fn appeared(&self, name: &Str) -> bool {
        self.already_appeared.contains(name)
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

impl Context {
    fn instantiate_tp(
        &self,
        quantified: TyParam,
        tmp_tv_cache: &mut TyVarCache,
        loc: &impl Locational,
    ) -> TyCheckResult<TyParam> {
        match quantified {
            TyParam::FreeVar(fv) if fv.is_linked() => {
                self.instantiate_tp(fv.crack().clone(), tmp_tv_cache, loc)
            }
            TyParam::FreeVar(fv) if fv.is_generalized() => {
                let (name, constr) = (fv.unbound_name().unwrap(), fv.constraint().unwrap());
                if let Some(tp) = tmp_tv_cache.get_typaram(&name) {
                    let tp = tp.clone();
                    if let TyParam::FreeVar(fv) = &tp {
                        if fv
                            .constraint()
                            .map(|cons| cons.is_uninited())
                            .unwrap_or(false)
                        {
                            let new_constr =
                                tmp_tv_cache.instantiate_constraint(constr, self, loc)?;
                            fv.update_constraint(new_constr, true);
                        }
                    }
                    Ok(tp)
                } else if let Some(t) = tmp_tv_cache.get_tyvar(&name) {
                    let t = t.clone();
                    if let Type::FreeVar(fv) = &t {
                        if fv
                            .constraint()
                            .map(|cons| cons.is_uninited())
                            .unwrap_or(false)
                        {
                            let new_constr =
                                tmp_tv_cache.instantiate_constraint(constr, self, loc)?;
                            fv.update_constraint(new_constr, true);
                        }
                    }
                    Ok(TyParam::t(t))
                } else {
                    if tmp_tv_cache.appeared(&name) {
                        let tp =
                            TyParam::named_free_var(name.clone(), self.level, Constraint::Uninited);
                        tmp_tv_cache.push_or_init_typaram(&name, &tp);
                        return Ok(tp);
                    }
                    if let Some(tv_cache) = &self.tv_cache {
                        if let Some(tp) = tv_cache.get_typaram(&name) {
                            return Ok(tp.clone());
                        } else if let Some(t) = tv_cache.get_tyvar(&name) {
                            return Ok(TyParam::t(t.clone()));
                        }
                    }
                    tmp_tv_cache.push_appeared(name.clone());
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
            TyParam::Record(rec) => {
                let rec = rec
                    .into_iter()
                    .map(|(k, v)| {
                        let v = self.instantiate_tp(v, tmp_tv_cache, loc)?;
                        Ok((k, v))
                    })
                    .collect::<TyCheckResult<_>>()?;
                Ok(TyParam::Record(rec))
            }
            TyParam::Lambda(lambda) => {
                let nd_params = lambda
                    .nd_params
                    .into_iter()
                    .map(|pt| pt.try_map_type(|t| self.instantiate_t_inner(t, tmp_tv_cache, loc)))
                    .collect::<TyCheckResult<_>>()?;
                let var_params = lambda
                    .var_params
                    .map(|pt| pt.try_map_type(|t| self.instantiate_t_inner(t, tmp_tv_cache, loc)))
                    .transpose()?;
                let d_params = lambda
                    .d_params
                    .into_iter()
                    .map(|pt| pt.try_map_type(|t| self.instantiate_t_inner(t, tmp_tv_cache, loc)))
                    .collect::<TyCheckResult<_>>()?;
                let body = lambda
                    .body
                    .into_iter()
                    .map(|v| self.instantiate_tp(v, tmp_tv_cache, loc))
                    .collect::<TyCheckResult<_>>()?;
                Ok(TyParam::Lambda(TyParamLambda::new(
                    lambda.const_,
                    nd_params,
                    var_params,
                    d_params,
                    body,
                )))
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
            other => {
                type_feature_error!(
                    self,
                    loc.loc(),
                    &format!("instantiating type-parameter {other}")
                )
            }
        }
    }

    /// 'T -> ?T (quantified to free)
    pub(crate) fn instantiate_t_inner(
        &self,
        unbound: Type,
        tmp_tv_cache: &mut TyVarCache,
        loc: &impl Locational,
    ) -> TyCheckResult<Type> {
        match unbound {
            FreeVar(fv) if fv.is_linked() => {
                self.instantiate_t_inner(fv.crack().clone(), tmp_tv_cache, loc)
            }
            FreeVar(fv) if fv.is_generalized() => {
                let (name, constr) = (fv.unbound_name().unwrap(), fv.constraint().unwrap());
                if let Some(t) = tmp_tv_cache.get_tyvar(&name) {
                    let t = t.clone();
                    if let Type::FreeVar(fv) = &t {
                        if fv
                            .constraint()
                            .map(|cons| cons.is_uninited())
                            .unwrap_or(false)
                        {
                            let new_constr =
                                tmp_tv_cache.instantiate_constraint(constr, self, loc)?;
                            fv.update_constraint(new_constr, true);
                        }
                    }
                    Ok(t)
                } else if let Some(tp) = tmp_tv_cache.get_typaram(&name) {
                    if let TyParam::Type(t) = tp {
                        let t = *t.clone();
                        if let Type::FreeVar(fv) = &t {
                            if fv
                                .constraint()
                                .map(|cons| cons.is_uninited())
                                .unwrap_or(false)
                            {
                                let new_constr =
                                    tmp_tv_cache.instantiate_constraint(constr, self, loc)?;
                                fv.update_constraint(new_constr, true);
                            }
                        }
                        Ok(t)
                    } else {
                        todo!(
                            "typaram_insts: {}\ntyvar_insts:{}\n{tp}",
                            tmp_tv_cache.typaram_instances,
                            tmp_tv_cache.tyvar_instances,
                        )
                    }
                } else {
                    if tmp_tv_cache.appeared(&name) {
                        let tyvar = named_free_var(name.clone(), self.level, Constraint::Uninited);
                        tmp_tv_cache.push_or_init_tyvar(&name, &tyvar, self);
                        return Ok(tyvar);
                    }
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
                    tmp_tv_cache.push_appeared(name.clone());
                    let constr = tmp_tv_cache.instantiate_constraint(constr, self, loc)?;
                    let tyvar = named_free_var(name.clone(), self.level, constr);
                    tmp_tv_cache.push_or_init_tyvar(&name, &tyvar, self);
                    Ok(tyvar)
                }
            }
            Refinement(mut refine) => {
                refine.t = Box::new(self.instantiate_t_inner(*refine.t, tmp_tv_cache, loc)?);
                for tp in refine.pred.typarams_mut() {
                    *tp = self.instantiate_tp(mem::take(tp), tmp_tv_cache, loc)?;
                }
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
            Quantified(subr) => {
                log!(err "a quantified type should not be instantiated: {subr}");
                unreachable_error!(TyCheckErrors, TyCheckError, self)
            }
            Structural(t) => {
                let t = self.instantiate_t_inner(*t, tmp_tv_cache, loc)?;
                Ok(t.structuralize())
            }
            FreeVar(fv) if fv.is_linked() => {
                self.instantiate_t_inner(fv.crack().clone(), tmp_tv_cache, loc)
            }
            FreeVar(fv) => {
                let (sub, sup) = fv.get_subsup().unwrap();
                let sub = self.instantiate_t_inner(sub, tmp_tv_cache, loc)?;
                let sup = self.instantiate_t_inner(sup, tmp_tv_cache, loc)?;
                let new_constraint = Constraint::new_sandwiched(sub, sup);
                fv.update_constraint(new_constraint, true);
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
            Not(ty) => {
                let ty = self.instantiate_t_inner(*ty, tmp_tv_cache, loc)?;
                Ok(self.complement(&ty))
            }
            other if other.is_monomorphic() => Ok(other),
            other => type_feature_error!(self, loc.loc(), &format!("instantiating type {other}")),
        }
    }

    pub(crate) fn instantiate(&self, quantified: Type, callee: &hir::Expr) -> TyCheckResult<Type> {
        match quantified {
            FreeVar(fv) if fv.is_linked() => self.instantiate(fv.crack().clone(), callee),
            Quantified(quant) => {
                let mut tmp_tv_cache = TyVarCache::new(self.level, self);
                let ty = self.instantiate_t_inner(*quant, &mut tmp_tv_cache, callee)?;
                if let Some(self_t) = ty.self_t() {
                    self.sub_unify(callee.ref_t(), self_t, callee, Some(&Str::ever("self")))?;
                }
                if cfg!(feature = "debug") && ty.has_qvar() {
                    panic!("{ty} has qvar")
                }
                Ok(ty)
            }
            // HACK: {op: |T|(T -> T) | op == F} => ?T -> ?T
            Refinement(refine) if refine.t.is_quantified_subr() => {
                let quant = enum_unwrap!(*refine.t, Type::Quantified);
                let mut tmp_tv_cache = TyVarCache::new(self.level, self);
                let t = self.instantiate_t_inner(*quant, &mut tmp_tv_cache, callee)?;
                match &t {
                    Type::Subr(subr) => {
                        if let Some(self_t) = subr.self_t() {
                            self.sub_unify(
                                callee.ref_t(),
                                self_t,
                                callee,
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

    pub(crate) fn instantiate_dummy(&self, quantified: Type) -> TyCheckResult<Type> {
        match quantified {
            FreeVar(fv) if fv.is_linked() => self.instantiate_dummy(fv.crack().clone()),
            Quantified(quant) => {
                let mut tmp_tv_cache = TyVarCache::new(self.level, self);
                let ty = self.instantiate_t_inner(*quant, &mut tmp_tv_cache, &())?;
                if cfg!(feature = "debug") && ty.has_qvar() {
                    panic!("{ty} has qvar")
                }
                Ok(ty)
            }
            Refinement(refine) if refine.t.is_quantified_subr() => {
                let quant = enum_unwrap!(*refine.t, Type::Quantified);
                let mut tmp_tv_cache = TyVarCache::new(self.level, self);
                self.instantiate_t_inner(*quant, &mut tmp_tv_cache, &())
            }
            _other => unreachable_error!(TyCheckErrors, TyCheckError, self),
        }
    }
}
