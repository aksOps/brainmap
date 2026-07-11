use anyhow::Result;
use serde::Serialize;
use std::io::Write;

pub const SCHEMA_VERSION: &str = "brainmap-build-info-v1";
pub const QUALIFICATION_MARKER: &str = "brainmap-clean-locked-two-root-v1";
pub const NONQUALIFYING_MARKER: &str = "nonqualifying";

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    pub schema_version: &'static str,
    pub candidate_commit: &'static str,
    pub cargo_profile: &'static str,
    pub qualification: BuildQualification,
    pub producer_digests: ProducerDigests,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildQualification {
    pub eligible: bool,
    pub marker: &'static str,
    pub release: bool,
    pub locked: bool,
    pub two_root_candidate: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProducerDigests {
    pub integrated_qualification_sha256: &'static str,
    pub codex_fia5_sha256: &'static str,
    pub release_qualification_sha256: &'static str,
    pub assemble_qualification_sha256: &'static str,
}

#[must_use]
pub fn build_info() -> BuildInfo {
    BuildInfo {
        schema_version: SCHEMA_VERSION,
        candidate_commit: env!("BRAINMAP_BUILD_CANDIDATE_COMMIT"),
        cargo_profile: env!("BRAINMAP_BUILD_CARGO_PROFILE"),
        qualification: BuildQualification {
            eligible: env!("BRAINMAP_BUILD_QUALIFICATION_ELIGIBLE") == "true",
            marker: env!("BRAINMAP_BUILD_QUALIFICATION_MARKER"),
            release: env!("BRAINMAP_BUILD_QUALIFICATION_RELEASE") == "true",
            locked: env!("BRAINMAP_BUILD_QUALIFICATION_LOCKED") == "true",
            two_root_candidate: env!("BRAINMAP_BUILD_TWO_ROOT_CANDIDATE") == "true",
        },
        producer_digests: ProducerDigests {
            integrated_qualification_sha256: env!("BRAINMAP_M8_INTEGRATED_QUALIFICATION_SHA256"),
            codex_fia5_sha256: env!("BRAINMAP_M8_CODEX_FIA5_SHA256"),
            release_qualification_sha256: env!("BRAINMAP_M8_RELEASE_QUALIFICATION_SHA256"),
            assemble_qualification_sha256: env!("BRAINMAP_M8_ASSEMBLE_QUALIFICATION_SHA256"),
        },
    }
}

pub fn build_info_json() -> Result<String> {
    Ok(serde_json::to_string(&build_info())?)
}

pub fn print_build_info() -> Result<()> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    output.write_all(build_info_json()?.as_bytes())?;
    output.write_all(b"\n")?;
    Ok(())
}
