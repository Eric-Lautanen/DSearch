# DSearch

This entire project is being autonomously built with [AutoCode](https://github.com/Eric-Lautanen/AutoCode)

Distributed search engine foundation. Multi-agent discovery, registration, and peer-to-peer search primitives built and tested on a local mesh network.

## Build Status

| Phase | Description | Status |
|-------|-------------|--------|
| 1 | Core networking + identity | ✅ Complete |
| 2 | Data model + config | ✅ Complete |
| 3 | Local storage | 🔜 Next |
| 4 | Search | ⬜ Pending |
| 5 | Scraper + announcement + dedup | ⬜ Pending |
| 6 | Sanitization | ⬜ Pending |
| 7 | Agent API + CLI | ⬜ Pending |
| 8 | First-run + UI | ⬜ Pending |
| 9 | Hardening + service | ⬜ Pending |

## Phase 1 — Core networking + identity

- QUIC transport (quinn 0.11.9) with self-signed cert verifier
- Ed25519 keypair + TLS cert generation on first run
- Versioned wire protocol (`src/proto/`)
- Kademlia DHT — Tier 1 routing table
- Bootstrap peer resolution: `bootstrap.toml` → DNS → compiled defaults
- Protocol version handshake on every connection
- Graceful shutdown: drain in-flight streams, send `Goodbye`, close
- Two-node handshake + clean disconnect verified via exit test

## Phase 2 — Data model + config

- `ContentRecord` struct: `id`, `source_url`, `source_hash`, `schema`, `tags`, `body`, `created_at`, `expires_at`, `scrape_source`, `refresh_policy`, `sig`
- `Announcement` struct: `record_id`, `source_hash`, `schema`, `tags`, `holder_addr`, `expires_at`, `sig`
- `ScrapeJob` config object with source/refresh/lifecycle axes
- Signature scheme: canonical length-prefixed encoding, Ed25519 sign/verify for both structs
- Blake3 content addressing (`record_id`, `source_hash`)
- Schemas: `wiki/article`, `rust/crate`, `link/media`, `generic/kv`
- Hard caps: 1 MB record, 256 B announcement entry
- Full `config.toml` with all keys and defaults
- Config migration framework (`config_version` in `[meta]` table)
- Future `config_version` rejected on node start (prevents silent corruption)
- `dsearch config show/set/reset` CLI commands working
- 31 unit tests passing, all Phase 2 exit tests passing

## Quick Start

```bash
# Initialize a node
dsearch init

# Start the node
dsearch node start --headless

# Check config
dsearch config show

# View identity
dsearch identity show

# List bootstrap peers
dsearch bootstrap list
```
