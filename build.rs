fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    fastframe_i18n::build::compile_catalogs("assets/i18n");
    #[cfg(feature = "telegram")]
    telegram();
}
#[cfg(feature = "telegram")]
fn telegram() {
    let lib = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("tdlib/lib");
    if !lib.join("libtdjson.so.1.8.61").exists() {
        tdlib_rs::build::build(None);
    }
    println!("cargo:rustc-link-search=native={}", lib.display());
    println!("cargo:rustc-link-lib=dylib=tdjson");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", lib.display());
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/lib");
    println!("cargo:rustc-link-arg=-Wl,--disable-new-dtags");
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join(".local/share")
        });
    let native = base.join("tg-player/native");
    if native.is_dir() {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", native.display());
    }
}
