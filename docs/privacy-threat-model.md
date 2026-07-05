# Privacy Threat Model

Threats:

- secret capture
- prompt injection in imported content
- archive path traversal
- accidental remote model use with private memory
- arbitrary command execution via MCP

Controls:

- regex redaction before capture/export/packets
- imported content treated as untrusted evidence
- hard-coded policy precedence
- checksum manifest and path traversal rejection
- no network in hot path
- MCP allowlist without shell tool

