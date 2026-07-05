use crate::util;
use anyhow::Result;
use clap::Args;
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
            if item.path.exists() {
                fs::remove_file(&item.path)?;
                println!("removed {}", item.path.display());
            }
            continue;
        }
        if item.path.exists() {
            let backup = item
                .path
                .with_extension(format!("bak-{}", chrono::Utc::now().timestamp()));
            fs::copy(&item.path, &backup)?;
            println!("backup {}", backup.display());
        }
        util::write_atomic(&item.path, item.contents.as_bytes())?;
        println!("wrote {} ({})", item.path.display(), item.enforcement);
    }
    Ok(())
}

struct PlanItem {
    path: PathBuf,
    enforcement: &'static str,
    contents: String,
}

fn plan(args: &InstallHarnessArgs) -> Vec<PlanItem> {
    let base = if args.global {
        util::home_dir()
    } else {
        args.project.clone().unwrap_or_else(|| PathBuf::from("."))
    };
    match args.target.as_str() {
        "claude-code" => vec![PlanItem {
            path: base.join(".claude/skills/build-decision-engine/SKILL.md"),
            enforcement: "best-effort",
            contents: skill("Claude Code"),
        }],
        "codex" => vec![PlanItem {
            path: base.join("AGENTS.md"),
            enforcement: "instruction-only",
            contents: managed_block("Codex"),
        }],
        "opencode" => vec![PlanItem {
            path: base.join("opencode.json"),
            enforcement: "best-effort",
            contents: "{\"instructions\":\"Ask Brainmap before asking the user; use brainmap gate --json.\"}\n".into(),
        }],
        "copilot" => vec![PlanItem {
            path: base.join(".github/copilot-instructions.md"),
            enforcement: "instruction-only",
            contents: managed_block("GitHub Copilot"),
        }],
        "generic-stdio" => vec![PlanItem {
            path: base.join("brainmap-harness.md"),
            enforcement: "enforced",
            contents: "Generic stdio harness can enforce with `brainmap harness stdio --fail-on-block`. Send one JSON gate request per line; read one gate JSON response per line.\n".into(),
        }],
        _ => vec![PlanItem {
            path: base.join("brainmap-harness-unsupported.txt"),
            enforcement: "instruction-only",
            contents: format!("Unsupported target {}; no install performed\n", args.target),
        }],
    }
}

fn managed_block(host: &str) -> String {
    format!(
        r#"# Brainmap Harness Instructions

<!-- BEGIN BRAINMAP MANAGED BLOCK -->
Host: {host}
Enforcement: instruction-only unless host hooks explicitly call Brainmap.

Before asking the user a decision question, run:

```bash
brainmap gate --intent would-ask-user --situation "..." --options "A|B|C" --json
```

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

Use Brainmap to learn decisions, not knowledge. AgentMemory is optional. If absent, use interview mode.

Ask Brainmap before asking the user:

```bash
brainmap gate --intent would-ask-user --situation "..." --options "A|B|C" --json
```

Do not store project archives, raw code, raw transcripts, secrets, credentials, or private keys. Use update packets. Host: {host}.
"#
    )
}
