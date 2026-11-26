pub(crate) const IS_RELEASE: bool = cfg!(feature = "release-branding");
pub(crate) const APP_NAME: &str = if IS_RELEASE {
    "Gramwave"
} else {
    "Gramwave Dev"
};
pub(crate) const APP_ID: &str = if IS_RELEASE {
    "gramwave"
} else {
    "gramwave-dev"
};
