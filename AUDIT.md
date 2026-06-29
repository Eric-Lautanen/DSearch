# DSearch Codebase Audit

**Date:** 2025-07-13  
**Scope:** Full codebase vs. ROADMAP.md specification  
**Status:** All 140 unit tests pass; `cargo check` compiles with 74 warnings (mostly dead-code for future phases)

---

## Executive Summary

The codebase is a solid Phase 1–8 skeleton with most data structures, storage, search, API, CLI, UI, and service plumbing in place. However, there are **critical security holes**, **race conditions**, **spec-vs-implementation gaps**, and **missing features** that would block production readiness. The most severe issues are: a certificate verifier that accepts *anything*, signature verification code that is never called, a TOCTOU race in record dedup, unsanitized API inputs enabling injection attacks, and multiple roadmap-mandated features that are stubs or missing entirely.

---

## 1. CRITICAL — Security Vulnerabilities

### 1.1 Certificate Verifier Accepts All Certificates (No Real Verification)

**File:** `src/proto/cert.rs:157–189`  
**Severity:** 🔴 Critical

The `DsearchCertVerifier` unconditionally returns `assertion()` for every check — `verify_server_cert`, `verify_tls12_signature`, and `verify_tls13_signature`. This means **any** QUIC connection is accepted regardless of the peer's certificate. The roadmap explicitly states:

> *"Extracts the node ID from the SAN, Verifies the cert is self-signed and the public key matches the node ID, Accepts — no CA chain check"*

Instead, the verifier should:
1. Extract the node_id from the SAN URI field
2. Verify the cert is self-signed (issuer == subject)
3. Verify the public key in the cert matches the claimed node_id (Blake3 of the Ed25519 public key)
4. Verify the cert's signature is valid against its own public key

**Current code:**
```rust
fn verify_server_cert(&self, _end_entity: &CertificateDer<'_>, ...) 
    -> Result<ServerCertVerified, Error> 
{
    Ok(ServerCertVerified::assertion()) // ← accepts everything!
}
```

**Impact:** Any attacker can connect to a node claiming any node_id. This completely breaks the trust model — the entire identity/signature chain depends on QUIC-level authentication being real.

---

### 1.2 Signature Verification Never Called at Runtime

**Files:** `src/trust/sign.rs`, `src/trust/verify.rs`  
**Severity:** 🔴 Critical

The signing and verification functions (`sign_record`, `verify_record_sig`, `sign_announcement`, `verify_announcement_sig`, `verify_record_id`) are **never called** outside of unit tests. The compiler confirms this with 74 dead-code warnings.

The roadmap specifies verification must happen at:
- **Announce receipt** → drop announcement, apply peer penalty on failure
- **RecordReply receipt** → reject record, apply peer penalty on failure  
- **ReplicatePush receipt** → reject push, respond with error, apply peer penalty

**Current state:** When a record is announced via `dsearch record announce`, the `Announcement.sig` is set to `""` (empty string) — see `src/main.rs:832` and `src/api/handlers.rs:427`. No signing key is used. No verification is performed on receipt.

**Impact:** Any node can forge announcements and records with arbitrary content. The entire trust model is non-functional.

---

### 1.3 Identity Key Stored as Raw Bytes with No File Permissions

**File:** `src/proto/cert.rs:60`  
**Severity:** 🟠 High

```rust
std::fs::write(data_dir.join("identity.key"), &key_bytes)
```

The Ed25519 private key is written with default OS file permissions (typically world-readable on Linux). This key is the node's entire identity — if leaked, an attacker can impersonate the node, sign forged records, and decrypt traffic.

**Fix:** Set file permissions to 0600 (owner-read-write only) on Unix. On Windows, use ACLs to restrict to the current user.

---

### 1.4 Gateway API Key Stored in Plaintext JSON

**File:** `src/api/gateway_keys.rs:61–78`  
**Severity:** 🟠 High

The roadmap states: *"Keys stored hashed (Blake3) in the meta table; the raw secret is never persisted or logged after creation."*

