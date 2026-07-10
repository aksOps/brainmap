use crate::{gate, index, learning, skill, util, vault};
use anyhow::{Context, Result, bail};
use clap::Args;
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;

#[derive(Args)]
pub struct InstallHarnessArgs {
    #[arg(long)]
    pub target: String,
    #[arg(long)]
    pub global: bool,
    #[arg(long)]
    pub project: Option<PathBuf>,
    #[arg(long)]
    pub vault: Option<PathBuf>,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub uninstall: bool,
}

pub fn install_harness(args: InstallHarnessArgs) -> Result<()> {
    let plan = plan(&args);
    if args.dry_run {
        println!("install harness dry-run target={}", args.target);
        for item in &plan {
            println!("{} ({})", item.path.display(), item.enforcement);
        }
        return Ok(());
    }
    for item in plan {
        if args.uninstall {
            item.uninstall()?;
        } else {
            item.install()?;
        }
    }
    Ok(())
}

pub fn integration_doctor(args: crate::cli::IntegrationDoctorArgs) -> Result<()> {
    let supported = matches!(
        args.target.as_str(),
        "codex" | "claude-code" | "opencode" | "copilot" | "generic-stdio"
    );
    let install_args = InstallHarnessArgs {
        target: args.target.clone(),
        global: false,
        project: args.project.clone(),
        vault: args.vault.clone(),
        dry_run: true,
        uninstall: false,
    };
    let planned = plan(&install_args);
    let root = vault::resolve_vault(args.vault);
    let installed = supported && planned.iter().all(|item| item.path.exists());
    let configuration_valid = planned.iter().all(|item| {
        if !item.path.exists() {
            return true;
        }
        if item.path.extension().and_then(|value| value.to_str()) == Some("json") {
            return fs::read(&item.path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .is_some();
        }
        if item.path.ends_with(".codex/config.toml") {
            return fs::read_to_string(&item.path)
                .map(|text| codex_mcp_config_status(&text, &root).valid)
                .unwrap_or(false);
        }
        true
    });
    let contract = planned
        .iter()
        .filter_map(|item| fs::read_to_string(&item.path).ok())
        .collect::<Vec<_>>()
        .join("\n");
    let learning_probe = probe_learning_lifecycle().unwrap_or_default();
    let recording_supported =
        contract.contains("record-decision") && learning_probe.recording_works;
    let feedback_supported = contract.contains("learn-feedback") && learning_probe.feedback_works;
    let activation_requires_approval = contract.contains("apply --pending --yes")
        && learning_probe.preview_works
        && learning_probe.approved_apply_changes_decision;
    let executable = std::env::current_exe().is_ok_and(|path| path.exists());
    let mcp_vault_configured = args.target != "codex"
        || planned
            .iter()
            .find(|item| item.path.ends_with(".codex/config.toml"))
            .and_then(|item| fs::read_to_string(&item.path).ok())
            .is_some_and(|text| codex_mcp_config_status(&text, &root).vault_matches);
    let vault_exists = root.exists();
    let index_status = index::status(&root).ok();
    let index_valid = index_status.as_ref().is_some_and(|status| status.valid);
    let gate_reachable = index_valid
        && gate::evaluate(
            &root,
            gate::GateInput {
                intent: "integration-doctor".into(),
                situation: "Choose v1 storage".into(),
                options: vec!["Markdown+JSONL".into(), "External Vector DB".into()],
                proposed_action: String::new(),
                risk: "low".into(),
                reversible: Some(true),
                decision_type: "architecture".into(),
                scope: "global".into(),
                agent_confidence: None,
                dry_run: true,
            },
        )
        .is_ok();
    let enforcement = planned
        .iter()
        .map(|item| item.enforcement)
        .collect::<Vec<_>>();
    let healthy = supported
        && installed
        && configuration_valid
        && executable
        && vault_exists
        && index_valid
        && gate_reachable
        && recording_supported
        && feedback_supported
        && activation_requires_approval;
    let healthy = healthy && mcp_vault_configured;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "target": args.target,
            "supported": supported,
            "installed": installed,
            "configurationValid": configuration_valid,
            "executableAvailable": executable,
            "vaultExists": vault_exists,
            "indexValid": index_valid,
            "gateReachable": gate_reachable,
            "recordingSupported": recording_supported,
            "feedbackSupported": feedback_supported,
            "activationRequiresApproval": activation_requires_approval,
            "mcpVaultConfigured": mcp_vault_configured,
            "enforcement": enforcement,
            "healthy": healthy,
        }))?
    );
    if !healthy {
        let mut issues = Vec::new();
        if !supported {
            issues.push("unsupported target");
        }
        if !installed {
            issues.push("adapter files missing");
        }
        if !configuration_valid {
            issues.push("invalid host configuration");
        }
        if !executable {
            issues.push("brainmap executable unavailable");
        }
        if !vault_exists {
            issues.push("vault missing");
        } else if !index_valid {
            issues.push("compiled index missing or invalid");
        }
        if !gate_reachable {
            issues.push("decision gate unhealthy");
        }
        if !recording_supported {
            issues.push("recording contract missing");
        }
        if !feedback_supported {
            issues.push("feedback contract missing");
        }
        if !activation_requires_approval {
            issues.push("explicit activation approval missing");
        }
        if !mcp_vault_configured {
            issues.push("Codex MCP vault path does not match the requested vault");
        }
        bail!("integration doctor unhealthy: {}", issues.join(", "));
    }
    Ok(())
}

