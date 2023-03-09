use std::borrow::{Borrow, Cow};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Add, Deref};

pub type RcStr = std::rc::Rc<str>;

/// Used to hold an immutable string.
///
/// It can construct as a const (by Str::ever).
#[derive(Debug, Clone, Eq)]
pub enum Str {
    Rc(RcStr),
    Static(&'static str),
}

impl PartialEq for Str {
    #[inline]
    fn eq(&self, other: &Str) -> bool {
        self[..] == other[..]
    }
}

impl Add<&str> for Str {
    type Output = Str;
    #[inline]
    fn add(self, other: &str) -> Str {
        Str::from(&format!("{self}{other}"))
    }
}

impl Hash for Str {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Str::Rc(s) => s[..].hash(state),
            Str::Static(s) => (*s).hash(state),
        }
    }
}

impl fmt::Display for Str {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Str::Rc(s) => write!(f, "{s}"),
            Str::Static(s) => write!(f, "{s}"),
        }
    }
}

impl From<&Str> for String {
    #[inline]
    fn from(s: &Str) -> Self {
        s.to_string()
    }
}

impl From<Str> for String {
    #[inline]
    fn from(s: Str) -> Self {
        s.to_string()
    }
}

impl<'a> From<Str> for Cow<'a, str> {
    fn from(s: Str) -> Self {
        match s {
            Str::Static(s) => Cow::Borrowed(s),
            Str::Rc(s) => Cow::Owned(s.to_string()),
        }
    }
}

// &'static str -> &strになってしまわないように
// あえて`impl<S: Into<Str>> From<S> for Str { ... }`はしない
impl From<&'static str> for Str {
    #[inline]
    fn from(s: &'static str) -> Self {
        Str::ever(s)
    }
}

impl From<&String> for Str {
    #[inline]
    fn from(s: &String) -> Self {
        Str::Rc((s[..]).into())
    }
}

impl From<String> for Str {
    #[inline]
    fn from(s: String) -> Self {
        Str::Rc((s[..]).into())
    }
}

impl From<&RcStr> for Str {
    #[inline]
    fn from(s: &RcStr) -> Self {
        Str::Rc(s.clone())
    }
}

impl From<RcStr> for Str {
    #[inline]
    fn from(s: RcStr) -> Self {
        Str::Rc(s)
    }
}

impl From<&Str> for Str {
    #[inline]
    fn from(s: &Str) -> Self {
        match s {
            Str::Rc(s) => Str::Rc(s.clone()),
            Str::Static(s) => Str::Static(s),
        }
    }
}

impl From<Cow<'_, str>> for Str {
    #[inline]
    fn from(s: Cow<'_, str>) -> Self {
        match s {
            Cow::Borrowed(s) => Str::rc(s),
            Cow::Owned(s) => Str::Rc(s.into()),
        }
    }
}

impl Deref for Str {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        self.borrow()
    }
}

impl Borrow<str> for Str {
    #[inline]
    fn borrow(&self) -> &str {
        match self {
            Str::Rc(s) => &s[..],
            Str::Static(s) => s,
        }
    }
}

impl AsRef<str> for Str {
    fn as_ref(&self) -> &str {
        self.borrow()
    }
}

impl Str {
    pub const fn ever(s: &'static str) -> Self {
        Str::Static(s)
    }

    pub fn rc(s: &str) -> Self {
        Str::Rc(s.into())
    }

    pub fn into_rc(self) -> RcStr {
        match self {
            Str::Rc(s) => s,
            Str::Static(s) => RcStr::from(s),
        }
    }

    pub fn is_uppercase(&self) -> bool {
        self.chars()
            .next()
            .map(|c| c.is_uppercase())
            .unwrap_or(false)
    }

    pub fn split_with(&self, seps: &[&str]) -> Vec<&str> {
        let mut ret = vec![];
        let mut start = 0;
        #[allow(unused_assignments)]
        let mut end = 0;
        let mut i = 0;
        while i < self.len() {
            let mut found = false;
            for sep in seps {
                if self[i..].starts_with(sep) {
                    end = i;
                    ret.push(&self[start..end]);
                    start = i + sep.len();
                    i = start;
                    found = true;
                    break;
                }
            }
            if !found {
                i += 1;
            }
        }
        ret.push(&self[start..]);
        ret
    }

    /// ```
    /// use erg_common::str::Str;
    /// let s = Str::rc("a.b.c");
    /// assert_eq!(s.rpartition_with(&[".", "/"]), ("a.b", "c"));
    /// let s = Str::rc("a::b.c");
    /// assert_eq!(s.rpartition_with(&["/", "::"]), ("a", "b.c"));
    /// ```
    pub fn rpartition_with(&self, seps: &[&str]) -> (&str, &str) {
        let mut i = self.len();
        while i > 0 {
            for sep in seps {
                if self[i..].starts_with(sep) {
                    return (&self[..i], &self[i + sep.len()..]);
                }
            }
            i -= 1;
        }
        (&self[..], "")
    }

    pub fn reversed(&self) -> Str {
        Str::rc(&self.chars().rev().collect::<String>())
    }

    pub fn multi_replace(&self, paths: &[(&str, &str)]) -> Self {
        let mut self_ = self.to_string();
        for (from, to) in paths {
            self_ = self_.replace(from, to);
        }
        Str::rc(&self_)
    }

    pub fn is_snake_case(&self) -> bool {
        self.chars().all(|c| !c.is_uppercase())
    }

    pub fn to_snake_case(&self) -> Str {
        let mut ret = String::new();
        let mut prev = '_';
        for c in self.chars() {
            if c.is_ascii_uppercase() {
                if prev != '_' {
                    ret.push('_');
                }
                ret.push(c.to_ascii_lowercase());
            } else {
                ret.push(c);
            }
            prev = c;
        }
        Str::rc(&ret)
    }

    pub fn find_sub<'a>(&self, pats: &[&'a str]) -> Option<&'a str> {
        pats.iter().find(|&&pat| self.contains(pat)).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_with() {
        assert_eq!(
            Str::ever("aa::bb.cc").split_with(&[".", "::"]),
            vec!["aa", "bb", "cc"]
        );
        assert_eq!(
            Str::ever("aa::bb.cc").split_with(&["::", "."]),
            vec!["aa", "bb", "cc"]
        );
        assert_eq!(
            Str::ever("aaxxbbyycc").split_with(&["xx", "yy"]),
            vec!["aa", "bb", "cc"]
        );
        assert_ne!(
            Str::ever("aaxxbbyycc").split_with(&["xx", "yy"]),
            vec!["aa", "bb", "ff"]
        );
    }
}