The implementation stores keys in `gateway_keys.json` with the **Blake3 hash** of the secret — this part is correct. However:
1. The file is stored as plaintext JSON with no file permissions restriction
2. The key hash is stored alongside the nickname, created_at, etc. — an attacker who reads the file can attempt offline brute-force against the 256-bit keys (unlikely but the file should still be protected)
3. The roadmap says "meta table" (i.e., redb) but the implementation uses a separate JSON file — this is a spec deviation

---

### 1.5 HTTP Request Parsing Vulnerable to Injection

**File:** `src/api/request.rs:14–56`  
**Severity:** 🟠 High

The HTTP request parser does not validate or sanitize:
- **Path traversal:** A request like `GET /record/../../identity.key HTTP/1.1` would be routed to `handle_get_record` with `id = "../../identity.key"`. While the current handler only does a DB lookup (not file I/O), any future code that resolves record IDs to file paths would be vulnerable.
- **Header injection:** Headers are parsed but never validated for length or content. A malicious header value could contain control characters.
- **Body size:** The API reads up to 65,536 bytes in a single read (`src/api/local.rs:91`) with no Content-Length validation. An attacker can send a 64KB body for any POST endpoint, consuming memory.

**Fix:** Validate paths don't contain `..`, limit header count/size, respect Content-Length, cap body size per endpoint.

---

### 1.6 Gateway Rate Limiting Uses "anonymous" for All Unauthenticated Requests

**File:** `src/api/gateway.rs:94–99`  
**Severity:** 🟡 Medium

```rust
let identifier = if let Some(ref key) = api_key {
    key.clone()
} else {
    "anonymous".to_string() // ← all unauthenticated requests share one bucket
};
```

The roadmap says: *"Rate-limited per key (falls back to per-IP if no key is presented)"*. The implementation falls back to a single "anonymous" identifier instead of per-IP tracking. This means one abusive unauthenticated client can exhaust the rate limit for all unauthenticated clients.

**Fix:** Extract the client IP from the TCP stream (available as `_addr` in `accept()`) and use it as the rate-limit identifier for unauthenticated requests.

---

### 1.7 Hand-rolled Base64 Decoder Accepts Invalid Input

**File:** `src/scraper/job.rs:271–295`  
**Severity:** 🟡 Medium

The base64 decoder doesn't validate padding or handle edge cases:
- Non-base64 characters produce an error but don't reject the entire cert loading
- Missing padding is handled (`trim_end_matches('=')`) but over-padding isn't checked
- A malformed cert in the CA bundle could silently fail, leaving the root store empty

This is low risk since it only affects the scraper's HTTPS fetch path, but a corrupted CA bundle would cause all HTTPS scrapes to fail with TLS errors.

---

## 2. CRITICAL — Race Conditions

### 2.1 TOCTOU Race in Record Deduplication

**File:** `src/storage/records.rs:21–93`  
**Severity:** 🔴 Critical

The `insert_record` function performs dedup in two separate transactions:

1. **Read transaction:** Check `source_index` for existing record → decide Inserted/ReplacedNewer/SkippedOlder
2. **Write transaction:** Delete old record (if replacing), then insert new record

