use std::process::Command;

#[test]
fn brainmap_help_binary_builds() {
    let output = Command::new(env!("CARGO_BIN_EXE_brainmap"))
        .arg("--help")
        .output()
        .expect("run brainmap --help");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Local deterministic personal decision engine"));
}
