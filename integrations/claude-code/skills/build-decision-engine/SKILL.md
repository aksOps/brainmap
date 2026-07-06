---
name: build-decision-engine
description: Build/update Brainmap from decisions, not knowledge.
---

Run the current local instructions and follow them:

```bash
brainmap skill build-decision-engine --host claude-code
```

If that command fails, use this minimal fallback: run `brainmap gate --intent would-ask-user --situation "..." --options "A|B|C" --json` before decision questions; ask naturally with concrete options and a free-text path; never store secrets, raw transcripts, raw code, or project archives.