Between step 1 and step 2, another concurrent insert with the same `source_hash` could complete, leading to:
- Two records with the same `source_hash` in the `source_index` (the second write overwrites the first)
- The old record not being deleted (if the second insert's read didn't see the first insert's write yet)

**Fix:** Perform the dedup check and insert in a single write transaction. redb's write transactions are serialized, so this eliminates the race.

---

### 2.2 Inverted Index Read-Then-Write Race

**File:** `src/storage/index.rs:7–46`  
**Severity:** 🟠 High

`index_record` reads the current index value in a read transaction, then writes the updated value in a separate write transaction. Two concurrent index operations for the same key could lose one update (lost update anomaly).

**Fix:** Perform the read-modify-write in a single write transaction.

---

### 2.3 Quota Check Then Insert Race

**File:** `src/storage/mod.rs:67–86`  
**Severity:** 🟡 Medium

`Store::insert_record` checks quota, then inserts. Between the quota check and the insert, another thread could insert records that push past the quota limit.

**Fix:** Move the quota check inside the same write transaction as the insert.

---

### 2.4 Peers.json Write Contention

**File:** `src/node/server.rs:267–280`  
**Severity:** 🟡 Medium

`write_peers_file` is called from multiple async tasks (inbound connect, outbound connect, disconnect). Each call reads the routing table and writes the entire file. Concurrent writes could corrupt the file or lose updates.

**Fix:** Use a dedicated `tokio::sync::Mutex` around file writes, or batch writes with a debounce channel.

---

## 3. HIGH — Missing Roadmap Features

### 3.1 DHT Fan-out for Search (Phase 4 Core Feature)

**Status:** ❌ Not implemented

The roadmap specifies: *"Query fans out via DHT to K=20 nearest peers"*. The current `search_records` only searches the local Tier 3 store. There is no code to:
- Send `SearchQuery` messages to peers
- Receive and aggregate `SearchReply` results
- Deduplicate results by record ID across peers
- Stream results back to the caller

The `SearchQuery`, `SearchReply`, `SearchResult` structs exist in `src/proto/msg_type.rs` but are never constructed (compiler confirms dead code).

**Files affected:** `src/search/query.rs`, `src/node/server.rs`, `src/api/handlers.rs`

---

### 3.2 Record Fetch from Remote Holders (Phase 4)

**Status:** ❌ Not implemented

The roadmap specifies: *"User clicks a result → node fetches full ContentRecord directly from the announcing node"*. The `RecordFetch` and `RecordReply` message types exist but are never used. There is no code to:
- Open a QUIC stream to a peer
- Send a `RecordFetch` message
- Receive and verify the `RecordReply`

---

### 3.3 Announcement Protocol (Phase 5 Core Feature)

**Status:** ❌ Stub only

The `Announce` and `AnnounceAck` message types exist but are never sent or received over the wire. The `handle_messages_inner` in `server.rs:407` only handles `Ping`, `FindNode`, and `Goodbye` — `Announce` falls through to the `_ => { debug!("Ignoring unknown message type") }` arm.

The CLI `dsearch record announce` just writes to the local Tier 2 store with an empty `sig` field — it doesn't send anything to peers.

---

### 3.4 Replication Push (Phase 5)

**Status:** ❌ Stub only (`src/scraper/replicate.rs` is a one-line comment)

The roadmap specifies Archive nodes accept `ReplicatePush` messages, verify the signature, and re-announce themselves as additional holders. None of this exists.

---

### 3.5 Keyword Discovery / Search Providers (Phase 5)

**Status:** ❌ Stub only

`src/scraper/discovery/providers.rs`, `html_scrape.rs`, and `api.rs` are each one-line comments. The roadmap specifies a full pluggable search provider system with DDG as the default, UA rotation, cookie jars, backoff, and `search_providers.toml` parsing. None of this exists.

---

### 3.6 AutoNAT Probe (Phase 1)

**Status:** ❌ Stub only (`src/node/autonat.rs` is a two-line comment)

The roadmap specifies: *"ask connected peers 'can you dial me back?'"*. The onboarding wizard hardcodes `self.autonat_result = Some(false)` — it never actually probes.

---

### 3.7 STUN Role (Phase 1)

**Status:** ❌ Stub only (`src/node/stun.rs` is a two-line comment)

The roadmap specifies: *"~50 LOC, stateless"*. Not implemented.

---

### 3.8 DNS SRV Bootstrap Resolution (Phase 1)

**Status:** ❌ Comment placeholder

`src/bootstrap/resolver.rs:33` has a comment `// 2. DNS SRV lookup (placeholder for Phase 1)` but no implementation. The roadmap specifies: `Try DNS SRV: _dsearch._udp.dsearch.network`.

---

### 3.9 Log Streaming (Phase 7)

**Status:** ❌ Not implemented

`src/main.rs:77–80`:
```rust
Commands::Log { .. } => {
    eprintln!("Log streaming not yet implemented");
    std::process::exit(1);
}
```

The roadmap specifies `GET /log/stream` — chunked, one JSON log line per chunk (debug builds only).

---

### 3.10 Storage Vacuum / Export / Import

**Status:** ❌ Stub

`src/api/handlers.rs:247–249`:
```rust
fn handle_storage_vacuum(node_id: &str) -> HttpResponse {
    let body = serde_json::json!({ "ok": true, "message": "vacuum not yet implemented" });
```

The CLI commands `dsearch storage vacuum/export/import` are not implemented.

---

### 3.11 Scraper Job Pause/Resume/Remove/Status

**Status:** ❌ Partially missing

The CLI only implements `Add`, `List`, and `Run` for scraper jobs. The roadmap specifies: `dsearch scraper job list/remove/pause/resume/run/status`. The `ScraperAction` enum in `src/cli/cmd.rs` only has `Add`, `List`, and `Run`.

---

### 3.12 Periodic Scraper Job Execution

**Status:** ❌ Not implemented

Scrape jobs can only be run manually via `dsearch scraper job run`. The roadmap specifies `interval` and `on-change` refresh policies with automatic periodic execution. There is no scheduler/timer system.

---

### 3.13 Feed and API Source Types

**Status:** ❌ Not implemented

`run_url_job` in `src/scraper/job.rs` only handles `url`-source jobs. The `feed` and `api` source types have no implementation. The `keyword` source type discovery is also a stub.

---

### 3.14 Transforms

**Status:** ❌ Not implemented

The roadmap specifies transforms: `crates_io_v1`, `wikimedia_dump`, `html_strip`, `weather_v1`. The `ScrapeJob.transform` field exists but is never used.

---

## 4. HIGH — Spec Deviations

### 4.1 Routing Table Is Not Kademlia K-Buckets

**File:** `src/node/dht.rs:22–27`  
**Severity:** 🟠 High

The roadmap specifies: *"Kademlia k-buckets: node_id → address"*. The implementation uses a flat `BTreeMap<String, RoutingEntry>` — not k-buckets. The comment even says: *"simplified for Phase 1 — full k-buckets later"*.

**Impact:** 
- No bucket-based distance ordering (the `find_closest` function sorts by first 8 bytes of XOR distance, losing precision for 32-byte IDs)
- No bucket capacity limits (k=20 per bucket)
- No bucket refresh mechanism
- The `K_BUCKET_SIZE` and `MAX_NODE_ID_BITS` constants are defined but unused

---

### 4.2 XOR Distance Comparison Truncates to 8 Bytes

**File:** `src/node/dht.rs:61–70`

```rust
entries.sort_by_key(|e| {
    let dist = Self::xor_distance(&e.node_id, target_id);
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&dist[..8]); // ← only first 8 bytes!
    u64::from_be_bytes(arr)
});
```

This means two node IDs that differ only in bytes 9–32 would be considered equidistant. For a 32-byte (256-bit) node ID space, this is a significant loss of precision.

**Fix:** Use the full 32-byte distance for comparison, or at minimum use a 256-bit integer comparison.

---

### 4.3 Announcement Key Format Differs from Spec

**File:** `src/storage/records.rs:254`

```rust
let key = format!("{}:{}", ann.record_id, ann.holder_addr);
```

The roadmap says Tier 2 is `record_id → [node_addr, ...]` (one-to-many). The implementation uses a composite key `record_id:holder_addr`, which means:
- Multiple holders of the same record create separate entries (not a list)
- Listing all holders of a record requires scanning all keys with a prefix
- The 256-byte announcement size limit may be exceeded by the composite key

---

### 4.4 Config Save Doesn't Preserve `[meta]` Section

**File:** `src/config/mod.rs:275–282`

```rust
pub fn save_config(data_dir: &Path, config: &DsearchConfig) -> Result<(), String> {
    let toml_str = toml::to_string_pretty(config)?;
    std::fs::write(&config_path, toml_str)?;
    Ok(())
}
```

`DsearchConfig` doesn't include the `[meta]` section (it's not a serde field). When `save_config` is called (e.g., from `config set`), it overwrites the file **without** the `[meta]` section, losing `config_version`. The next startup then treats it as version 0 and re-runs migrations.

