use crate::util;
use anyhow::{Context, Result};
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
        match (&item.action, args.uninstall) {
            (PlanAction::Text(_), true) => {
                if item.path.exists() {
                    fs::remove_file(&item.path)?;
                    println!("removed {}", item.path.display());
                }
            }
            (PlanAction::JsonHooks(bindings), true) => {
                if item.path.exists() {
                    backup(&item.path)?;
                    let contents = json_hooks_contents(&item.path, bindings, true)?;
                    util::write_atomic(&item.path, contents.as_bytes())?;
                    println!("updated {} ({})", item.path.display(), item.enforcement);
                }
            }
            _ => {
                if item.path.exists() {
                    backup(&item.path)?;
                }
                let contents = item.contents()?;
                util::write_atomic(&item.path, contents.as_bytes())?;
                println!("wrote {} ({})", item.path.display(), item.enforcement);
            }
        }
    }
    Ok(())
}

struct PlanItem {
    path: PathBuf,
    enforcement: &'static str,
    action: PlanAction,
}

enum PlanAction {
    Text(String),
    JsonHooks(Vec<HookBinding>),
}

#[derive(Clone)]
struct HookBinding {
    event: &'static str,
    matcher: Option<&'static str>,
    command: String,
    timeout_secs: u64,
}

impl PlanItem {
    fn contents(&self) -> Result<String> {
        match &self.action {
            PlanAction::Text(contents) => Ok(contents.clone()),
            PlanAction::JsonHooks(bindings) => json_hooks_contents(&self.path, bindings, false),
        }
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
                enforcement: "instruction+skill",
                action: PlanAction::Text(skill("Claude Code")),
            },
            PlanItem {
                path: base.join(".claude/settings.json"),
                enforcement: "hooked",
                action: PlanAction::JsonHooks(hook_bindings("claude-code")),
            },
        ],
        "codex" => vec![
            PlanItem {
                path: base.join("AGENTS.md"),
                enforcement: "instruction fallback",
                action: PlanAction::Text(managed_block("Codex")),
            },
            PlanItem {
                path: base.join(".codex/hooks.json"),
                enforcement: "hooked",
                action: PlanAction::JsonHooks(hook_bindings("codex")),
            },
        ],
        "opencode" => vec![PlanItem {
            path: base.join("opencode.json"),
            enforcement: "best-effort",
            action: PlanAction::Text(
                "{\"instructions\":\"Ask Brainmap before asking the user; use brainmap gate --json.\"}\n"
                    .into(),
            ),
        }],
        "copilot" => vec![PlanItem {
            path: base.join(".github/copilot-instructions.md"),
            enforcement: "instruction-only",
            action: PlanAction::Text(managed_block("GitHub Copilot")),
        }],
        "generic-stdio" => vec![PlanItem {
            path: base.join("brainmap-harness.md"),
            enforcement: "enforced",
            action: PlanAction::Text("Generic stdio harness can enforce with `brainmap harness stdio --fail-on-block`. Send one JSON gate request per line; read one gate JSON response per line.\n".into()),
        }],
        _ => vec![PlanItem {
            path: base.join("brainmap-harness-unsupported.txt"),
            enforcement: "instruction-only",
            action: PlanAction::Text(format!(
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
    let backup = path.with_extension(format!("bak-{}", chrono::Utc::now().timestamp()));
    fs::copy(path, &backup)
        .with_context(|| format!("backup {} to {}", path.display(), backup.display()))?;
    println!("backup {}", backup.display());
    Ok(())
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
    let merged = merge_hook_bindings(root, bindings, uninstall);
    Ok(format!("{}\n", serde_json::to_string_pretty(&merged)?))
}

fn merge_hook_bindings(mut root: Value, bindings: &[HookBinding], uninstall: bool) -> Value {
    if !root.is_object() {
        root = json!({});
    }
    let root_obj = root.as_object_mut().expect("root object");
    let hooks = root_obj.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks_obj = hooks.as_object_mut().expect("hooks object");

    for binding in bindings {
        let entries = hooks_obj.entry(binding.event).or_insert_with(|| json!([]));
        if !entries.is_array() {
            *entries = json!([]);
        }
        let entries = entries.as_array_mut().expect("hook entries");
        if uninstall {
            entries.retain(|entry| !entry_has_command(entry, &binding.command));
        } else if !entries
            .iter()
            .any(|entry| entry_has_command(entry, &binding.command))
        {
            entries.push(binding_json(binding));
        }
    }
    root
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
        r#"# Brainmap Harness Instructions

<!-- BEGIN BRAINMAP MANAGED BLOCK -->
Host: {host}
Enforcement: host hooks call `brainmap harness hook`; this file is the fallback.

Before asking the user a decision question, run:

```bash
brainmap gate --intent would-ask-user --situation "..." --options "A|B|C" --json
```

AgentMemory can seed context but must not suppress required calibration. When Brainmap confidence or coverage is missing, ask the question with concrete options and a free-text path.

Before meaningful actions, run Brainmap gate. After action, record decision. After correction, learn feedback. Never store secrets or raw project archives in Brainmap.
<!-- END BRAINMAP MANAGED BLOCK -->
"#
    )
}

fn skill(host: &str) -> String {
    format!(
        r#"---
name: build-decision-engine
description: Build or update Brainmap Decision Engine from decision traces, interview answers, or current session.
---

Use Brainmap to learn decisions, not knowledge. AgentMemory is optional seed context, not a replacement for calibration. If AgentMemory is absent or low-confidence, use interview mode. If AgentMemory is present but Brainmap coverage has gaps, still ask the calibration questions.

Brainmap hooks are installed by default. Manual fallback before asking the user:

```bash
brainmap gate --intent would-ask-user --situation "..." --options "A|B|C" --json
```

Every calibration question must include concrete options and a free-text answer path. Use `brainmap build-decision-engine --mode agentmemory --dry-run --questions N` for the local question set.

Do not store project archives, raw code, raw transcripts, secrets, credentials, or private keys. Use update packets. Host: {host}.
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
        let merged = merge_hook_bindings(root, &bindings, false);
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

        let merged_again = merge_hook_bindings(merged.clone(), &bindings, false);
        assert_eq!(
            pre_tool.len(),
            merged_again["hooks"]["PreToolUse"]
                .as_array()
                .unwrap()
                .len()
        );

        let removed = merge_hook_bindings(merged_again, &bindings, true);
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
}
