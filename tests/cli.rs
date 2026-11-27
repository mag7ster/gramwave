use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_gramwave-dev"))
        .args(args)
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .unwrap()
}

#[test]
fn help_is_plain_text_and_explains_playback_and_startup() {
    for (args, detail) in [
        (&["--help"][..], "--headless"),
        (&["--ctl", "--json", "--help"], "--ctl quit"),
        (&["--ctl", "quit", "--help"], "wait for the player to exit"),
        (&["--ctl", "select", "--help"], "starting at 1"),
    ] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(0));
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(
            stdout.contains("Usage:") && stdout.contains(detail),
            "{stdout}"
        );
    }

    let output = run(&["--ctl"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("search Jazz") && stderr.contains("--ctl quit"),
        "{stderr}"
    );
}

#[test]
fn invalid_arguments_are_text_unless_json_is_requested() {
    let output = run(&["--ctl", "select", "0"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("0"));

    for args in [
        &["--ctl", "--json", "select", "0"][..],
        &["--ctl", "select", "0", "--json"],
        &["--ctl", "--json"],
    ] {
        let output = run(args);
        assert_eq!(output.status.code(), Some(1), "{args:?}");
        assert!(output.stderr.is_empty());
        let reply: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(reply["ok"], false);
        assert!(reply["error"].is_string());
    }
}
