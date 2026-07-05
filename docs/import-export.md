# Import Export

Portable exports are `.brainmap.tar.zst` archives with `manifest.json`, checksums, Markdown vault files, config, ledgers, and pending packets.

Encrypted exports use age-compatible recipient encryption:

```bash
brainmap export --mode portable --encrypt --recipient age1... --vault ./tmp/BrainMap --out ./tmp/brainmap.brainmap.tar.zst.age
brainmap verify-export ./tmp/brainmap.brainmap.tar.zst.age --identity ./identity.txt
brainmap restore --file ./tmp/brainmap.brainmap.tar.zst.age --identity ./identity.txt --to ./tmp/Restored
```

Excluded by default:

- SQLite index
- model cache
- locks
- large backups
- secrets

Restore validates manifest/checksums, rejects path traversal, backs up existing target directory, extracts files, rebuilds index, runs link-check, and runs a gate smoke test.