#[derive(Default)]
struct LearningLifecycleProbe {
    recording_works: bool,
    feedback_works: bool,
    preview_works: bool,
    approved_apply_changes_decision: bool,
}

fn probe_learning_lifecycle() -> Result<LearningLifecycleProbe> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().join("BrainMap");
    vault::init_vault_quiet(Some(root.clone()), true)?;
    index::rebuild(&root)?;
    let request = |dry_run| gate::GateInput {
        intent: "integration-doctor".into(),
        situation: "Choose package manager for integration doctor".into(),
        options: vec!["npm".into(), "pnpm".into()],
        proposed_action: String::new(),
        risk: "low".into(),
        reversible: Some(true),
        decision_type: "tooling".into(),
        scope: "project:integration-doctor".into(),
        agent_confidence: None,
        dry_run,
    };
    let initial = gate::evaluate(&root, request(false))?;
    learning::record_decision_quiet(crate::cli::RecordDecisionArgs {
        decision_id: Some(initial.decision_id.clone()),
        chosen: Some("pnpm".into()),
        was_asked: Some(true),
        vault: Some(root.clone()),
    })?;
    let packet_id = learning::learn_feedback_quiet(crate::cli::LearnFeedbackArgs {
        decision_id: initial.decision_id,
        correction: None,
        chosen: Some("pnpm".into()),
        rejected: Some("npm".into()),
        incident: None,
        vault: Some(root.clone()),
    })?
    .context("integration learning probe did not create a packet")?;
    let preview = learning::pending_updates_value(&root, Some(&packet_id))?;
    learning::apply_update_by_id(&root, &packet_id)?;
    let changed = gate::evaluate(&root, request(true))?;
    Ok(LearningLifecycleProbe {
        recording_works: true,
        feedback_works: true,
        preview_works: preview.as_array().is_some_and(|packets| packets.len() == 1),
        approved_apply_changes_decision: changed.selected_option.as_deref() == Some("pnpm"),
    })
}

struct PlanItem {
    path: PathBuf,
    enforcement: &'static str,
    action: PlanAction,
}

enum PlanAction {
    OwnedText(String),
    ManagedText(String),
    ManagedToml(String),
    JsonHooks(Vec<HookBinding>),
    JsonInstruction(String),
}

#[derive(Clone)]
struct HookBinding {
    event: &'static str,
    matcher: Option<&'static str>,
    command: String,
    timeout_secs: u64,
}

impl PlanItem {
    #[cfg(test)]
    fn contents(&self) -> Result<String> {
        match &self.action {
            PlanAction::OwnedText(contents)
            | PlanAction::ManagedText(contents)
            | PlanAction::ManagedToml(contents) => Ok(contents.clone()),
            PlanAction::JsonHooks(bindings) => json_hooks_contents(&self.path, bindings, false),
            PlanAction::JsonInstruction(instruction) => {
                json_instruction_contents(&self.path, instruction, false)
            }
        }
    }

