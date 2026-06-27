# DSearch

This entire project is being autonomously built with [AutoCode](https://github.com/Eric-Lautanen/AutoCode)

Distributed search engine foundation. Multi-agent discovery, registration, and peer-to-peer search primitives built and tested on a local mesh network.

## Build Status

| Phase | Description | Status |
|-------|-------------|--------|
| 1 | Core networking + identity | ✅ Complete |
| 2 | Data model + config | ✅ Complete |
| 3 | Local storage | ✅ Complete |
| 4 | Search | ✅ Complete |
| 5 | Scraper + announcement + dedup | ✅ Complete |
| 6 | Sanitization | ✅ Complete |
| 7 | Agent API + CLI | ✅ Complete |
| 8 | First-run + UI | ✅ Complete |
| 9 | Hardening + service | ✅ Complete |

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

## Phase 3 — Local storage

- `redb 2.6.0` — `{data_dir}/store.redb`
- Tables: `records`, `source_index`, `announcements`, `routing`, `pins`, `peers`, `banned_peers`, `meta`
- Inverted index on `(schema, tag_key, tag_value)`
- `source_index`: `source_hash → record_id` for dedup
- Tier 2 TTL enforcement via async expiry sweeper
- Storage quota + eviction policy (`evict_oldest`, `pause_scraper`, `warn_only`)
- Schema version check + migration runner on open
- `dsearch record insert/get/list/pin/unpin/delete/announce/sweep` CLI commands

## Phase 4 — Search

- Local Tier 3 search (sub-ms)
- Query language parser (`src/search/query.rs`): AND terms, `"exact phrases"`, `OR`, `-exclude`, field filters (`schema:`, `tag:`, `source:`, `scraped:`, `refresh:`), `limit:`, `since:`/`before:`
- Ranking: field match weight (title > tag > body) + exact phrase bonus + freshness + holder count
- `dsearch search "query" --schema --limit --output json` CLI command
- 23 query parser + search engine unit tests

## Phase 5 — Scraper + announcement + dedup

- Source hash dedup before write (same `source_hash` → keep newer `created_at`)
- Announcement creation via `dsearch record announce`
- `dsearch scraper add/list/run` CLI commands
- Hand-rolled HTTP/1.1 client over `tokio::net::TcpStream` for `url` sources
- Hand-rolled HTTPS via existing `rustls` crate (quinn's TLS backend) + `std::net::TcpStream` in `spawn_blocking` — no extra deps
- Hand-rolled PEM cert parsing + base64 decoder — plain format logic per roadmap dep philosophy
- `reqwest` reserved for keyword-discovery path only (cookie jars, UA rotation, redirect handling)

## Phase 6 — Sanitization

- Single `sanitize()` pipeline — all ingest paths
- Valid UTF-8, no control chars 0x00–0x1F except 0x0A (newline)
- No Unicode Cf (format) or Cc (control) categories
- Caps: 1 MB record, 256 B key, 64 KB value
- `sanitize_record()` applied on `record insert` and scraper output
- 15 sanitization unit tests

## Phase 7 — Agent API + CLI

- Hand-written async HTTP/1.1 server (`tokio::net::TcpListener`) — local API on `127.0.0.1:7743`
- Port conflict handling: auto-increment 7743→7753, actual port written to `{data_dir}/api.port`
- All CLI subcommands proxy through local HTTP API when node is running (DB lock-safe)
- `--output json` on all list/get commands (CLI and API parity)
- Local API routes: `/health`, `/node`, `/search`, `/record/{id}`, `/records`, `/schema`, `/schema/{id}`, `/peers`, `/peers/add`, `/scraper`, `/scraper/run`, `/storage`, `/storage/vacuum`, `/config`, `/config/set`, `/identity`, `/bootstrap`, `/openapi.json`
- Write routes: `/record/insert`, `/record/pin`, `/record/unpin`, `/record/delete`, `/record/announce`, `/record/sweep`
- Gateway API: optional public read-only surface (`0.0.0.0:7744`), GET-only, per-key rate limiting
- Gateway API keys: 256-bit random secrets, Blake3-hashed storage, auto-generated nicknames (`swift-falcon-7x2`), `dsearch gateway key-create/key-list/key-revoke`
- OpenAPI 3.1 spec served at `/openapi.json`
- Response headers: `X-Node-Id`, `X-Protocol-Version`, `X-Record-Count`
- Error responses: `{ "error": "...", "code": N }` with proper HTTP status codes
- `dsearch node status` now queries the live API
- 110 unit tests passing + Phase 7 exit test (two-node, port auto-increment, all routes, CLI/API JSON parity)

## Phase 8 — First-run + UI

- egui 0.34 desktop UI with `eframe` runtime
- First-run wizard: data dir selection → identity generation → role picker → bootstrap connect
- UI and CLI onboarding converge on identical on-disk state (`identity.key`, `node.crt`, `config.toml`, `bootstrap.toml`)
- Settings panel: all config keys, data dir (clickable), identity, gateway keys, scrapers, bootstrap peers
- Status bar: role, peer count, record count, Tier 2 size, bandwidth
- Tray icon (`tray-icon` crate): status dot, open UI, pause/resume node, quit
- `--headless` flag for running without UI
- 41-check exit test covering init convergence, onboarding parity, settings data verification, tray wiring, and node start routing

## Phase 9 — Hardening + service

- **Connection pool cap** — default 200 QUIC connections, configurable via `node.max_connections`
- **Bounded Tokio channels** — capacity 256 for backpressure in node accept loop
- **DHT dead-peer pruning** — `prune_stale()` removes peers older than `PEER_STALE_SECS`, `prune_dead_peers()` uses default threshold
- **Peer reputation system** (`src/trust/reputation.rs`) — `ReputationTable` with penalty tracking, 24h linear decay, manual-only bans, 8 unit tests
- **Sybil resistance PoW** (`src/trust/pow.rs`) — `mine_pow()` / `verify_pow()` with configurable difficulty, 6 unit tests
- **Scraper subprocess isolation** (`src/scraper/sandbox.rs`) — `SandboxConfig` with path/namespace restrictions, `spawn_scraper_process` for isolated execution
- **Jittered re-announce** (`src/node/announce.rs`) — `jittered_reannounce_delay()` spreads pinned-record re-announcements with random jitter to avoid announcement storms
- **Relay bandwidth accounting** (`src/node/relay.rs`) — `RelayBandwidthAccount` with per-peer tracking, persisted across restarts via save/load
- **Search result cache** (`src/search/cache.rs`) — TTL-based cache with capacity eviction, prevents repeat fan-out on identical queries
- **Tier 2 write-rate limiter** (`src/storage/tier2_limiter.rs`) — per-peer rate limiting for announcement churn, independent of size cap
- **Concurrent query cap** — `node.max_concurrent_queries` config (default 50) prevents saturation from query volume
- **`dsearch doctor`** (`src/doctor/mod.rs`) — 6 categories (Identity, Storage, Network, API, Config, Service), real checks, `--output json` support
- **Service management** (`src/service/install.rs`) — `dsearch service install/enable/disable/status/uninstall` with real OS registration: systemd (Linux), launchd (macOS), Windows Service
- **Idle memory** — 14.4 MB measured (well under 50 MB target)
- 140 unit tests passing + 16-check Phase 9 exit test

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

# Insert a record from JSON
dsearch record insert record.json

# List records
dsearch record list

# Search
dsearch search "rust async" --schema wiki/article --limit 20

# Add and run a scraper job
dsearch scraper add --name my-job --source url --target https://example.com/page
dsearch scraper run my-job

# Announce a record
dsearch record announce <id>

# Health check
dsearch doctor

# Install as a system service
dsearch service install --headless
dsearch service enable
dsearch service status
```

## Test Suite

| Phase | Exit Test | Unit Tests |
|-------|-----------|------------|
| 1 | Two-node handshake + clean disconnect | — |
| 2 | Config round-trip, future version rejected, sign/verify | 9 model + 6 config + 3 migration + 6 sign |
| 3 | Record CRUD, dedup, expiry sweep, pin/unpin | 60 total |
| 4 | 15 query language features + JSON output | 84 total |
| 5 | Scraper job add/run, dedup, announce, sanitization | 99 total |
| 6 | 14 sanitization checks (control chars, BOM, size limits) | 110 total |
| 7 | Two-node, all API routes, CLI/API JSON parity, gateway keys | 110 total |
| 8 | 41 checks (init convergence, onboarding parity, settings, tray) | 140 total |
| 9 | 16 checks (doctor, service, pool cap, reputation, PoW, cache, limiter, sandbox, memory) | 140 total |
