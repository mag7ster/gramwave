fn main() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    bindgen::Builder::default()
        .header(root.join("vendor/mpv/client.h").to_string_lossy())
        .allowlist_type("mpv_event|mpv_event_end_file|mpv_event_start_file")
        .generate_comments(false)
        .layout_tests(false)
        .generate()
        .expect("generate pinned mpv ABI")
        .write_to_file(root.join("src/adapters/mpv_abi.rs"))
        .expect("write ABI");
}