    fn install(&self) -> Result<()> {
        if let PlanAction::OwnedText(contents) = &self.action {
            if self.path.exists() {
                let existing = fs::read_to_string(&self.path)
                    .with_context(|| format!("read {}", self.path.display()))?;
                if existing != *contents {
                    bail!(
                        "refusing to overwrite unmanaged file {}; move it or remove it explicitly",
                        self.path.display()
                    );
                }
                println!("unchanged {} ({})", self.path.display(), self.enforcement);
                return Ok(());
            }
            util::write_atomic(&self.path, contents.as_bytes())?;
            println!("wrote {} ({})", self.path.display(), self.enforcement);
            return Ok(());
        }
        let contents = match &self.action {
            PlanAction::OwnedText(_) => unreachable!(),
            PlanAction::ManagedText(block) => {
                let existing = if self.path.exists() {
                    fs::read_to_string(&self.path)
                        .with_context(|| format!("read {}", self.path.display()))?
                } else {
                    String::new()
                };
                merge_managed_text(&existing, block, false)
            }
            PlanAction::ManagedToml(block) => {
                let existing = if self.path.exists() {
                    fs::read_to_string(&self.path)
                        .with_context(|| format!("read {}", self.path.display()))?
                } else {
                    String::new()
                };
                merge_managed_toml(&existing, block, false)?
            }
            PlanAction::JsonHooks(bindings) => json_hooks_contents(&self.path, bindings, false)?,
            PlanAction::JsonInstruction(instruction) => {
                json_instruction_contents(&self.path, instruction, false)?
            }
        };
        if self.path.exists()
            && fs::read_to_string(&self.path)
                .with_context(|| format!("read {}", self.path.display()))?
                == contents
        {
            println!("unchanged {} ({})", self.path.display(), self.enforcement);
            return Ok(());
        }
        if self.path.exists() {
            backup(&self.path)?;
        }
        util::write_atomic(&self.path, contents.as_bytes())?;
        println!("wrote {} ({})", self.path.display(), self.enforcement);
        Ok(())
    }

    fn uninstall(&self) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        match &self.action {
            PlanAction::OwnedText(contents) => {
                let existing = fs::read_to_string(&self.path)
                    .with_context(|| format!("read {}", self.path.display()))?;
                if existing != *contents {
                    bail!(
                        "refusing to remove unmanaged or modified file {}",
                        self.path.display()
                    );
                }
                backup(&self.path)?;
                fs::remove_file(&self.path)?;
                println!("removed {}", self.path.display());
            }
            PlanAction::ManagedText(block) => {
                backup(&self.path)?;
                let existing = fs::read_to_string(&self.path)?;
                let contents = merge_managed_text(&existing, block, true);
                if contents.is_empty() {
                    fs::remove_file(&self.path)?;
                    println!("removed {}", self.path.display());
                } else {
                    util::write_atomic(&self.path, contents.as_bytes())?;
                    println!("updated {} ({})", self.path.display(), self.enforcement);
                }
            }
            PlanAction::ManagedToml(block) => {
                backup(&self.path)?;
                let existing = fs::read_to_string(&self.path)?;
                let contents = merge_managed_toml(&existing, block, true)?;
                if contents.is_empty() {
                    fs::remove_file(&self.path)?;
                    println!("removed {}", self.path.display());
                } else {
                    util::write_atomic(&self.path, contents.as_bytes())?;
                    println!("updated {} ({})", self.path.display(), self.enforcement);
                }
            }
            PlanAction::JsonHooks(bindings) => {
                backup(&self.path)?;
                let contents = json_hooks_contents(&self.path, bindings, true)?;
                util::write_atomic(&self.path, contents.as_bytes())?;
                println!("updated {} ({})", self.path.display(), self.enforcement);
            }
            PlanAction::JsonInstruction(instruction) => {
                backup(&self.path)?;
                let contents = json_instruction_contents(&self.path, instruction, true)?;
                util::write_atomic(&self.path, contents.as_bytes())?;
                println!("updated {} ({})", self.path.display(), self.enforcement);
            }
        }
        Ok(())
    }
}

