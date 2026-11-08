use super::*;
#[cfg(test)]
mod regression {
    #[test]
    fn english_is_fallback() {
        assert_eq!(super::tr("Library"), "Library");
    }
}
