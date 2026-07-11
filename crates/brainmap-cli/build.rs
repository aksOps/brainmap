use sha2::{Digest, Sha256};
use std::env;
use std::error::Error;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const MODEL_ID: &str = "minishlab/potion-base-8M";
const MODEL_BASE_URL: &str = "https://huggingface.co/minishlab/potion-base-8M/resolve/main";
const MODEL_DIR: &str = "potion-base-8M";
const UNKNOWN_COMMIT: &str = "0000000000000000000000000000000000000000";
const QUALIFICATION_MARKER: &str = "brainmap-clean-locked-two-root-v1";
const NONQUALIFYING_MARKER: &str = "nonqualifying";
const INTERNAL_MARKER_ENV: &str = "BRAINMAP_INTERNAL_QUALIFICATION_MARKER";
const INTERNAL_COMMIT_ENV: &str = "BRAINMAP_INTERNAL_CANDIDATE_COMMIT";
const INTERNAL_CLEAN_ENV: &str = "BRAINMAP_INTERNAL_SOURCE_CLEAN";
const INTERNAL_LOCKED_ENV: &str = "BRAINMAP_INTERNAL_LOCKED";
const INTERNAL_TWO_ROOT_ENV: &str = "BRAINMAP_INTERNAL_TWO_ROOT_CANDIDATE";

const PRODUCER_SCRIPTS: [(&str, &str); 4] = [
    (
        "BRAINMAP_M8_INTEGRATED_QUALIFICATION_SHA256",
        "scripts/m8-integrated-qualification.sh",
    ),
    ("BRAINMAP_M8_CODEX_FIA5_SHA256", "scripts/m8-codex-fia5.sh"),
    (
        "BRAINMAP_M8_RELEASE_QUALIFICATION_SHA256",
        "scripts/m8-release-qualification.sh",
    ),
    (
        "BRAINMAP_M8_ASSEMBLE_QUALIFICATION_SHA256",
        "scripts/m8-assemble-qualification.sh",
    ),
];

struct ModelFile {
    path: &'static str,
    sha256: &'static str,
    size: u64,
}

const MODEL_FILES: &[ModelFile] = &[
    ModelFile {
        path: ".gitattributes",
        sha256: "11ad7efa24975ee4b0c3c3a38ed18737f0658a5f75a0a96787b576a78a023361",
        size: 1519,
    },
    ModelFile {
        path: "README.md",
        sha256: "de8ec91bf63c5f4c0e20751c227b2d049953e1cab5f8d5d44211c59a44795bdd",
        size: 5203,
    },
    ModelFile {
        path: "config.json",
        sha256: "2a6ac0e9aaa356a68a5688070db78fc3a464fefe85d2f06a1905ce3718687553",
        size: 202,
    },
    ModelFile {
        path: "model.safetensors",
        sha256: "f65d0f325faadc1e121c319e2faa41170d3fa07d8c89abd48ca5358d9a223de2",
        size: 30_236_760,
    },
    ModelFile {
        path: "modules.json",
        sha256: "a68dcbed0429dcdd5bfdca92b0b03cc30d09122c0a3fcf4758787d4b244e45b2",
        size: 278,
    },
    ModelFile {
        path: "special_tokens_map.json",
        sha256: "a9e8fb6f99fb0b8803f0e6942fdf4d95d6645204620b67dc3310a1024bcbac59",
        size: 134,
    },
    ModelFile {
        path: "tokenizer.json",
        sha256: "e67e803f624fb4d67dea1c730d06e1067e1b14d830e2c2202569e3ef0f70bb50",
        size: 683_666,
    },
    ModelFile {
        path: "tokenizer_config.json",
        sha256: "6725995e3ab3039857ff5bd99178a7cdf42863abb04449e7bb31feb1f55fe567",
        size: 1431,
    },
    ModelFile {
        path: "vocab.txt",
        sha256: "1394523a67ddd404a825428018c0582a6998bcfa044ecbcbf1f4d71adb94c61c",
        size: 219_690,
    },
];