Only `cmd_init` manually appends `[meta]\nconfig_version = ...` after saving. All other save paths lose it.

**Fix:** Either include `config_version` in `DsearchConfig` or always append the meta section on save.

---

### 4.5 `source_hash` Not Normalized Per Roadmap

**File:** `src/trust/sign.rs:106–108`

```rust
pub fn compute_source_hash(source_url: &[u8]) -> String {
    blake3::hash(source_url).to_hex().to_string()
}
```

The roadmap specifies: *"The source_hash is Blake3 of the canonical source URL (normalised: lowercase, stripped of tracking params)"*. The implementation hashes the raw URL with no normalization. Two URLs that differ only in case or tracking parameters (`?utm_source=...`) will produce different hashes, defeating dedup.

---

### 4.6 `record_id` Computation Doesn't Match Roadmap

**File:** `src/trust/sign.rs:93–103`

The roadmap says: *"Computes record_id (Blake3 of canonical content fields)"* and lists the signed fields as `sign(id, source_url, source_hash, schema, tags, body, created_at, expires_at, scrape_source, refresh_policy)`.

But `compute_record_id` uses: `source_url, source_hash, schema, tags, body, created_at` — it excludes `id`, `expires_at`, `scrape_source`, and `refresh_policy`. This is actually correct per the roadmap's intent (the record_id should be content-derived, not include the ID itself), but the discrepancy between the sign function's field list and the record_id function's field list should be documented.

