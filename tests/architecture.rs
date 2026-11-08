#[test]
fn domain_has_no_infrastructure_dependencies() {
    for (name, source) in [
        ("model", include_str!("../src/domain/model.rs")),
        ("playback", include_str!("../src/domain/playback.rs")),
    ] {
        for forbidden in [
            "egui",
            "eframe",
            "rusqlite",
            "serde_json::Value",
            "fastframe",
            "std::fs",
            "std::net",
            "crate::adapters",
            "crate::application",
        ] {
            assert!(!source.contains(forbidden), "{name} depends on {forbidden}");
        }
    }
}
