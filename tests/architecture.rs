use std::{fs, path::Path};

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
        "gramwave-application",
    ] {
        assert!(!domain.contains(forbidden), "domain depends on {forbidden}");
    }
    for forbidden in ["eframe", "rusqlite", "tdlib-rs", "libloading"] {
        assert!(
            !application.contains(forbidden),
            "application depends on {forbidden}"
        );
    }
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = vec![src.join("screens.rs"), src.join("screens")];
    while let Some(path) = paths.pop() {
        let name = path.file_name().unwrap().to_string_lossy();
        if name == "tests" || name == "tests.rs" || name.ends_with("_tests.rs") {
            continue;
        }
        if path.is_dir() {
            paths.extend(
                fs::read_dir(path)
                    .unwrap()
                    .map(|entry| entry.unwrap().path()),
            );
        } else if name.ends_with(".rs") {
            let source = fs::read_to_string(&path).unwrap();
            for forbidden in ["std::fs", "std::path", "serde_json", ".call_async("] {
                assert!(
                    !source.contains(forbidden),
                    "{} contains {forbidden}",
                    path.display()
                );
            }
        }
    }
}