---

### 4.7 `title:` Search Filter Maps to `id + source_url`

**File:** `src/search/query.rs:205–213`

```rust
"title" => {
    // "title" maps to the record id or source_url for now
    // (ContentRecord doesn't have a separate title field)
    let searchable = format!("{} {}", record.id, record.source_url).to_lowercase();
    if !searchable.contains(value) {
        return false;
    }
}
```

The roadmap's search query language specifies `title:rust` as a field-scoped filter, but `ContentRecord` has no `title` field. The current fallback to `id + source_url` is misleading — a user searching `title:rust` would get matches on record IDs containing "rust" rather than actual titles.

---

### 4.8 `since:`/`before:` Date Parsing Is Approximate

**File:** `src/search/query.rs:136–165`

The date-to-unix-timestamp conversion uses an approximate formula (`365.25-day years`) rather than proper calendar arithmetic. This produces incorrect timestamps for dates near leap year boundaries, and the epoch calculation (`1970 * 365 + ...`) is off by ~1 day from the actual Unix epoch.

**Fix:** Use the `chrono` crate or implement proper calendar arithmetic.

---

## 5. MEDIUM — Race Conditions & Concurrency Issues

### 5.1 Shutdown Race: `running` Flag Checked Without Synchronization

**File:** `src/node/server.rs:394–403`

The `running` AtomicBool is checked in a `select!` branch with a 500ms sleep, but the actual frame read (`read_frame`) blocks indefinitely. If `running` is set to false while `read_frame` is blocked, the node won't shut down until the next 500ms timeout — or until the peer sends a frame. This is a liveness issue, not a correctness issue, but it means shutdown can take up to 500ms per peer even with the `select!`.

---

### 5.2 Expiry Sweeper Accesses redb from Async Context

**File:** `src/storage/expiry.rs:9–45`

The expiry sweeper runs as a tokio task but calls `delete_expired_records` and `delete_expired_announcements`, which are synchronous redb operations. redb's write transactions block the tokio thread while waiting for the write lock. Under heavy load, this could stall the async runtime.

**Fix:** Run the sweep in `spawn_blocking` or use a dedicated background thread.

---

### 5.3 UI Refreshes API Every Frame (500ms)

**File:** `src/ui/mod.rs:129–131`

```rust
fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    ctx.request_repaint_after(std::time::Duration::from_millis(500));
    self.refresh_status();
}
```

`refresh_status` makes 3 synchronous HTTP calls (`/node`, `/storage`, `/health`) every 500ms. These are blocking `std::net::TcpStream` calls on the UI thread. If the API is slow or unresponsive, the UI will freeze.

**Fix:** Use async HTTP calls or a background thread with a channel to update UI state.

