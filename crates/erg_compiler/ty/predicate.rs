use std::fmt;
use std::ops::{BitAnd, BitOr, Not};

#[allow(unused_imports)]
use erg_common::log;
use erg_common::set::Set;
use erg_common::{set, Str};

use super::free::{Constraint, HasLevel};
use super::typaram::TyParam;
use super::value::ValueObj;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Predicate {
    Value(ValueObj), // True/False
    Const(Str),
    /// i == 0 => Eq{ lhs: "i", rhs: 0 }
    Equal {
        lhs: Str,
        rhs: TyParam,
    },
    /// i > 0 => i >= 0+ε => GreaterEqual{ lhs: "i", rhs: 0+ε }
    GreaterEqual {
        lhs: Str,
        rhs: TyParam,
    },
    LessEqual {
        lhs: Str,
        rhs: TyParam,
    },
    NotEqual {
        lhs: Str,
        rhs: TyParam,
    },
    Or(Box<Predicate>, Box<Predicate>),
    And(Box<Predicate>, Box<Predicate>),
    Not(Box<Predicate>),
}

impl fmt::Display for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(v) => write!(f, "{v}"),
            Self::Const(c) => write!(f, "{c}"),
            Self::Equal { lhs, rhs } => write!(f, "{lhs} == {rhs}"),
            Self::GreaterEqual { lhs, rhs } => write!(f, "{lhs} >= {rhs}"),
            Self::LessEqual { lhs, rhs } => write!(f, "{lhs} <= {rhs}"),
            Self::NotEqual { lhs, rhs } => write!(f, "{lhs} != {rhs}"),
            Self::Or(l, r) => write!(f, "({l}) or ({r})"),
            Self::And(l, r) => write!(f, "({l}) and ({r})"),
            Self::Not(p) => write!(f, "not ({p})"),
        }
    }
}

impl HasLevel for Predicate {
    fn level(&self) -> Option<usize> {
        match self {
            Self::Value(_) | Self::Const(_) => None,
            Self::Equal { rhs, .. }
            | Self::GreaterEqual { rhs, .. }
            | Self::LessEqual { rhs, .. }
            | Self::NotEqual { rhs, .. } => rhs.level(),
            Self::And(lhs, rhs) | Self::Or(lhs, rhs) => {
                lhs.level().zip(rhs.level()).map(|(a, b)| a.min(b))
            }
            Self::Not(p) => p.level(),
        }
    }

    fn set_level(&self, level: usize) {
        match self {
            Self::Value(_) | Self::Const(_) => {}
            Self::Equal { rhs, .. }
            | Self::GreaterEqual { rhs, .. }
            | Self::LessEqual { rhs, .. }
            | Self::NotEqual { rhs, .. } => {
                rhs.set_level(level);
            }
            Self::And(lhs, rhs) | Self::Or(lhs, rhs) => {
                lhs.set_level(level);
                rhs.set_level(level);
            }
            Self::Not(p) => {
                p.set_level(level);
            }
        }
    }
}

impl BitAnd for Predicate {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self::and(self, rhs)
    }
}

impl BitOr for Predicate {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self::or(self, rhs)
    }
}

impl Not for Predicate {
    type Output = Self;

    fn not(self) -> Self::Output {
        Self::Not(Box::new(self))
    }
}

impl Predicate {
    pub const TRUE: Predicate = Predicate::Value(ValueObj::Bool(true));
    pub const FALSE: Predicate = Predicate::Value(ValueObj::Bool(false));

    pub const fn eq(lhs: Str, rhs: TyParam) -> Self {
        Self::Equal { lhs, rhs }
    }
    pub const fn ne(lhs: Str, rhs: TyParam) -> Self {
        Self::NotEqual { lhs, rhs }
    }
    /// >=
    pub const fn ge(lhs: Str, rhs: TyParam) -> Self {
        Self::GreaterEqual { lhs, rhs }
    }

    /// > (>= and !=)
    pub fn gt(lhs: Str, rhs: TyParam) -> Self {
        Self::and(Self::ge(lhs.clone(), rhs.clone()), Self::ne(lhs, rhs))
    }

    /// <=
    pub const fn le(lhs: Str, rhs: TyParam) -> Self {
        Self::LessEqual { lhs, rhs }
    }

    // < (<= and !=)
    pub fn lt(lhs: Str, rhs: TyParam) -> Self {
        Self::and(Self::le(lhs.clone(), rhs.clone()), Self::ne(lhs, rhs))
    }