fn plan(args: &InstallHarnessArgs) -> Vec<PlanItem> {
    let base = if args.global {
        util::home_dir()
    } else {
        args.project.clone().unwrap_or_else(|| PathBuf::from("."))
    };
    match args.target.as_str() {
        "claude-code" => vec![
            PlanItem {
                path: base.join(".claude/skills/build-decision-engine/SKILL.md"),
                enforcement: "instruction-only",
                action: PlanAction::OwnedText(skill::build_decision_engine_shim("claude-code")),
            },
            PlanItem {
                path: base.join(".claude/settings.json"),
                enforcement: "enforced",
                action: PlanAction::JsonHooks(hook_bindings("claude-code")),
            },
        ],
        "codex" => vec![
            PlanItem {
                path: base.join(".codex/skills/build-decision-engine/SKILL.md"),
                enforcement: "instruction-only",
                action: PlanAction::OwnedText(skill::build_decision_engine_shim("codex")),
            },
            PlanItem {
                path: base.join("AGENTS.md"),
                enforcement: "instruction-only",
                action: PlanAction::ManagedText(managed_block("codex")),
            },
            PlanItem {
                path: base.join(".codex/config.toml"),
                enforcement: "best-effort",
                action: PlanAction::ManagedToml(codex_mcp_block(args.vault.as_deref())),
            },
            PlanItem {
                path: base.join(".codex/hooks.json"),
                enforcement: "enforced",
                action: PlanAction::JsonHooks(hook_bindings("codex")),
            },
        ],
        "opencode" => vec![PlanItem {
            path: base.join("opencode.json"),
            enforcement: "best-effort",
            action: PlanAction::JsonInstruction(managed_block("opencode")),
        }],
        "copilot" => vec![PlanItem {
            path: base.join(".github/copilot-instructions.md"),
            enforcement: "instruction-only",
            action: PlanAction::ManagedText(managed_block("copilot")),
        }],
        "generic-stdio" => vec![PlanItem {
            path: base.join("brainmap-harness.md"),
            enforcement: "enforced",
            action: PlanAction::OwnedText("Generic stdio harness can enforce with `brainmap harness stdio --fail-on-block`. Send one JSON gate request per line; read one gate JSON response per line.\n".into()),
        }],
        _ => vec![PlanItem {
            path: base.join("brainmap-harness-unsupported.txt"),
            enforcement: "instruction-only",
            action: PlanAction::OwnedText(format!(
                "Unsupported target {}; no install performed\n",
                args.target
            )),
        }],
    }
}

fn hook_bindings(host: &str) -> Vec<HookBinding> {
    vec![
        HookBinding {
            event: "UserPromptSubmit",
            matcher: None,
            command: brainmap_hook_command(host, "UserPromptSubmit"),
            timeout_secs: 10,
        },
        HookBinding {
            event: "PreToolUse",
            matcher: Some("Bash|Edit|Write|MultiEdit|NotebookEdit"),
            command: brainmap_hook_command(host, "PreToolUse"),
            timeout_secs: 10,
        },
    ]
}

fn brainmap_hook_command(host: &str, event: &str) -> String {
    format!("brainmap harness hook --host {host} --event {event}")
}

fn backup(path: &PathBuf) -> Result<()> {
    let backup = path.with_extension(format!("bak-{}", chrono::Utc::now().timestamp_micros()));
    fs::copy(path, &backup)
        .with_context(|| format!("backup {} to {}", path.display(), backup.display()))?;
    println!("backup {}", backup.display());
    Ok(())
}

fn json_instruction_contents(path: &PathBuf, instruction: &str, uninstall: bool) -> Result<String> {
    let mut root = if path.exists() {
        serde_json::from_slice::<Value>(
            &fs::read(path).with_context(|| format!("read {}", path.display()))?,
        )
        .with_context(|| format!("parse {}", path.display()))?
    } else {
        json!({})
    };
    let object = root
        .as_object_mut()
        .context("OpenCode configuration must be a JSON object")?;
    let existing = object
        .get("instructions")
        .map(|value| {
            value
                .as_str()
                .context("OpenCode instructions must be a string")
        })
        .transpose()?
        .unwrap_or_default();
    let merged = merge_managed_text(existing, instruction, uninstall);
    if merged.is_empty() {
        object.remove("instructions");
    } else {
        object.insert("instructions".into(), Value::String(merged));
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&root)?))
}

const MANAGED_START: &str = "<!-- BEGIN BRAINMAP MANAGED BLOCK -->";
const MANAGED_END: &str = "<!-- END BRAINMAP MANAGED BLOCK -->";
const TOML_MANAGED_START: &str = "# BEGIN BRAINMAP MANAGED BLOCK";
const TOML_MANAGED_END: &str = "# END BRAINMAP MANAGED BLOCK";

fn merge_managed_text(existing: &str, block: &str, uninstall: bool) -> String {
    let cleaned = remove_managed_text(existing);
    if uninstall {
        return cleaned;
    }
    if cleaned.is_empty() {
        block.to_string()
    } else {
        format!("{cleaned}\n\n{block}")
    }
}