---

## 6. MEDIUM — Code Quality Issues

### 6.1 74 Compiler Warnings (Dead Code)

The build produces 74 warnings, almost all dead-code warnings for Phase 4–9 features that are implemented but not yet wired in. While this is expected for a phased build, the volume makes it hard to spot new warnings. Consider `#[allow(dead_code)]` on modules that are intentionally stubbed.

---

### 6.2 `url_encode` Is Incomplete

**File:** `src/main.rs:930–935`

```rust
fn url_encode(s: &str) -> String {
    s.replace(' ', "+")
     .replace('#', "%23")
     .replace('&', "%26")
     .replace('=', "%3D")
}
```

This only encodes 4 characters. Per RFC 3986, many more characters need encoding in query parameters: `%`, `+`, `/`, `?`, `[`, `]`, etc. A query like `rust/crate` would not be properly encoded.

---

### 6.3 HTTP Response Doesn't Include `Content-Length` Before Body

**File:** `src/api/response.rs:77–100`

The `to_bytes` method writes `content-length` correctly, but the header name is lowercase (`content-length`) while other headers use lowercase too. This is fine per HTTP/1.1 spec, but the `content-type` header is set both in the `json()` constructor and potentially overwritten in `handle_openapi` — there's a risk of duplicate `content-type` headers.

---

### 6.4 `ScraperAction` Missing CLI Subcommands

**File:** `src/cli/cmd.rs:306–338`

The roadmap specifies: `dsearch scraper job add/list/remove/pause/resume/run/status`. The CLI only has `Add`, `List`, and `Run`. Missing: `Remove`, `Pause`, `Resume`, `Status`.

---

### 6.5 `SearchProvider` CLI Commands Not Defined

The roadmap specifies: `dsearch search-provider list/add/enable/disable/set-priority/test/remove`. None of these CLI subcommands exist in `src/cli/cmd.rs`.

---

### 6.6 `Replicate` CLI Commands Not Defined

The roadmap specifies: `dsearch replicate push {id} --factor 3` and `dsearch replicate status {id}`. Not implemented.

---

### 6.7 `Schema` CLI Commands Not Defined

The roadmap specifies: `dsearch schema list / show {id}`. Not implemented as CLI commands (only as API routes).

---

### 6.8 Node Restart Is a No-Op

**File:** `src/main.rs:331–334`

```rust
NodeAction::Restart => {
    println!("Node restart: stop the node first with `dsearch node stop`, then `dsearch node start`");
    Ok(())
}
```

The roadmap lists `dsearch node restart` as a real command. This should stop then start the node.

---

### 6.9 `cmd_role` Doesn't Persist Role Changes

**File:** `src/main.rs:455–487`

`cmd_role set/add/remove` prints a message but doesn't write to `config.toml`. The role change is lost on restart.

---

### 6.10 Bootstrap `Test` Is Not Implemented

**File:** `src/main.rs:396–398`

```rust
BootstrapAction::Test => {
    println!("Bootstrap test: not yet implemented (requires running node)");
    Ok(())
}
```

The roadmap specifies: *"probe each peer, show latency + reachable"*.

---

### 6.11 `PeersAction::Add` Not Implemented

**File:** `src/main.rs:440–442`

```rust
PeersAction::Add { .. } => {
    println!("Peer add: not yet implemented (requires running node)");
    Ok(())
}
```

---

### 6.12 `PeersAction::Ban/Unban` Not Implemented

**File:** `src/main.rs:444–451`

Marked as "Phase 9" but the reputation system code exists and is tested — it's just not wired in.

---

## 7. MEDIUM — Architectural Concerns

### 7.1 redb Database Opened Multiple Times

**Files:** `src/main.rs:152`, `src/main.rs:723`, `src/main.rs:898`, `src/main.rs:1025`

