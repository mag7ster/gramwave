#[test]
fn domain_has_no_infrastructure_dependencies() {
    for (name, source) in [
        ("model", include_str!("../crates/domain/src/model.rs")),
        ("playback", include_str!("../crates/domain/src/playback.rs")),
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

#[test]
fn workspace_enforces_dependency_direction_and_screens_have_no_io() {
    let domain = include_str!("../crates/domain/Cargo.toml");
    let application = include_str!("../crates/application/Cargo.toml");
    for forbidden in [
        "eframe",
        "rusqlite",
        "tdlib-rs",
        "libloading",
        "tg-player-application",
    ] {
        assert!(!domain.contains(forbidden), "domain depends on {forbidden}");
    }
    for forbidden in ["eframe", "rusqlite", "tdlib-rs", "libloading"] {
        assert!(
            !application.contains(forbidden),
            "application depends on {forbidden}"
        );
    }
    let screens = include_str!("../src/screens.rs");
    for forbidden in ["std::fs", "std::path", "serde_json", ".call_async("] {
        assert!(!screens.contains(forbidden), "screens contain {forbidden}");
    }
}
