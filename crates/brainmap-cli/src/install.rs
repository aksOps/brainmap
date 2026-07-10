use crate::{skill, util};
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

struct PlanItem {
    path: PathBuf,
    enforcement: &'static str,
    action: PlanAction,
}

enum PlanAction {
    OwnedText(String),
    ManagedText(String),
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
            PlanAction::OwnedText(contents) | PlanAction::ManagedText(contents) => {
                Ok(contents.clone())
            }
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
        if self.path.exists() {
            backup(&self.path)?;
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
            PlanAction::JsonHooks(bindings) => json_hooks_contents(&self.path, bindings, false)?,
            PlanAction::JsonInstruction(instruction) => {
                json_instruction_contents(&self.path, instruction, false)?
            }
        };
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
                enforcement: "instruction+skill",
                action: PlanAction::OwnedText(skill::build_decision_engine_shim("claude-code")),
            },
            PlanItem {
                path: base.join(".claude/settings.json"),
                enforcement: "hooked",
                action: PlanAction::JsonHooks(hook_bindings("claude-code")),
            },
        ],
        "codex" => vec![
            PlanItem {
                path: base.join(".codex/skills/build-decision-engine/SKILL.md"),
                enforcement: "instruction+skill",
                action: PlanAction::OwnedText(skill::build_decision_engine_shim("codex")),
            },
            PlanItem {
                path: base.join("AGENTS.md"),
                enforcement: "instruction fallback",
                action: PlanAction::ManagedText(managed_block("codex")),
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
            dry_run: true,
            uninstall: false,
        };
        let plan = plan(&args);

        assert!(plan.iter().any(|item| {
            item.path
                .ends_with(".codex/skills/build-decision-engine/SKILL.md")
                && item.enforcement == "instruction+skill"
        }));
    }

    #[test]
    fn installed_skill_is_static_cli_shim() {
        let args = InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(PathBuf::from("/tmp/brainmap-project")),
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
            dry_run: false,
            uninstall: false,
        })
        .unwrap();

        let installed = fs::read_to_string(&agents).unwrap();
        assert!(installed.contains(original.trim()));
        assert!(installed.contains("BEGIN BRAINMAP MANAGED BLOCK"));

        install_harness(InstallHarnessArgs {
            target: "codex".into(),
            global: false,
            project: Some(project.clone()),
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
