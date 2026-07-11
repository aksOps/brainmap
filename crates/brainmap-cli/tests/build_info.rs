use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;

const BUILD_INFO_SCHEMA: &str = "brainmap-build-info-v1";
const NONQUALIFYING_MARKER: &str = "nonqualifying";

#[test]
fn normal_brainmap_binaries_expose_identical_nonqualifying_build_info() {
    let brainmap = build_info(Path::new(env!("CARGO_BIN_EXE_brainmap")));
    let brainmapd = build_info(Path::new(env!("CARGO_BIN_EXE_brainmapd")));

    assert_eq!(brainmap, brainmapd);
    assert_eq!(brainmap["schemaVersion"], BUILD_INFO_SCHEMA);
    assert_eq!(brainmap["candidateCommit"], git_head());
    assert!(
        brainmap["cargoProfile"]
            .as_str()
            .is_some_and(|profile| !profile.is_empty())
    );
    assert_eq!(brainmap["qualification"]["eligible"], false);
    assert_eq!(brainmap["qualification"]["marker"], NONQUALIFYING_MARKER);
    assert_eq!(brainmap["qualification"]["release"], false);
    assert_eq!(brainmap["qualification"]["locked"], false);
    assert_eq!(brainmap["qualification"]["twoRootCandidate"], false);

    let api = serde_json::to_value(brainmap_cli::build_info::build_info())
        .expect("serialize public build info API");
    assert_eq!(brainmap, api);

    for (field, script) in [
        (
            "integratedQualificationSha256",
            "scripts/m8-integrated-qualification.sh",
        ),
        ("codexFia5Sha256", "scripts/m8-codex-fia5.sh"),
        (
            "releaseQualificationSha256",
            "scripts/m8-release-qualification.sh",
        ),
        (
            "assembleQualificationSha256",
            "scripts/m8-assemble-qualification.sh",
        ),
    ] {
        assert_eq!(
            brainmap["producerDigests"][field],
            sha256_file(&workspace_root().join(script)),
            "embedded producer digest differs for {script}"
        );
    }
}

fn build_info(binary: &Path) -> serde_json::Value {
    let output = Command::new(binary)
        .arg("build-info")
        .output()
        .expect("run build-info command");
    assert!(
        output.status.success(),
        "build-info failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "build-info wrote stderr");
    serde_json::from_slice(&output.stdout).expect("parse build-info JSON")
}

fn git_head() -> String {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace_root())
        .output()
        .expect("resolve Git HEAD");
    assert!(output.status.success(), "git rev-parse HEAD failed");
    String::from_utf8(output.stdout)
        .expect("Git HEAD is UTF-8")
        .trim()
        .to_owned()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("brainmap-cli lives under crates/")
        .to_path_buf()
}

fn sha256_file(path: &Path) -> String {
    let bytes = std::fs::read(path).expect("read producer script");
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}
