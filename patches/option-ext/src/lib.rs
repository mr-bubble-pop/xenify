//! Stand-in for `option-ext` (see `patches/README.md`): the same trait and
//! method names, written for Xenify.

/// Extra methods on `Option`.
pub trait OptionExt<T> {
    /// Whether the option holds a value equal to `x`.
    #[must_use]
    fn contains<U>(&self, x: &U) -> bool
    where
        U: PartialEq<T>;

    /// `Option::map_or` with the closure first.
    #[must_use]
    fn map_or2<U, F: FnOnce(T) -> U>(self, f: F, default: U) -> U;

    /// `Option::map_or_else` with the closure first.
    #[must_use]
    fn map_or_else2<U, F: FnOnce(T) -> U, D: FnOnce() -> U>(self, f: F, default: D) -> U;
}

impl<T> OptionExt<T> for Option<T> {
    fn contains<U>(&self, x: &U) -> bool
    where
        U: PartialEq<T>,
    {
        self.as_ref().is_some_and(|v| x == v)
    }

    fn map_or2<U, F: FnOnce(T) -> U>(self, f: F, default: U) -> U {
        self.map_or(default, f)
    }

    fn map_or_else2<U, F: FnOnce(T) -> U, D: FnOnce() -> U>(self, f: F, default: D) -> U {
        self.map_or_else(default, f)
    }
}

#[cfg(test)]
mod tests {
    use super::OptionExt;

    #[test]
    fn helpers() {
        assert!(Some("a").contains(&"a"));
        assert!(!Some("a").contains(&"b"));
        assert!(!None::<&str>.contains(&"a"));
        assert_eq!(Some(2).map_or2(|v| v * 2, 0), 4);
        assert_eq!(None::<i32>.map_or_else2(|v| v * 2, || 7), 7);
    }
}
