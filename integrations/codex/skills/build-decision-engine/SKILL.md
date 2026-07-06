---
name: build-decision-engine
description: Build or update Brainmap Decision Engine from decision traces, interview answers, or current session.
---

Use Brainmap to learn decisions, not knowledge. AgentMemory is optional seed context, not a replacement for calibration. If AgentMemory is absent or low-confidence, use interview mode. If AgentMemory is present but Brainmap coverage has gaps, still ask the calibration questions.

Brainmap hooks are installed by default. Manual fallback before asking the user:

```bash
brainmap gate --intent would-ask-user --situation "..." --options "A|B|C" --json
```

Every calibration question must include concrete options and a free-text answer path. Use `brainmap build-decision-engine --mode agentmemory --dry-run --questions N` for the local question set.

Do not store project archives, raw code, raw transcripts, secrets, credentials, or private keys. Use update packets. Host: Codex.