fn merge_managed_toml(existing: &str, block: &str, uninstall: bool) -> Result<String> {
    let start_markers = existing.matches(TOML_MANAGED_START).count();
    let end_markers = existing.matches(TOML_MANAGED_END).count();
    if start_markers != end_markers || start_markers > 1 {
        bail!("invalid or duplicate Brainmap managed TOML markers");
    }
    let cleaned = remove_marked_block(existing, TOML_MANAGED_START, TOML_MANAGED_END);
    if uninstall {
        return Ok(cleaned);
    }
    let existing_table = cleaned
        .parse::<toml::Table>()
        .context("invalid existing Codex TOML configuration")?;
    if existing_table
        .get("mcp_servers")
        .and_then(toml::Value::as_table)
        .is_some_and(|servers| servers.contains_key("brainmap"))
    {
        bail!(
            "refusing to replace unmanaged Brainmap MCP table; remove [mcp_servers.brainmap] or manage it explicitly"
        );
    }
    let mut out = cleaned.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(block.trim());
    out.push('\n');
    out.parse::<toml::Table>()
        .context("generated invalid Codex TOML configuration")?;
    Ok(out)
}

#[derive(Clone, Copy, Debug, Default)]
struct CodexMcpConfigStatus {
    valid: bool,
    vault_matches: bool,
}