    pub fn and(lhs: Predicate, rhs: Predicate) -> Self {
        match (lhs, rhs) {
            (Predicate::Value(ValueObj::Bool(true)), p) => p,
            (p, Predicate::Value(ValueObj::Bool(true))) => p,
            (Predicate::Value(ValueObj::Bool(false)), _)
            | (_, Predicate::Value(ValueObj::Bool(false))) => Predicate::FALSE,
            (p1, p2) => Self::And(Box::new(p1), Box::new(p2)),
        }
    }

    pub fn or(lhs: Predicate, rhs: Predicate) -> Self {
        match (lhs, rhs) {
            (Predicate::Value(ValueObj::Bool(true)), _)
            | (_, Predicate::Value(ValueObj::Bool(true))) => Predicate::TRUE,
            (Predicate::Value(ValueObj::Bool(false)), p) => p,
            (p, Predicate::Value(ValueObj::Bool(false))) => p,
            (p1, p2) => Self::Or(Box::new(p1), Box::new(p2)),
        }
    }

    pub fn is_equal(&self) -> bool {
        matches!(self, Self::Equal { .. })
    }

    pub fn consist_of_equal(&self) -> bool {
        match self {
            Self::Equal { .. } => true,
            Self::And(lhs, rhs) | Self::Or(lhs, rhs) => {
                lhs.consist_of_equal() && rhs.consist_of_equal()
            }
            Self::Not(pred) => pred.consist_of_equal(),
            _ => false,
        }
    }

    pub fn ands(&self) -> Set<&Predicate> {
        match self {
            Self::And(lhs, rhs) => {
                let mut set = lhs.ands();
                set.extend(rhs.ands());
                set
            }
            _ => set! { self },
        }
    }

    pub fn ors(&self) -> Set<&Predicate> {
        match self {
            Self::Or(lhs, rhs) => {
                let mut set = lhs.ors();
                set.extend(rhs.ors());
                set
            }
            _ => set! { self },
        }
    }

    pub fn subject(&self) -> Option<&str> {
        match self {
            Self::Equal { lhs, .. }
            | Self::LessEqual { lhs, .. }
            | Self::GreaterEqual { lhs, .. }
            | Self::NotEqual { lhs, .. } => Some(&lhs[..]),
            Self::And(lhs, rhs) | Self::Or(lhs, rhs) => {
                let l = lhs.subject();
                let r = rhs.subject();
                if l != r {
                    todo!()
                } else {
                    l
                }
            }
            Self::Not(pred) => pred.subject(),
            _ => None,
        }
    }

    pub fn change_subject_name(self, name: Str) -> Self {
        match self {
            Self::Equal { rhs, .. } => Self::eq(name, rhs),
            Self::GreaterEqual { rhs, .. } => Self::ge(name, rhs),
            Self::LessEqual { rhs, .. } => Self::le(name, rhs),
            Self::NotEqual { rhs, .. } => Self::ne(name, rhs),
            Self::And(lhs, rhs) => Self::and(
                lhs.change_subject_name(name.clone()),
                rhs.change_subject_name(name),
            ),
            Self::Or(lhs, rhs) => Self::or(
                lhs.change_subject_name(name.clone()),
                rhs.change_subject_name(name),
            ),
            Self::Not(pred) => Self::not(pred.change_subject_name(name)),
            _ => self,
        }
    }

    pub fn mentions(&self, name: &str) -> bool {
        match self {
            Self::Const(n) => &n[..] == name,
            Self::Equal { lhs, .. }
            | Self::LessEqual { lhs, .. }
            | Self::GreaterEqual { lhs, .. }
            | Self::NotEqual { lhs, .. } => &lhs[..] == name,
            Self::And(lhs, rhs) | Self::Or(lhs, rhs) => lhs.mentions(name) || rhs.mentions(name),
            _ => false,
        }
    }

    pub fn can_be_false(&self) -> bool {
        match self {
            Self::Value(l) => matches!(l, ValueObj::Bool(false)),
            Self::Const(_) => todo!(),
            Self::Or(lhs, rhs) => lhs.can_be_false() || rhs.can_be_false(),
            Self::And(lhs, rhs) => lhs.can_be_false() && rhs.can_be_false(),
            Self::Not(pred) => !pred.can_be_false(),
            _ => true,
        }
    }

