---
name: build-decision-engine
description: Build or update Brainmap Decision Engine from decision traces, interview answers, or current session.
---

Run the current local instructions and follow them:

```bash
brainmap skill build-decision-engine --host codex
```

If that command fails, use this minimal fallback: run `brainmap gate --intent would-ask-user --situation "..." --options "A|B|C" --json` before decision questions; ask naturally with concrete options and a free-text path; never store secrets, raw transcripts, raw code, or project archives.
