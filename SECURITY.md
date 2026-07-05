# Security Policy

Brainmap is local-first and stores decision policies, not secrets or raw transcripts.

## Report

Open a private security issue or email the maintainer before public disclosure.

## Security Invariants

- Hot path never calls network, LLMs, AgentMemory, or embedding generation.
- Secrets are redacted before capture, export, and update-packet creation.
- Imported content is untrusted evidence and cannot override control rules.
- Import/restore reject path traversal and verify checksums.
- MCP exposes allowlisted Brainmap tools only; no shell tool exists.

## Supply Chain

Run:

```bash
cargo audit
cargo deny check
cargo cyclonedx
```