fn main() -> Result<(), Box<dyn Error>> {
    println!("cargo:rerun-if-changed=build.rs");
    emit_build_provenance()?;
    let out_dir = PathBuf::from(env::var("OUT_DIR")?);
    let model_dir = out_dir.join("model-download").join(MODEL_DIR);
    fs::create_dir_all(&model_dir)?;

    for file in MODEL_FILES {
        let dest = model_dir.join(file.path);
        if verify_file(&dest, file).is_err() {
            download_file(file, &dest)?;
            verify_file(&dest, file)?;
        }
    }

    let manifest = model_manifest();
    fs::write(model_dir.join("model-manifest.json"), manifest)?;

    let pack = out_dir.join("default.brainmap-model.tar.zst");
    write_pack(&model_dir, &pack)?;
    let pack_bytes = fs::read(&pack)?;
    println!(
        "cargo:rustc-env=BRAINMAP_MODEL_PACK_SHA256={}",
        sha256_hex(&pack_bytes)
    );
    println!(
        "cargo:rustc-env=BRAINMAP_MODEL_PACK_LEN={}",
        pack_bytes.len()
    );
    Ok(())
}

fn emit_build_provenance() -> Result<(), Box<dyn Error>> {
    for variable in [
        INTERNAL_MARKER_ENV,
        INTERNAL_COMMIT_ENV,
        INTERNAL_CLEAN_ENV,
        INTERNAL_LOCKED_ENV,
        INTERNAL_TWO_ROOT_ENV,
        "PROFILE",
    ] {
        println!("cargo:rerun-if-env-changed={variable}");
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let workspace_root = manifest_dir
        .parent()
        .and_then(Path::parent)
        .ok_or("brainmap-cli package must live under the workspace crates directory")?;
    emit_git_reruns(workspace_root);

    let profile = env::var("PROFILE").unwrap_or_else(|_| "unknown".to_owned());
    let supplied_commit = env::var(INTERNAL_COMMIT_ENV).unwrap_or_default();
    let qualifying = profile == "release"
        && env::var(INTERNAL_MARKER_ENV).as_deref() == Ok(QUALIFICATION_MARKER)
        && is_full_commit(&supplied_commit)
        && env::var(INTERNAL_CLEAN_ENV).as_deref() == Ok("true")
        && env::var(INTERNAL_LOCKED_ENV).as_deref() == Ok("true")
        && env::var(INTERNAL_TWO_ROOT_ENV).as_deref() == Ok("true");

    let candidate_commit = if qualifying {
        supplied_commit
    } else {
        git_head(workspace_root).unwrap_or_else(|| UNKNOWN_COMMIT.to_owned())
    };
    let marker = if qualifying {
        QUALIFICATION_MARKER
    } else {
        NONQUALIFYING_MARKER
    };

    println!("cargo:rustc-env=BRAINMAP_BUILD_CANDIDATE_COMMIT={candidate_commit}");
    println!("cargo:rustc-env=BRAINMAP_BUILD_CARGO_PROFILE={profile}");
    println!("cargo:rustc-env=BRAINMAP_BUILD_QUALIFICATION_MARKER={marker}");
    println!("cargo:rustc-env=BRAINMAP_BUILD_QUALIFICATION_ELIGIBLE={qualifying}");
    println!("cargo:rustc-env=BRAINMAP_BUILD_QUALIFICATION_RELEASE={qualifying}");
    println!("cargo:rustc-env=BRAINMAP_BUILD_QUALIFICATION_LOCKED={qualifying}");
    println!("cargo:rustc-env=BRAINMAP_BUILD_TWO_ROOT_CANDIDATE={qualifying}");

    for (rustc_env, relative) in PRODUCER_SCRIPTS {
        let path = workspace_root.join(relative);
        println!("cargo:rerun-if-changed={}", path.display());
        let digest = match fs::read(&path) {
            Ok(bytes) => sha256_hex(&bytes),
            Err(error) if !qualifying => {
                println!(
                    "cargo:warning=producer script unavailable for nonqualifying build: {} ({error})",
                    path.display()
                );
                "0".repeat(64)
            }
            Err(error) => {
                return Err(format!(
                    "qualifying build requires producer script {}: {error}",
                    path.display()
                )
                .into());
            }
        };
        println!("cargo:rustc-env={rustc_env}={digest}");
    }
    Ok(())
}

fn is_full_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn git_head(workspace_root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(workspace_root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let commit = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    is_full_commit(&commit).then_some(commit)
}

fn emit_git_reruns(workspace_root: &Path) {
    let Some(git_dir) = git_path(workspace_root, "--absolute-git-dir") else {
        return;
    };
    let head = PathBuf::from(&git_dir).join("HEAD");
    println!("cargo:rerun-if-changed={}", head.display());
    let Ok(contents) = fs::read_to_string(&head) else {
        return;
    };
    let Some(reference) = contents.trim().strip_prefix("ref: ") else {
        return;
    };
    let Some(common_dir) = git_path(workspace_root, "--git-common-dir") else {
        return;
    };
    let common_dir = if Path::new(&common_dir).is_absolute() {
        PathBuf::from(common_dir)
    } else {
        workspace_root.join(common_dir)
    };
    println!(
        "cargo:rerun-if-changed={}",
        common_dir.join(reference).display()
    );
}

fn git_path(workspace_root: &Path, argument: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(workspace_root)
        .args(["rev-parse", argument])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn download_file(file: &ModelFile, dest: &Path) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let url = format!("{MODEL_BASE_URL}/{}", file.path);
    let status = Command::new("curl")
        .args(["-L", "--fail", "--retry", "3", "--retry-delay", "2", "-o"])
        .arg(dest)
        .arg(&url)
        .status()?;
    if !status.success() {
        return Err(format!("download failed: {url}").into());
    }
    Ok(())
}

fn verify_file(path: &Path, expected: &ModelFile) -> Result<(), Box<dyn Error>> {
    let bytes = fs::read(path)?;
    if bytes.len() as u64 != expected.size {
        return Err(format!("size mismatch for {}", expected.path).into());
    }
    let got = sha256_hex(&bytes);
    if got != expected.sha256 {
        return Err(format!("sha256 mismatch for {}", expected.path).into());
    }
    Ok(())
}

fn model_manifest() -> String {
    let mut files = String::new();
    for (idx, file) in MODEL_FILES.iter().enumerate() {
        if idx > 0 {
            files.push_str(",\n");
        }
        files.push_str(&format!(
            r#"    {{"path": "{}", "sha256": "{}", "size": {}}}"#,
            file.path, file.sha256, file.size
        ));
    }
    format!(
        r#"{{
  "modelId": "{MODEL_ID}",
  "format": "model2vec",
  "dimension": 256,
  "source": "https://huggingface.co/{MODEL_ID}",
  "license": "MIT",
  "packKind": "build-time-download",
  "files": [
{files}
  ]
}}
"#
    )
}

