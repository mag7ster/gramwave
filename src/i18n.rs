//! English is the source language. Catalogs are compiled by fastframe, as in Spotifast.
include!(concat!(env!("OUT_DIR"), "/catalogs.rs"));
#[derive(Clone, Copy)]
pub enum Locale {
    English,
}
impl fastframe_i18n::Locale for Locale {
    fn catalog(self) -> Option<&'static dyn fastframe_i18n::Translator> {
        None
    }
}
pub fn tr(source: &'static str) -> String {
    fastframe_i18n::gettext(Locale::English, source).into_owned()
}
pub fn track_count(count: usize) -> String {
    fastframe_i18n::ngettext(
        Locale::English,
        "{count} track",
        "{count} tracks",
        count.min(u32::MAX as usize) as u32,
    )
    .replace("{count}", &count.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn english_is_fallback() {
        assert_eq!(super::tr("Library"), "Library");
    }
}
