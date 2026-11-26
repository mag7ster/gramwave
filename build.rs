fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    fastframe_i18n::build::compile_catalogs("assets/i18n");
    let credentials = if cfg!(feature = "telegram") {
        telegram_credentials()
    } else {
        "None".into()
    };
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out.join("telegram_credentials.rs"), credentials)
        .expect("write generated Telegram credentials");
    #[cfg(feature = "telegram")]
    telegram();
}

fn telegram_credentials() -> String {
    const CONFIG: &str = "GRAMWAVE_TELEGRAM_CONFIG";
    println!("cargo:rerun-if-env-changed={CONFIG}");
    let Some(path) = std::env::var_os(CONFIG) else {
        return "None".into();
    };
    let path = std::path::PathBuf::from(path);
    println!("cargo:rerun-if-changed={}", path.display());
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("Cannot read {CONFIG} file {}: {error}", path.display()));
    let mut fields = contents.split_whitespace();
    let id = fields.next().and_then(|value| value.parse::<i32>().ok());
    let hash = fields.next();
    match (id, hash, fields.next()) {
        (Some(id), Some(hash), None)
            if id > 0 && hash.len() == 32 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
        {
            format!("Some(({id}, {hash:?}))")
        }
        _ => panic!(
            "Invalid {CONFIG} file. Supply a positive API ID and a 32-digit hexadecimal API hash."
        ),
    }
}

#[cfg(feature = "telegram")]
fn telegram() {
    println!("cargo:rerun-if-env-changed=GRAMWAVE_TDLIB_DIR");
    let local = std::env::var_os("GRAMWAVE_TDLIB_DIR").map(std::path::PathBuf::from);
    assert!(
        local.is_some() || cfg!(feature = "download-tdlib"),
        "Set GRAMWAVE_TDLIB_DIR to a prefix containing lib/libtdjson.so.1.8.61"
    );
    let lib = local
        .as_ref()
        .map(|path| path.join("lib"))
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("tdlib/lib")
        });
    let tdjson = lib.join("libtdjson.so.1.8.61");
    #[cfg(feature = "download-tdlib")]
    if local.is_none() && !tdjson.is_file() {
        tdlib_rs::build::build(None);
    }
    assert!(
        tdjson.is_file(),
        "TDLib library not found at {}",
        tdjson.display()
    );
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
    let release = cfg!(feature = "release-branding");
    let profile = if release { "gramwave" } else { "gramwave-dev" };
    let native = base.join(profile).join("native");
    println!("cargo:rerun-if-env-changed=XDG_DATA_HOME");
    println!("cargo:rerun-if-env-changed=HOME");
    if !release || native.is_dir() {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{}", native.display());
    }
}
