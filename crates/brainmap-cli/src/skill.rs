use anyhow::Result;
use clap::Args;

#[derive(Args, Clone)]
pub struct BuildDecisionEngineSkillArgs {
    #[arg(long, default_value = "generic")]
    pub host: String,
}

pub fn build_decision_engine_cmd(args: BuildDecisionEngineSkillArgs) -> Result<()> {
    print!("{}", build_decision_engine(&args.host));
    Ok(())
}

pub fn build_decision_engine(host: &str) -> String {
    format!(
        r#"---
name: build-decision-engine
description: Build or update Brainmap Decision Engine from decision traces, interview answers, or current session.
---

Use Brainmap to learn decisions, not knowledge. AgentMemory is optional seed context, not a replacement for calibration. If AgentMemory is absent or low-confidence, use interview mode. If AgentMemory is present but Brainmap coverage has gaps, still ask the calibration questions.

Local hooks are installed by default. Manual fallback before asking the user:

```bash
brainmap gate --intent would-ask-user --situation "..." --options "A|B|C" --json
```

Every calibration question must include concrete options and a free-text answer path. Ask naturally; do not expose Brainmap, policy, or gate internals in user-facing questions. Use `brainmap build-decision-engine --mode agentmemory --dry-run --questions N` for the local question set.

Do not store project archives, raw code, raw transcripts, secrets, credentials, or private keys. Use update packets. Host: {host}.
"#
    )
}

pub fn build_decision_engine_shim(host: &str) -> String {
    format!(
        r#"---
name: build-decision-engine
description: Build or update Brainmap Decision Engine from decision traces, interview answers, or current session.
---

Run the current local instructions and follow them:

```bash
brainmap skill build-decision-engine --host {host}
```

If that command fails, use this minimal fallback: run `brainmap gate --intent would-ask-user --situation "..." --options "A|B|C" --json` before decision questions; ask naturally with concrete options and a free-text path; never store secrets, raw transcripts, raw code, or project archives.
"#
    )
}