    pub fn qvars(&self) -> Set<(Str, Constraint)> {
        match self {
            Self::Value(_) | Self::Const(_) => set! {},
            Self::Equal { rhs, .. }
            | Self::GreaterEqual { rhs, .. }
            | Self::LessEqual { rhs, .. }
            | Self::NotEqual { rhs, .. } => rhs.qvars(),
            Self::And(lhs, rhs) | Self::Or(lhs, rhs) => lhs.qvars().concat(rhs.qvars()),
            Self::Not(pred) => pred.qvars(),
        }
    }

    pub fn has_qvar(&self) -> bool {
        match self {
            Self::Value(_) => false,
            Self::Const(_) => false,
            Self::Equal { rhs, .. }
            | Self::GreaterEqual { rhs, .. }
            | Self::LessEqual { rhs, .. }
            | Self::NotEqual { rhs, .. } => rhs.has_qvar(),
            Self::Or(lhs, rhs) | Self::And(lhs, rhs) => lhs.has_qvar() || rhs.has_qvar(),
            Self::Not(pred) => pred.has_qvar(),
        }
    }

    pub fn has_unbound_var(&self) -> bool {
        match self {
            Self::Value(_) => false,
            Self::Const(_) => false,
            Self::Equal { rhs, .. }
            | Self::GreaterEqual { rhs, .. }
            | Self::LessEqual { rhs, .. }
            | Self::NotEqual { rhs, .. } => rhs.has_unbound_var(),
            Self::Or(lhs, rhs) | Self::And(lhs, rhs) => {
                lhs.has_unbound_var() || rhs.has_unbound_var()
            }
            Self::Not(pred) => pred.has_unbound_var(),
        }
    }

    pub fn min_max<'a>(
        &'a self,
        min: Option<&'a TyParam>,
        max: Option<&'a TyParam>,
    ) -> (Option<&'a TyParam>, Option<&'a TyParam>) {
        match self {
            Predicate::Equal { rhs: _, .. } => todo!(),
            // {I | I <= 1; I <= 2}
            Predicate::LessEqual { rhs, .. } => (
                min,
                max.map(|l: &TyParam| match l.cheap_cmp(rhs) {
                    Some(c) if c.is_ge() => l,
                    Some(_) => rhs,
                    _ => l,
                })
                .or(Some(rhs)),
            ),
            // {I | I >= 1; I >= 2}
            Predicate::GreaterEqual { rhs, .. } => (
                min.map(|l: &TyParam| match l.cheap_cmp(rhs) {
                    Some(c) if c.is_le() => l,
                    Some(_) => rhs,
                    _ => l,
                })
                .or(Some(rhs)),
                max,
            ),
            Predicate::And(_l, _r) => todo!(),
            _ => todo!(),
        }
    }

    pub fn typarams(&self) -> Vec<&TyParam> {
        match self {
            Self::Value(_) | Self::Const(_) => vec![],
            Self::Equal { rhs, .. }
            | Self::GreaterEqual { rhs, .. }
            | Self::LessEqual { rhs, .. }
            | Self::NotEqual { rhs, .. } => vec![rhs],
            Self::And(lhs, rhs) | Self::Or(lhs, rhs) => {
                lhs.typarams().into_iter().chain(rhs.typarams()).collect()
            }
            Self::Not(pred) => pred.typarams(),
        }
    }

    pub fn typarams_mut(&mut self) -> Vec<&mut TyParam> {
        match self {
            Self::Value(_) | Self::Const(_) => vec![],
            Self::Equal { rhs, .. }
            | Self::GreaterEqual { rhs, .. }
            | Self::LessEqual { rhs, .. }
            | Self::NotEqual { rhs, .. } => vec![rhs],
            Self::And(lhs, rhs) | Self::Or(lhs, rhs) => lhs
                .typarams_mut()
                .into_iter()
                .chain(rhs.typarams_mut())
                .collect(),
            Self::Not(pred) => pred.typarams_mut(),
        }
    }

    pub fn invert(self) -> Self {
        match self {
            Self::Value(ValueObj::Bool(b)) => Self::Value(ValueObj::Bool(!b)),
            Self::Equal { lhs, rhs } => Self::ne(lhs, rhs),
            Self::GreaterEqual { lhs, rhs } => Self::lt(lhs, rhs),
            Self::LessEqual { lhs, rhs } => Self::gt(lhs, rhs),
            Self::NotEqual { lhs, rhs } => Self::eq(lhs, rhs),
            Self::Not(pred) => *pred,
            other => Self::not(other),
        }
    }
}