fn codex_mcp_config_status(text: &str, expected_vault: &std::path::Path) -> CodexMcpConfigStatus {
    let Ok(document) = text.parse::<toml::Table>() else {
        return CodexMcpConfigStatus::default();
    };
    let Some(server) = document
        .get("mcp_servers")
        .and_then(toml::Value::as_table)
        .and_then(|servers| servers.get("brainmap"))
        .and_then(toml::Value::as_table)
    else {
        return CodexMcpConfigStatus::default();
    };
    let Some(args) = server
        .get("args")
        .and_then(toml::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(toml::Value::as_str)
                .collect::<Vec<_>>()
        })
    else {
        return CodexMcpConfigStatus::default();
    };
    let expected_tools = [
        "brainmap_decision_gate",
        "brainmap_context",
        "brainmap_record_decision",
        "brainmap_learn_feedback",
        "brainmap_list_pending",
        "brainmap_preview_update",
        "brainmap_apply_update",
        "brainmap_autopilot_status",
    ];
    let enabled_tools = server
        .get("enabled_tools")
        .and_then(toml::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(toml::Value::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tools = server.get("tools").and_then(toml::Value::as_table);
    let approval_is_prompt = |tool: &str| {
        tools
            .and_then(|tools| tools.get(tool))
            .and_then(toml::Value::as_table)
            .and_then(|settings| settings.get("approval_mode"))
            .and_then(toml::Value::as_str)
            == Some("prompt")
    };
    let args_shape_valid = args.len() == 4 && args[..3] == ["mcp", "serve", "--vault"];
    let valid = server.get("command").and_then(toml::Value::as_str) == Some("brainmap")
        && args_shape_valid
        && server.get("required").and_then(toml::Value::as_bool) == Some(true)
        && enabled_tools == expected_tools
        && approval_is_prompt("brainmap_learn_feedback")
        && approval_is_prompt("brainmap_apply_update");
    CodexMcpConfigStatus {
        valid,
        vault_matches: valid
            && args.get(3).copied() == Some(expected_vault.to_string_lossy().as_ref()),
    }
}

fn remove_marked_block(existing: &str, start_marker: &str, end_marker: &str) -> String {
    let Some(start) = existing.find(start_marker) else {
        return existing.trim().to_string() + if existing.trim().is_empty() { "" } else { "\n" };
    };
    let Some(relative_end) = existing[start..].find(end_marker) else {
        return existing.to_string();
    };
    let end = start + relative_end + end_marker.len();
    let mut out = format!("{}{}", &existing[..start], &existing[end..]);
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out.trim().to_string() + if out.trim().is_empty() { "" } else { "\n" }
}

fn codex_mcp_block(vault: Option<&std::path::Path>) -> String {
    let vault = vault
        .map(util::expand_tilde)
        .unwrap_or_else(util::default_vault);
    let vault = serde_json::to_string(&vault.to_string_lossy()).unwrap();
    format!(
        r#"{TOML_MANAGED_START}
[mcp_servers.brainmap]
command = "brainmap"
args = ["mcp", "serve", "--vault", {vault}]
required = true
enabled_tools = ["brainmap_decision_gate", "brainmap_context", "brainmap_record_decision", "brainmap_learn_feedback", "brainmap_list_pending", "brainmap_preview_update", "brainmap_apply_update", "brainmap_autopilot_status"]

[mcp_servers.brainmap.tools.brainmap_learn_feedback]
approval_mode = "prompt"

[mcp_servers.brainmap.tools.brainmap_apply_update]
approval_mode = "prompt"
{TOML_MANAGED_END}"#
    )
}

fn remove_managed_text(existing: &str) -> String {
    let Some(start) = existing.find(MANAGED_START) else {
        return existing.to_string();
    };
    let Some(relative_end) = existing[start..].find(MANAGED_END) else {
        return existing.to_string();
    };
    let end = start + relative_end + MANAGED_END.len();
    let mut before = existing[..start].to_string();
    if before.ends_with("\n\n") {
        before.truncate(before.len() - 2);
    }
    let mut after = &existing[end..];
    if let Some(rest) = after.strip_prefix('\n') {
        after = rest;
    }
    format!("{before}{after}")
}

fn json_hooks_contents(
    path: &PathBuf,
    bindings: &[HookBinding],
    uninstall: bool,
) -> Result<String> {
    let root = if path.exists() {
        serde_json::from_slice(&fs::read(path).with_context(|| format!("read {}", path.display()))?)
            .with_context(|| format!("parse {}", path.display()))?
    } else {
        json!({})
    };
    let merged = merge_hook_bindings(root, bindings, uninstall)?;
    Ok(format!("{}\n", serde_json::to_string_pretty(&merged)?))
}

fn merge_hook_bindings(
    mut root: Value,
    bindings: &[HookBinding],
    uninstall: bool,
) -> Result<Value> {
    let root_obj = root
        .as_object_mut()
        .context("hook configuration must be a JSON object")?;
    let hooks = root_obj.entry("hooks").or_insert_with(|| json!({}));
    let hooks_obj = hooks
        .as_object_mut()
        .context("hook configuration 'hooks' must be an object")?;

    for binding in bindings {
        let entries = hooks_obj.entry(binding.event).or_insert_with(|| json!([]));
        let entries = entries
            .as_array_mut()
            .with_context(|| format!("hook event '{}' must be an array", binding.event))?;
        if uninstall {
            entries.retain_mut(|entry| remove_hook_command(entry, &binding.command));
        } else if !entries
            .iter()
            .any(|entry| entry_has_command(entry, &binding.command))
        {
            entries.push(binding_json(binding));
        }
    }
    Ok(root)
}

fn remove_hook_command(entry: &mut Value, command: &str) -> bool {
    let Some(hooks) = entry.get_mut("hooks").and_then(Value::as_array_mut) else {
        return true;
    };
    hooks.retain(|hook| hook.get("command").and_then(Value::as_str) != Some(command));
    !hooks.is_empty()
}

fn binding_json(binding: &HookBinding) -> Value {
    let mut entry = json!({
        "hooks": [
            {
                "type": "command",
                "command": binding.command,
                "timeout": binding.timeout_secs
            }
        ]
    });
    if let Some(matcher) = binding.matcher {
        entry["matcher"] = json!(matcher);
    }
    entry
}

fn entry_has_command(entry: &Value, command: &str) -> bool {
    entry
        .get("hooks")
        .and_then(Value::as_array)
        .is_some_and(|hooks| {
            hooks
                .iter()
                .any(|hook| hook.get("command").and_then(Value::as_str) == Some(command))
        })
}

fn managed_block(host: &str) -> String {
    format!(
        r#"<!-- BEGIN BRAINMAP MANAGED BLOCK -->
# Brainmap Harness Instructions

Host: {host}
Enforcement: host hooks call `brainmap harness hook`; this file is the fallback.

Load current local instructions before decision-engine work:

```bash
brainmap skill build-decision-engine --host {host}
```

If that command fails, run `brainmap gate --intent would-ask-user --situation "..." --options "A|B|C" --json` before decision questions. Ask naturally with concrete options and a free-text path. Never store secrets or raw project archives.
<!-- END BRAINMAP MANAGED BLOCK -->
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_hooks_is_idempotent_and_preserves_existing_hooks() {
        let bindings = hook_bindings("codex");
        let root = json!({
            "hooks": {
                "PreToolUse": [
                    {
                        "matcher": "Bash",
                        "hooks": [
                            {"type": "command", "command": "rtk hook claude"}
                        ]
                    }
                ]
            }
        });
        let merged = merge_hook_bindings(root, &bindings, false).unwrap();
        let pre_tool = merged["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(
            pre_tool
                .iter()
                .any(|entry| entry_has_command(entry, "rtk hook claude"))
        );
        assert!(pre_tool.iter().any(|entry| entry_has_command(
            entry,
            "brainmap harness hook --host codex --event PreToolUse"
        )));

        let merged_again = merge_hook_bindings(merged.clone(), &bindings, false).unwrap();
        assert_eq!(
            pre_tool.len(),
            merged_again["hooks"]["PreToolUse"]
                .as_array()
                .unwrap()
                .len()
        );

        let removed = merge_hook_bindings(merged_again, &bindings, true).unwrap();
        let pre_tool = removed["hooks"]["PreToolUse"].as_array().unwrap();
        assert!(
            pre_tool
                .iter()
                .any(|entry| entry_has_command(entry, "rtk hook claude"))
        );
        assert!(!pre_tool.iter().any(|entry| entry_has_command(
            entry,
            "brainmap harness hook --host codex --event PreToolUse"
        )));
    }

    #[test]
    fn uninstall_removes_only_brainmap_command_from_mixed_hook_entry() {
        let bindings = hook_bindings("codex");
        let root = json!({
            "hooks": {
                "PreToolUse": [{
                    "matcher": "Bash",
                    "hooks": [
                        {"type": "command", "command": "brainmap harness hook --host codex --event PreToolUse"},
                        {"type": "command", "command": "user-owned-hook"}
                    ]
                }]
            }
        });

        let removed = merge_hook_bindings(root, &bindings, true).unwrap();
        let entries = removed["hooks"]["PreToolUse"].as_array().unwrap();

        assert_eq!(entries.len(), 1);
        assert!(entry_has_command(&entries[0], "user-owned-hook"));
        assert!(!entry_has_command(
            &entries[0],
            "brainmap harness hook --host codex --event PreToolUse"
        ));
    }

    #[test]
    fn codex_plan_installs_skill() {
        let args = InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(PathBuf::from("/tmp/brainmap-project")),
            vault: None,
            dry_run: true,
            uninstall: false,
        };
        let plan = plan(&args);

        assert!(plan.iter().any(|item| {
            item.path
                .ends_with(".codex/skills/build-decision-engine/SKILL.md")
                && item.enforcement == "instruction-only"
        }));
        let config = plan
            .iter()
            .find(|item| item.path.ends_with(".codex/config.toml"))
            .unwrap()
            .contents()
            .unwrap();
        assert!(config.contains("[mcp_servers.brainmap]"));
        assert!(config.contains("brainmap_apply_update"));
        assert!(config.contains("approval_mode = \"prompt\""));
    }

    #[test]
    fn codex_mcp_config_merge_is_idempotent_and_preserves_user_toml() {
        let existing = "model = \"gpt-5\"\n";
        let block = codex_mcp_block(Some(std::path::Path::new("/tmp/BrainMap")));
        let merged = merge_managed_toml(existing, &block, false).unwrap();
        let merged_again = merge_managed_toml(&merged, &block, false).unwrap();

        assert_eq!(merged, merged_again);
        assert!(merged.contains("model = \"gpt-5\""));
        assert!(merged.contains("/tmp/BrainMap"));
        assert_eq!(
            merge_managed_toml(&merged, &block, true).unwrap(),
            "model = \"gpt-5\"\n"
        );
    }

    #[test]
    fn codex_install_refuses_an_unmanaged_brainmap_mcp_table() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        let config = project.join(".codex/config.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        let original = r#"[mcp_servers.brainmap]
command = "user-owned-brainmap"
"#;
        fs::write(&config, original).unwrap();

        let error = install_harness(InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(project),
            vault: Some(PathBuf::from("/tmp/BrainMap")),
            dry_run: false,
            uninstall: false,
        })
        .unwrap_err();

        assert!(error.to_string().contains("unmanaged Brainmap MCP table"));
        assert_eq!(fs::read_to_string(config).unwrap(), original);
    }

    #[test]
    fn installed_skill_is_static_cli_shim() {
        let args = InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(PathBuf::from("/tmp/brainmap-project")),
            vault: None,
            dry_run: true,
            uninstall: false,
        };
        let plan = plan(&args);
        let skill = plan
            .iter()
            .find(|item| {
                item.path
                    .ends_with(".codex/skills/build-decision-engine/SKILL.md")
            })
            .unwrap()
            .contents()
            .unwrap();

        assert!(skill.contains("brainmap skill build-decision-engine --host codex"));
        assert!(skill.contains("If that command fails"));
        assert!(!skill.contains("Use Brainmap to learn decisions, not knowledge."));
    }

    #[test]
    fn codex_install_and_uninstall_preserve_existing_agents_content() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let agents = project.join("AGENTS.md");
        let original = "# Project instructions\n\nKeep this content.\n";
        fs::write(&agents, original).unwrap();

        install_harness(InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(project.clone()),
            vault: None,
            dry_run: false,
            uninstall: false,
        })
        .unwrap();

        let installed = fs::read_to_string(&agents).unwrap();
        assert!(installed.contains(original.trim()));
        assert!(installed.contains("BEGIN BRAINMAP MANAGED BLOCK"));
        let backups_after_first_install = fs::read_dir(&project)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("AGENTS.bak-")
            })
            .count();

        install_harness(InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(project.clone()),
            vault: None,
            dry_run: false,
            uninstall: false,
        })
        .unwrap();
        assert_eq!(fs::read_to_string(&agents).unwrap(), installed);
        let backups_after_second_install = fs::read_dir(&project)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("AGENTS.bak-")
            })
            .count();
        assert_eq!(backups_after_second_install, backups_after_first_install);

        install_harness(InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(project.clone()),
            vault: None,
            dry_run: false,
            uninstall: true,
        })
        .unwrap();

        assert_eq!(fs::read_to_string(&agents).unwrap(), original);
        let backups = fs::read_dir(&project)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("AGENTS.bak-")
            })
            .count();
        assert_eq!(backups, 2);
    }

    #[test]
    fn opencode_install_preserves_json_and_existing_instructions() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        fs::create_dir_all(&project).unwrap();
        let config = project.join("opencode.json");
        fs::write(
            &config,
            serde_json::to_vec_pretty(&json!({
                "theme": "dark",
                "instructions": "Keep existing instructions."
            }))
            .unwrap(),
        )
        .unwrap();

        install_harness(InstallHarnessArgs {
            target: "opencode".into(),
            global: false,
            project: Some(project.clone()),
            vault: None,
            dry_run: false,
            uninstall: false,
        })
        .unwrap();
        let installed: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        assert_eq!(installed["theme"], "dark");
        assert!(
            installed["instructions"]
                .as_str()
                .unwrap()
                .contains("Keep existing instructions.")
        );
        assert!(
            installed["instructions"]
                .as_str()
                .unwrap()
                .contains(MANAGED_START)
        );

        install_harness(InstallHarnessArgs {
            target: "opencode".into(),
            global: false,
            project: Some(project),
            vault: None,
            dry_run: false,
            uninstall: true,
        })
        .unwrap();
        let uninstalled: Value = serde_json::from_slice(&fs::read(config).unwrap()).unwrap();
        assert_eq!(uninstalled["theme"], "dark");
        assert_eq!(uninstalled["instructions"], "Keep existing instructions.");
    }

    #[test]
    fn owned_files_are_never_overwritten_or_removed_when_unmanaged() {
        let tmp = tempfile::tempdir().unwrap();
        let project = tmp.path().join("project");
        let skill_path = project.join(".codex/skills/build-decision-engine/SKILL.md");
        fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
        fs::write(&skill_path, "user-owned skill\n").unwrap();

        let install_error = install_harness(InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(project.clone()),
            vault: None,
            dry_run: false,
            uninstall: false,
        })
        .unwrap_err();
        assert!(install_error.to_string().contains("refusing to overwrite"));
        assert_eq!(
            fs::read_to_string(&skill_path).unwrap(),
            "user-owned skill\n"
        );

        let uninstall_error = install_harness(InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(project),
            vault: None,
            dry_run: false,
            uninstall: true,
        })
        .unwrap_err();
        assert!(uninstall_error.to_string().contains("refusing to remove"));
        assert_eq!(
            fs::read_to_string(&skill_path).unwrap(),
            "user-owned skill\n"
        );
    }
}