fn write_pack(model_dir: &Path, pack: &Path) -> Result<(), Box<dyn Error>> {
    let file = File::create(pack)?;
    let encoder = zstd::Encoder::new(file, 3)?;
    let mut tar = tar::Builder::new(encoder);
    append_dir(&mut tar, MODEL_DIR)?;
    for file in MODEL_FILES {
        append_file(
            &mut tar,
            &model_dir.join(file.path),
            &format!("{MODEL_DIR}/{}", file.path),
            file.size,
        )?;
    }
    let manifest_path = model_dir.join("model-manifest.json");
    let manifest_size = fs::metadata(&manifest_path)?.len();
    append_file(
        &mut tar,
        &manifest_path,
        &format!("{MODEL_DIR}/model-manifest.json"),
        manifest_size,
    )?;
    let encoder = tar.into_inner()?;
    encoder.finish()?;
    Ok(())
}

fn append_dir<W: io::Write>(tar: &mut tar::Builder<W>, path: &str) -> Result<(), Box<dyn Error>> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Directory);
    header.set_size(0);
    header.set_mode(0o755);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    tar.append_data(&mut header, format!("{path}/"), io::empty())?;
    Ok(())
}

fn append_file<W: io::Write>(
    tar: &mut tar::Builder<W>,
    source: &Path,
    archive_path: &str,
    size: u64,
) -> Result<(), Box<dyn Error>> {
    let mut header = tar::Header::new_gnu();
    header.set_size(size);
    header.set_mode(0o644);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    tar.append_data(&mut header, archive_path, File::open(source)?)?;
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