The redb database is opened separately in `cmd_node` (for the API server), and again in `cmd_record` and `cmd_search` (for direct store access when the API isn't reachable). redb supports multiple readers but only one writer at a time. If the node is running (holding a write transaction) and the CLI tries a direct store access, it will block or fail.

**Fix:** Always go through the API when the node is running. The current fallback to direct DB access is a convenience for offline use but creates contention risk.

---

### 7.2 No Backpressure on API Connections

**File:** `src/api/local.rs:64–81`

The API server spawns a new tokio task for every incoming TCP connection with no limit. Under load, this could exhaust file descriptors or memory.

**Fix:** Add a `Semaphore` or connection counter with a reasonable limit (e.g., 100 concurrent API connections).

---

### 7.3 No Backpressure on QUIC Connections (Partial)

**File:** `src/node/server.rs:127–135`

The QUIC accept loop does enforce a connection pool cap (`max_connections`), but when the pool is full, it accepts the connection and immediately closes it. This is correct behavior per the code comment, but the close reason code `0u32.into()` is opaque — the remote side gets no useful error message.

---

### 7.4 Inbound Message Channel Never Consumed

**File:** `src/node/server.rs:82`

```rust
let (inbound_tx, _inbound_rx) = mpsc::channel::<Vec<u8>>(256);
```

The `inbound_tx` is passed to `handle_connection` but `_inbound_rx` is never read. The channel exists for backpressure but the receiving end is dropped, meaning messages sent to it will eventually fail silently when the buffer fills.

---

### 7.5 Search Cache Never Used

**File:** `src/search/cache.rs`

`SearchCache` is fully implemented and tested but never instantiated anywhere in the codebase. The roadmap specifies: *"Search result caching at the querying node, short TTL, so repeat queries don't re-trigger full K=20 fan-out"*.

---

### 7.6 Tier 2 Rate Limiter Never Used

**File:** `src/storage/tier2_limiter.rs`

`Tier2RateLimiter` is fully implemented and tested but never instantiated. The roadmap specifies: *"Tier 2 write-rate limiting per peer (separate from the existing size cap)"*.

---

### 7.7 PoW Sybil Resistance Never Used

**File:** `src/trust/pow.rs`

`mine_pow` and `verify_pow` are implemented and tested but never called. The roadmap specifies: *"Sybil resistance: PoW node ID"*.

---

### 7.8 `RelayBandwidthAccount` Never Used at Runtime

**File:** `src/node/relay.rs`

Fully implemented and tested but never instantiated in the relay path. The relay role itself is not implemented.

---

## 8. LOW — Minor Issues

### 8.1 `BootstrapPeer` IDs Are Placeholders

**File:** `src/bootstrap/defaults.rs:16–18`

```rust
id: "placeholder_bootstrap_1".to_string(),
```

These are not real Ed25519 public key hashes. Any node trying to verify these will fail.

---

### 8.2 `parse_date_value` Doesn't Handle Month Overflow

**File:** `src/search/query.rs:153`

If `m` (month) is 0 or > 12, the `month_days` array access could panic (index out of bounds for 0) or produce wrong results for > 12.

---

### 8.3 Windows HTTPS Scraper Will Fail Without CA Bundle

**File:** `src/scraper/job.rs:241–251`

The hand-rolled HTTPS scraper on Windows tries to load CA certs from `C:\ProgramData\curl\ca-bundle.crt` or `C:\curl\ca-bundle.crt`. Most Windows installations don't have curl's CA bundle at these paths. The code has a TODO comment acknowledging this.

---

### 8.4 `eframe::App::logic` May Not Exist in All Versions

**File:** `src/ui/mod.rs:129`

The `logic` method on `eframe::App` may not be available in all egui/eframe versions. The standard entry point is `update(&mut self, ctx, frame)`. If this compiles, it's fine, but it's worth verifying against the pinned eframe version.

---

### 8.5 `cmd_init` Doesn't Create `search_providers.toml`

The roadmap specifies `search_providers.toml` should be created with defaults on first run. `cmd_init` creates `config.toml` and `bootstrap.toml` but not `search_providers.toml`.

---

### 8.6 Onboarding Wizard Doesn't Actually Connect to Bootstrap Peers

**File:** `src/ui/onboarding.rs:251–304`

The "Connect to Network" step resolves bootstrap peers but never actually connects. Clicking "Retry" just sets `bootstrap_connected = true` and moves on. The onboarding claims "Connected to N peer(s)" but no QUIC connection is made.

---

### 8.7 `dsearch tray start` Launches Full UI Instead of Tray

**File:** `src/main.rs:1118–1128`

```rust
TrayAction::Start => {
    ui::run_ui(data_dir.clone())?;
    Ok(())
}
```

The roadmap specifies the tray mode should run with a hidden window and only a system tray icon. The current implementation launches the full UI.

---

### 8.8 `dsearch tray stop` Not Implemented

**File:** `src/main.rs:1124–1126`

Just prints a message. No mechanism to signal the tray process to stop.

---

### 8.9 Tray Quit Uses `std::process::exit(0)`

**File:** `src/ui/tray.rs:33`

```rust
"quit" => {
    std::process::exit(0);
}
```

This bypasses the graceful shutdown path (sending `Goodbye` to peers, draining streams, cleaning up PID files). The roadmap specifies: *"Quit — clean shutdown (same Goodbye-then-close path as `dsearch node stop`)"*.

---

### 8.10 `doctor` Check Uses Wrong Port for QUIC

**File:** `src/doctor/mod.rs:206–214`

The doctor's network check uses the gateway bind port (from `config.gateway.bind`) as the QUIC port, but the QUIC port defaults to 7744 while the gateway defaults to `0.0.0.0:7744`. These happen to be the same number but are conceptually different — the QUIC port should come from the `node start --port` flag or a separate config key.

---

## 9. Summary Table

| Category | Critical | High | Medium | Low | Total |
|----------|----------|------|--------|-----|-------|
| Security | 2 | 2 | 2 | 0 | 6 |
| Race Conditions | 1 | 1 | 2 | 0 | 4 |
| Missing Features | 0 | 8 | 0 | 0 | 8 |
| Spec Deviations | 0 | 4 | 0 | 0 | 4 |
| Code Quality | 0 | 0 | 4 | 0 | 4 |
| Architecture | 0 | 0 | 4 | 0 | 4 |
| Minor | 0 | 0 | 0 | 10 | 10 |
| **Total** | **3** | **15** | **12** | **10** | **40** |

---

## 10. Recommended Fix Priority

### Immediate (blocks any real network use)
1. **Implement real certificate verification** (`src/proto/cert.rs`) — without this, the trust model is completely broken
2. **Wire signature signing/verification into announce and record paths** — without this, any node can forge anything
3. **Fix TOCTOU race in record dedup** — merge read+write into single write transaction
4. **Set file permissions on identity.key** — private key must not be world-readable

### Before Multi-Node Testing
5. **Implement DHT fan-out for search** — this is the core value proposition
6. **Implement announcement protocol over QUIC** — without this, nodes can't discover each other's content
7. **Implement record fetch from remote holders** — without this, search results can't be retrieved
8. **Fix inverted index race condition** — merge read+write into single write transaction
9. **Normalize source URLs before hashing** — dedup is broken without this
10. **Preserve `[meta]` section in config saves** — config_version loss causes silent re-migration

### Before Public Network Exposure
11. **Add per-IP rate limiting on gateway** — current "anonymous" bucket is a DoS vector
12. **Validate HTTP request paths** — prevent path traversal
13. **Add API connection limit** — prevent resource exhaustion
14. **Wire in search cache** — prevents query amplification on repeat searches
15. **Wire in Tier 2 rate limiter** — prevents announcement flooding

### Before 1.0
16. Implement k-bucket routing table
17. Implement AutoNAT probe
18. Implement STUN role
19. Implement DNS SRV bootstrap resolution
20. Implement keyword discovery / search providers
21. Implement replication push
22. Implement periodic scraper job scheduler
23. Implement feed/API source types
24. Implement transforms
25. Implement log streaming
26. Implement storage vacuum/export/import
27. Implement missing CLI subcommands (scraper pause/resume/remove/status, search-provider *, replicate *, schema *)
28. Fix `title:` search filter (add title field to ContentRecord or document the limitation)
29. Fix date parsing in search queries
30. Fix `url_encode` to be RFC 3986 compliant
