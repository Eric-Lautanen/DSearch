# Decentralized Search Network — Roadmap

---

## Build philosophy (read this before writing any code)

These are standing constraints, not a one-time checklist — they apply to
every phase below, not just the phases that mention them explicitly.

### Lightweight, resource-friendly, minimal deps
- Target footprint stated in Phase 9 (<50 MB idle, <200 MB loaded) is a
  **running constraint to check after every phase**, not a final-pass
  cleanup item. If a phase's work visibly fattens idle memory or adds a
  background poll loop, fix it before moving on — don't defer it to
  "hardening will catch it."
- Default to writing it yourself over adding a crate. The bar: hand-roll
  anything that's plain protocol/format logic this doc already specifies
  (the wire framing, the HTTP/1.1 server, the query parser, the cert
  verifier) — that's the whole point of those being custom in the first
  place. Pull in a dependency only for things that are genuinely
  dangerous or wasteful to reimplement: cryptographic primitives
  (`ed25519-dalek`, `blake3`), the QUIC/TLS stack itself (`quinn`),
  the embedded DB engine (`redb`). When in doubt, ask: "is this crate
  replacing a primitive I shouldn't be hand-rolling, or just saving me
  20 lines of glue code?" Only the former justifies the dependency.
- Every dependency added beyond what's already listed in **Cargo.toml**
  needs a one-line justification comment at its point of use explaining
  which of the two categories above it falls into.

### Build order is a gate, not a suggestion
- Phases are numbered because they're layered, not because of
  arbitrary preference — Phase 4 (search) cannot be meaningfully tested
  without Phase 1's transport and Phase 3's storage actually working.
  Do not start a phase's work until the previous phase's exit test
  (below) passes.
- If a later phase reveals a gap in an earlier one (e.g. building
  Phase 5's announce logic surfaces a missing field in Phase 2's
  `Announcement` struct), fix the earlier phase directly and re-run
  that phase's exit test — don't patch around it forward.

### Test the CLI as you build it, not after
For an autonomous agent, "write tests" is too vague to act on. The
concrete version: **every time a phase adds or changes a CLI-surfaced
behavior, actually run that CLI command against a real local node and
check the output**, before considering the phase done or starting the
next one. This is how the build catches the two known dangerous-silent
failure modes (broken wire protocol bytes, an unimplemented cert
verifier) at the phase where they're introduced, instead of six phases
later when something downstream mysteriously doesn't work.

Each phase below ends with a short **Exit test** — the minimum CLI
session that has to behave correctly before the phase counts as done.
Treat it as a gate: if the exit test doesn't pass, the phase isn't
finished, regardless of how much code has been written.

---

## Core mental model

Every node maintains exactly three data structures. Understanding what lives
where is the foundation of every design decision.

```
┌─────────────────────────────────────────────────────────────┐
│ Tier 1 — Routing table                                      │
│   Kademlia k-buckets: node_id → address                     │
│   Who is on the network and where they live.                │
│   Small. Every node holds this. No content here.            │
├─────────────────────────────────────────────────────────────┤
│ Tier 2 — Announcement index                                 │
│   record_id → [node_addr, ...]                              │
│   Which nodes claim to hold which content.                  │
│   Pointers only — not the content itself.                   │
│   Populated by record announcements from scraping nodes.    │
├─────────────────────────────────────────────────────────────┤
│ Tier 3 — Content store                                      │
│   The actual ContentRecord data.                            │
│   A node ONLY holds what it scraped, pinned, or accepted    │
│   as an optional replication target.                        │
│   This is local. No node holds everyone else's content.     │
└─────────────────────────────────────────────────────────────┘
```

**Consequence:** the network only knows about content that at least one node
has scraped and is currently online. If the sole holder of a record goes
offline, that record is temporarily unfindable — until the node returns or
another node has scraped the same source. This is the decentralized contract,
not a bug.

---

## How search works end-to-end

```
User types query
       │
       ▼
Local node searches Tier 3 (own content) instantly
       │
       ▼
Query fans out via DHT to K=20 nearest peers (Tier 1 routing)
       │
       ├── Each peer searches their own Tier 3
       ├── Each peer checks their Tier 2 announcement index
       └── Results (summaries: id, title, schema, tags) stream back
              │
              ▼
       Node aggregates, deduplicates, ranks, displays
              │
              ▼
       User clicks a result → node fetches full ContentRecord
       directly from the announcing node (Tier 2 pointer → Tier 3 fetch)
```

No node fetches or caches another node's content speculatively.
Full records are only pulled on explicit request.

---

## Record lifecycle

### 1. Scrape
A Scraper node fetches external content, runs a transform,
passes through `sanitize()`, writes to its own Tier 3 store.
A `source_url` field is stored alongside the record so duplicate
detection can work across nodes.

### 2. Deduplication
Before writing, the node checks its `source_index` table:
- If `source_hash` already exists → compare `created_at`; keep newer, drop older
- If new → write and proceed to announce
- The `source_hash` is Blake3 of the canonical source URL (normalised:
  lowercase, stripped of tracking params)

### 3. Announce
After writing, the node announces to the DHT:
- Computes `record_id` (Blake3 of canonical content fields)
- Finds K nodes whose ID is closest to `record_id`
- Signs the announcement (see **Trust model** below) and sends:
  `{ record_id, source_hash, schema, tags, holder_addr, expires_at, sig }`
- Those peers verify `sig`, then store `record_id → holder_addr` in Tier 2
- Peers receiving an announcement with a known `source_hash` but newer
  `created_at` update their Tier 2 pointer to the fresher holder, provided
  the new announcement's signature also verifies
- Announcement TTL matches record expiry; re-announced periodically

### 4. Discovery
Search fan-out hits nodes holding Tier 2 pointers → returns summaries.
Querying node fetches full record from holder on user request only.
On fetch, the querying node verifies the record's `sig` (see **Trust
model**) before displaying or storing it.

### 5. Optional replication (Archive nodes)
- Archive nodes advertise willingness in their DHT peer record
- Scraping nodes push full copies to N Archive nodes (per-scraper config)
- Archive nodes verify `sig` on the pushed record before accepting it,
  then re-announce themselves as additional holders
- Light nodes never accept replication pushes
- Default replication factor: 0

### 6. Expiry
- Ephemeral records: removed from Tier 3 by sweep task; Tier 2 entries
  age out via TTL
- Pinned records: never expire; re-announced at `TTL / 2` so a missed
  cycle never causes the Tier 2 entry to age out before the next attempt
- Offline holder: Tier 2 entries age out; record unfindable until holder
  returns or a replica exists

---

## Scrape sources

Step 1 of the record lifecycle ("Scrape") starts with the operator
deciding *what* to scrape and *how often*. This section defines the
three orthogonal choices a scraper job makes, independent of each other,
plus the DDG-backed keyword mode and the new `ScrapeJob` config object
they're all driven by.

### Three independent axes

Every scrape job is a combination of all three — they don't imply each
other (the weather example is `api` + `interval` + `ephemeral`; a pasted
article is `url` + `once` + `pinned`):

**1. Source — where the content comes from**
| Source | Input | Notes |
|---|---|---|
| `url` | operator pastes a URL | single-page scrape; the baseline source type all others resolve down to |
| `feed` | RSS/Atom URL | naturally periodic; entries become individual records |
| `api` | endpoint + named transform | structured sources like weather, crates.io |
| `keyword` | search phrase | resolves to a batch of `url` jobs via DDG discovery (below) — not a content source itself |

**2. Refresh policy — when it re-runs**
| Policy | Behavior |
|---|---|
| `once` | scrape, store, done — no re-run |
| `interval` | cron-style fixed schedule, e.g. every 15 min |
| `on-change` | cheap conditional check (HTTP `ETag`/`Last-Modified`) on the interval; full scrape only fires if the source actually changed |

**3. Lifecycle — how long the record lives**
| Lifecycle | Behavior |
|---|---|
| `ephemeral` | short TTL (job-configurable, default 1h); expected to be superseded by the next refresh |
| `pinned` | never expires; re-announced at `TTL / 2` per the Expiry rules above |

### Keyword discovery — pluggable search providers

`keyword` jobs are a **resolver**, not a parallel scraping pipeline — they
sit in front of the `url` source and produce a batch of `url` jobs. The
provider that does the resolving is swappable, not hardcoded — there are
many search backends, operators will have preferences (privacy posture,
rate limits, self-hosted vs public), and a single hardcoded provider
becomes a single point of breakage if that provider changes their page
or blocks the node:

```
User enters keyword/phrase, hits enter
       │
       ▼
Active provider (config-selected; DDG by default) resolves the phrase
to a batch of result URLs — mechanism depends on provider `kind`:
  html_scrape → scrape the provider's own results page through the
                same sanitize() pipeline as any other source
  api         → call the provider's documented search API directly
       │
       ▼
Parse/extract result URLs (capped per-provider, see search_providers.toml)
       │
       ▼
Each result URL becomes a `url`-source scrape job, inheriting the
parent keyword job's refresh policy and lifecycle
```

### Provider config — `search_providers.toml`

Lives at `{data_dir}/search_providers.toml`, same spirit as
`bootstrap.toml`: user-editable, ships with one built-in default, never
silently overwritten by an update.

```toml
# {data_dir}/search_providers.toml
# Providers are tried in list order; if the active provider fails
# (blocked, rate-limited, parse failure) the next one in the list is
# tried automatically for that job. Add as many as you want.

[[providers]]
name              = "duckduckgo"
kind              = "html_scrape"        # html_scrape | api
enabled           = true
priority          = 1                    # lower = tried first
target            = "https://html.duckduckgo.com/html/?q={query}"
result_selector   = "a.result__a"        # CSS selector for result links
max_results       = 10
min_interval_secs = 30                   # rate cap, see note below
ua_rotation       = true
cookie_jar        = true

[[providers]]
name              = "searxng-self-hosted"
kind              = "api"
enabled           = false                # off until operator points it at their instance
priority          = 2
target            = "https://searx.example.org/search?q={query}&format=json"
max_results       = 10
min_interval_secs = 5                    # self-hosted: operator's own rate limit, not a public site's
api_key           = ""

# [[providers]]
# name            = "brave-search-api"
# kind            = "api"
# enabled         = false
# target          = "https://api.search.brave.com/res/v1/web/search?q={query}"
# api_key         = ""                    # paid API; requires operator's own key
```

For `kind = "html_scrape"` providers — scraping a search engine's own
results page rather than a sanctioned API — each is inherently
adversarial to that provider's bot detection. This is a best-effort
component that will need ongoing maintenance as each provider's page
changes, not a permanently solved problem per provider. Mitigations,
applied per-provider using that provider's own settings above:
- **UA rotation**: pool of realistic user-agent strings, rotated per request
- **Cookie jar**: persistent per-provider, per-scraper-instance cookie
  jar (not shared across providers or jobs) so each looks like a
  continuing browser session rather than a fresh bot hit every time
- **Backoff**: exponential backoff on HTTP 429 or a detected
  CAPTCHA/block page; job pauses and retries on its next scheduled
  interval rather than hammering immediately, then fails over to the
  next-priority provider if backoff exhausts retries
- **Rate cap**: `min_interval_secs` per provider — public scrape
  targets default conservative, self-hosted/API targets can be tighter
  since the operator controls or pays for that endpoint

`kind = "api"` providers skip the scrape-specific mitigations (no UA
rotation or cookie jar needed against your own SearXNG instance or a
paid API) but still respect `min_interval_secs` and still feed results
through `sanitize()` before becoming scrape jobs — provider output is
untrusted input like any other external source.

### CLI / UI for providers

Full command set in **UI ↔ CLI parity** below
(`dsearch search-provider list/add/enable/disable/set-priority/test/remove`).

UI: Settings → "Search providers" panel, same shape as the Bootstrap
peers panel — table with source/priority/enabled columns, add/remove,
a "Test" button per row. Most users will never touch this and will
just paste URLs directly via the `url` source type — keyword discovery
and its provider list are there for the minority who want to seed scrape
jobs from a phrase instead of a link.

### `ScrapeJob` config object

Replaces the flat `[scraper]` defaults with a list of named jobs (full
`config.toml` shape in **Config file** below):

```toml
[[scraper.jobs]]
name              = "local-weather"
source            = "api"
target            = "https://api.weather.example/v1/current?loc=..."
transform         = "weather_v1"
refresh           = "interval"
interval_secs     = 900              # 15 min
lifecycle         = "ephemeral"
ttl_secs          = 3600

[[scraper.jobs]]
name              = "rust-crate-news"
source            = "keyword"
target            = "rust async runtime benchmarks"
# resolved via search_providers.toml's enabled providers, in priority
# order — this job doesn't name a provider, the provider list does
max_results       = 10
refresh           = "interval"
interval_secs     = 86400            # re-search daily
lifecycle         = "pinned"

[[scraper.jobs]]
name              = "saved-article"
source            = "url"
target            = "https://example.com/article"
refresh           = "once"
lifecycle         = "pinned"
```

### Structured data and search index impact

The source/refresh/lifecycle choices need to be visible in search, not
just used internally by the scraper:

- `ContentRecord` gains `scrape_source` (`url`/`feed`/`api`/`keyword`)
  and `refresh_policy` (`once`/`interval`/`on-change`) fields, signed as
  part of the record like every other field (see **Trust model**)
- Query language gains a corresponding filter:
  `scraped:keyword`, `scraped:feed`, `refresh:interval` — same pattern
  as the existing `schema:` / `tag:` filters (kept distinct from the
  existing `source:domain` filter, which filters by source *domain*,
  not scrape mechanism)
- Ranking gains no new factor from this — freshness and holder count
  already capture "how current is this," which is what a refresh policy
  ultimately signals to a searcher

### CLI / UI

Full command set in **UI ↔ CLI parity** below (`dsearch scraper job
add/list/remove/pause/resume/run/status`).

UI: a "New scrape job" form in Settings — source-type dropdown reveals
only the relevant fields (URL field for `url`/`feed`, search box for
`keyword`, endpoint + transform picker for `api`), refresh and lifecycle
as a second step. Same one form covers the weather-every-15-minutes case
and the paste-a-URL case — the difference is just which fields are filled in.

---

## Trust model

Two independent objects carry a `sig` field — `ContentRecord` and
`Announcement` — and they protect against two different attacks. This
section defines who signs what, what exactly is covered by the signature,
when a receiving node checks it, and what happens when a check fails.

### Identity and signing key

- A node's signing key is its Ed25519 identity keypair, generated at
  first run and stored at `{data_dir}/identity.key` (see **First-run /
  onboarding flow**).
- The node's public key is also its `node_id` (see **TLS / certificate
  strategy** — the same keypair backs both the QUIC cert and content
  signatures, so verifying one inherently authenticates the other).
- There is exactly one signing identity per node. Multi-role nodes
  (e.g. Scraper + Archive) sign everything with the same key.

### What gets signed

**`ContentRecord.sig`** — signed by the Scraper node that produced the
record (i.e. the node that ran `sanitize()` and wrote it to its own
Tier 3). The signature covers the canonical byte encoding of every field
*except* `sig` itself:
```
sign(id, source_url, source_hash, schema, tags, body, created_at, expires_at)
```
This is what lets any node, anywhere, later confirm "this content really
did come from the scraper that claims to hold it" — independent of who is
relaying it.

**`Announcement.sig`** — signed by the announcing node (the current
holder, i.e. `holder_addr`'s owner) using the same per-node key. Covers:
```
sign(record_id, source_hash, schema, tags, holder_addr, expires_at)
```
This proves "the node at `holder_addr` is the one asserting it holds
`record_id`" — it does **not** prove the holder actually has valid
content, only that the holder's own node signed the claim. That gap is
intentional and is closed at fetch time, not at announce time (see
below).

Canonical encoding for both: fields concatenated in struct-declaration
order, each length-prefixed (`u32` BE), UTF-8 for strings — the same
framing discipline as the wire protocol, so there is one encoding rule
to reason about rather than two.

### Verification — when it happens

| Event | Verifier checks | On failure |
|---|---|---|
| Node receives `Announce` | `sig` against the node_id embedded in `holder_addr`'s known identity (from Tier 1, or from the handshake if not yet known) | Drop announcement, do **not** write to Tier 2, apply peer penalty (see below) |
| Node receives `RecordReply` (user-requested fetch) | `sig` against the `ContentRecord`'s claimed origin — the scraper node_id is not in the record itself, so the *querying* node trusts the `sig` only insofar as it matches a public key it can resolve; if the holder is relaying a record it didn't originate, the original scraper's signature still must verify, otherwise treat as untrusted | Reject record, do not display as verified, apply peer penalty to the relaying holder |
| Archive node receives `ReplicatePush` | `sig` on the pushed `ContentRecord`, same as RecordReply | Reject push, respond `ReplicateAck { error }`, apply peer penalty |
| Node re-announces its own pinned content | N/A — re-signs locally each time, since it is signing its own claim | — |

Verification is **never** performed at announce-fan-out time for every
hop — only by the node directly receiving the `Announce`, `RecordReply`,
or `ReplicatePush`. This keeps Tier 2 writes O(1) per announcement
(one signature check) rather than requiring a fetch-and-validate round
trip before any pointer is accepted.

### Poison resistance — what is and isn't prevented

A signature on an `Announcement` proves *who* is claiming to hold a
record. It cannot prove the claim is true — a node can sign and announce
a `record_id` for content it never actually scraped. This is the residual
attack surface, and it is handled probabilistically rather than
preventively:

1. **Optimistic accept.** Tier 2 writes happen on a verified signature
   alone. This is deliberate — Tier 2 is a pointer index, not a content
   store, and requiring proof-of-content before accepting a pointer would
   mean fetching every announced record just to index it, defeating the
   purpose of having a lightweight Tier 2 at all.
2. **Verification on fetch.** The lie surfaces the moment any querying
   node actually fetches the record: either the `RecordReply` fails
   signature verification outright, or — if the holder fabricates a
   plausible-looking signed record under their own key — the `record_id`
   (Blake3 of canonical content fields) won't match what was announced,
   which is checked before display regardless of signature validity.
3. **Peer reputation penalty.** Any of the following cause a node to be
   marked and penalized in the requesting node's local peer reputation
   table (Phase 9 — Hardening):
   - `Announce` signature fails verification
   - `RecordReply` signature fails verification
   - `RecordReply` content hashes to a different `record_id` than was
     announced
   - `ReplicatePush` fails either check
   Repeated penalties escalate from de-prioritization in routing/fan-out
   to an explicit local ban (`dsearch peers ban`). Penalties decay
   linearly back to zero over 24h of clean behavior, so a transient
   blip doesn't become permanent exile. Bans are manual-only — they
   don't auto-expire, since banning is a deliberate operator decision,
   not an accumulated-score outcome. (`dsearch peers unban` reverses it.)
4. **`source_hash` collisions are not a trust issue.** Two honest nodes
   independently scraping the same URL is the expected case and is
   handled by the dedup rule in **Record lifecycle → Deduplication**
   (newer `created_at` wins), not by the trust model — both records can
   carry valid signatures from their respective honest scrapers.

This means Tier 2 poisoning degrades gracefully into "wasted fetch
attempts against bad pointers, followed by reputation-based pruning of
the offending peer" rather than either being structurally impossible or
silently corrupting search results indefinitely.

### Unsigned fields and forward compatibility

`holder_addr` in an `Announcement` is *not* covered by any other
mechanism for binding it to a transport-layer identity beyond the
signature itself — a node can only announce on behalf of an
address it controls because the QUIC connection used to send the
`Announce` is already authenticated via the cert-based node ID
(**TLS / certificate strategy**). A receiving node should treat an
`Announce` arriving over a connection whose authenticated node ID
doesn't match the signer of `sig` as an automatic verification failure,
not just a mismatched-but-tolerated edge case.

Unknown future fields added to either struct are excluded from the
signature unless explicitly added to the canonical encoding in a
version bump — consistent with the wire protocol's "unknown fields
ignored for forward compat" rule, signatures only ever cover fields
the signing node's protocol version knows about.

---

## Wire protocol

All peer communication uses a simple versioned frame over QUIC streams:

```
┌──────────┬──────────┬──────────┬─────────────────────┐
│ version  │ msg_type │  length  │       payload        │
│  u8 (1)  │  u8 (1)  │  u32 (4) │    JSON (UTF-8)      │
└──────────┴──────────┴──────────┴─────────────────────┘
```

- `version` — protocol version; nodes reject connections where versions are
  incompatible (see Protocol versioning below)
- `msg_type` — enum defined in `src/proto/msg_type.rs`
- `length` — byte length of payload; max 1 MB (matches record cap)
- `payload` — UTF-8 JSON; unknown fields ignored for forward compat

### Message types
```
0x01  Handshake          { version, node_id, roles, capabilities }
0x02  HandshakeAck       { version, node_id, roles, capabilities }
0x03  Ping               { nonce }
0x04  Pong               { nonce }
0x05  FindNode           { target_id }
0x06  FindNodeReply      { nodes: [{ id, addr }] }
0x07  Announce           { record_id, source_hash, schema, tags,
                           holder_addr, expires_at, sig }
0x08  AnnounceAck        { record_id }
0x09  SearchQuery        { query, schema?, limit, ttl, query_id }
0x0A  SearchReply        { query_id, results: [{ id, title, schema,
                           tags, holder_addr, score }] }
0x0B  RecordFetch        { record_id }
0x0C  RecordReply        { record: ContentRecord } | { error }
0x0D  ReplicatePush      { record: ContentRecord }
0x0E  ReplicateAck       { record_id } | { error }
0x0F  PeerExchange       { peers: [{ id, addr, roles }] }
0xFF  Goodbye            { reason }
```

Unknown `msg_type` values are silently ignored — never crash on unknown messages.

### Protocol versioning
- Current protocol version: `1`
- Handshake always sent first on every new connection
- If `abs(local_version - remote_version) > 1` → send `Goodbye`, close
- Minor changes (new optional fields) → same version, unknown fields ignored
- Breaking changes → increment version; maintain one version of back-compat
- Minimum supported version baked into binary, configurable via
  `config.toml` (`node.min_protocol_version`)

### TLS / certificate strategy
QUIC requires TLS 1.3. Rather than a CA chain (impractical for dynamic P2P):
- Each node derives a self-signed X.509 cert from its Ed25519 keypair at
  first run; stored in `{data_dir}/node.crt`
- The cert's Subject Alternative Name contains the node's Blake3 node ID
- Quinn is configured with a custom `ServerCertVerifier` that:
  1. Extracts the node ID from the SAN
  2. Verifies the cert is self-signed and the public key matches the node ID
  3. Accepts — no CA chain check
- Peers are trusted by node ID, not by CA. A node ID seen in Tier 1 is
  the trust anchor for that connection.
- Cert is rotated only when the identity keypair is regenerated
- This is the same keypair used for `ContentRecord` and `Announcement`
  signatures (see **Trust model**), so an authenticated QUIC connection
  and a verified `sig` are checking the same underlying identity from
  two different layers — a receiving node should treat a mismatch
  between the two as an automatic verification failure

---

## Bootstrap peer discovery

### How a new node joins the network

```
1. Read {data_dir}/bootstrap.toml          ← user-editable, highest priority
2. Try DNS SRV: _dsearch._udp.dsearch.network
3. Fall back to compiled-in default list   ← updated each release
4. Connect to any reachable bootstrap peer
5. Send FindNode(own_id) → get initial k-bucket entries
6. Mark as joined; begin normal DHT operation
```

### bootstrap.toml — user-editable, lives in data dir

```toml
# {data_dir}/bootstrap.toml
# Edit freely. Add community or private bootstrap nodes here.
# Format: each entry needs an id (node public key as hex) and addr.
# The built-in list is always tried alongside this file.
# Remove the built-in list entirely by setting use_defaults = false.

use_defaults = true   # set false to run a fully private network

[[peers]]
id   = "a1b2c3..."    # Ed25519 public key hex (64 chars)
addr = "bootstrap1.dsearch.network:7744"
note = "official bootstrap 1"

[[peers]]
id   = "d4e5f6..."
addr = "203.0.113.10:7744"
note = "community node run by @alice"
```

### CLI commands
```
dsearch bootstrap list              # show active bootstrap peers + source
dsearch bootstrap add --id HEX --addr HOST:PORT --note "..."
dsearch bootstrap remove --id HEX
dsearch bootstrap test              # probe each peer, show latency + reachable
dsearch bootstrap reset             # restore defaults only, clear custom entries
```

### UI
Settings panel → "Bootstrap peers" section:
- Table of all peers (source column: "built-in" / "user" / "dns")
- Add / Remove buttons
- "Test all" button — shows green/red reachability per peer
- "Use defaults" toggle — when off, shows warning: "private network mode"

### Built-in default list (compiled in, `src/bootstrap/defaults.rs`)
```rust
pub const DEFAULT_BOOTSTRAP_PEERS: &[BootstrapPeer] = &[
    BootstrapPeer { id: "...", addr: "bootstrap1.dsearch.network:7744" },
    BootstrapPeer { id: "...", addr: "bootstrap2.dsearch.network:7744" },
    BootstrapPeer { id: "...", addr: "bootstrap3.dsearch.network:7744" },
];
```
Updated each release. Users who have a custom `bootstrap.toml` are never
affected — their file takes precedence.

---

## First-run / onboarding flow

On first launch (no `{data_dir}/identity.key` found):

```
Step 1 — Welcome screen
  "dsearch is a decentralised search network."
  "Your data will be stored at: {data_dir}"  ← clickable, can change
  [Change location]  [Continue]

Step 2 — Generate identity
  "Generating your node identity..."
  Ed25519 keypair written to {data_dir}/identity.key
  TLS cert derived and written to {data_dir}/node.crt
  "Done. Your node ID: a1b2c3..."  ← copyable
  [Continue]

Step 3 — Choose role
  AutoNAT probe runs in background while user reads descriptions.
  Radio group:
    ◉ Light    — best for laptops and home connections (recommended)
    ○ Full     — contribute to search routing (needs open port)
    ○ Scraper  — index web content for the network
    ○ Archive  — store content long-term for resilience
    ○ Custom   — choose multiple roles manually
  AutoNAT result shown inline when ready:
    ✓ "Your node appears publicly reachable — Full node is possible"
    ✗ "Your node is behind NAT — Light node recommended"
  [Continue]

Step 4 — Connect to network
  "Connecting to bootstrap peers..."
  Progress: peer count ticking up
  "Connected to 4 peers. Network ready."  ← success path
  If bootstrap.toml, DNS SRV, and all compiled-in defaults fail:
    "Couldn't reach the network automatically."
    [Retry]  [Add a peer manually]  ← opens single-field multiaddr input,
                                       wraps `dsearch peers add`, then retries
  [Start searching]

Step 5 — Empty search screen with helpful state
  "Your node is connected. Start searching, or add a scraper
   in Settings to contribute content to the network."
```

All steps completable via CLI:
```
dsearch init                        # runs steps 1-4 non-interactively
dsearch init --role full --data-dir /mnt/data/dsearch
```

---

## Search query language

Consistent behaviour for both UI and agent API.

### Basic
```
rust async           → AND match across title, body, tags
"rust async"          → exact phrase match
rust OR async         → OR
rust -async           → rust AND NOT async
```

### Field-scoped
```
title:rust                    → match in title field only
tag:category:networking       → exact tag match
schema:rust/crate             → filter by schema (same as ?schema= param)
source:crates.io              → filter by source domain
scraped:keyword                → filter by scrape_source (url|feed|api|keyword)
refresh:interval               → filter by refresh_policy (once|interval|on-change)
```

### Operators
```
rust async limit:20           → result count hint (overrides ?limit=)
rust async since:2024-01-01   → filter by created_at
rust async before:2025-01-01
```

### Ranking
Results scored by:
1. Field match weight: title match > tag match > body match
2. Exact phrase bonus
3. Record freshness (`created_at` recency)
4. Holder count (more holders = more stable = slight boost)

Ranking is deterministic given the same result set — agents get consistent
ordering across calls.

---

## Config file

Lives at `{data_dir}/config.toml`. Created with defaults on first run.
All keys are optional — missing keys use the compiled-in default.
Unknown keys are warned about but not fatal (forward compat).

```toml
[node]
role                  = "light"          # light|full|bootstrap|relay|stun|scraper|archive|gateway|observer
max_connections       = 200
min_protocol_version  = 1
ipv4                  = true
ipv6                  = true

[api]
port                  = 7743             # local HTTP API port
# If 7743 is taken, node logs a warning and tries 7744, 7745... up to 7753

[gateway]
enabled               = false
bind                  = "0.0.0.0:7744"
rate_limit_per_min    = 60               # per API key (or per-IP if no key set)
require_api_key       = false            # true = reject requests with no key

[storage]
quota_mb              = 0                # 0 = unlimited
quota_action          = "evict_oldest"   # evict_oldest|pause_scraper|warn_only
tier2_max_mb          = 512              # Tier 2 announcement index cap

[relay]
bandwidth_limit_mbps  = 100

[scraper]
default_interval_secs = 3600
default_replicate     = 0               # replication factor
default_lifecycle     = "ephemeral"     # ephemeral|pinned, used when a job omits it
# Individual jobs are NOT listed here — see [[scraper.jobs]] array,
# defined in full under Scrape sources → ScrapeJob config object
# Keyword-discovery provider settings (UA rotation, rate caps, etc.)
# are NOT here either — see {data_dir}/search_providers.toml

[log]
level                 = "info"          # error|warn|info|debug|trace
output                = "stderr"        # stderr|file|both
file                  = "{data_dir}/dsearch.log"
max_size_mb           = 50
rotate_count          = 3

[bootstrap]
# Override in bootstrap.toml — this file is for node behaviour only
use_defaults          = true
```

### Config migration
- `meta` table in redb stores `config_version: u32`
- On startup: if `config_version < current` → run migration functions in order
- Migrations are append-only in `src/config/migrations.rs`
- If store is from a *future* version → log error, refuse to open (prevents
  silent data corruption on downgrade)
- **Not the same number as the wire protocol version** (Wire protocol →
  Protocol versioning) — `config_version` tracks `config.toml`'s own
  schema, independent of peer-to-peer compatibility. The two evolve on
  separate timelines and are never compared to each other.

---

## Logging

Uses `tracing` crate (one additional dep). Structured, async-safe.

```toml
[dependencies]
tracing            = "0.1"
tracing-subscriber = "0.3"
```

Log levels: `error` / `warn` / `info` / `debug` / `trace`
Default: `info` for release builds, `debug` for dev builds.

Operators running headless nodes get file logging with rotation.
The Observer role subscribes to the node's internal `tracing` spans and
can stream them to a remote collector via the local API:
`GET /log/stream` — chunked, one JSON log line per chunk (debug builds only).

---

## `dsearch doctor`

Runs a full health check and prints a human-readable report.
Also available as `dsearch doctor --output json` for scripting.

```
dsearch doctor

  Identity
    ✓ Keypair found at {data_dir}/identity.key
    ✓ TLS cert valid, matches keypair
    ✓ Node ID: a1b2c3...

  Storage
    ✓ store.redb opens cleanly
    ✓ Schema version: 3 (current)
    ✓ Tier 3: 42,100 records (1.2 GB)
    ✓ Tier 2: 180,000 entries (87 MB / 512 MB cap)
    ✓ Quota: unlimited

  Network
    ✓ UDP port 7744 bindable
    ✓ Bootstrap peer bootstrap1.dsearch.network:7744 reachable (42 ms)
    ✓ Bootstrap peer bootstrap2.dsearch.network:7744 reachable (91 ms)
    ✗ Bootstrap peer bootstrap3.dsearch.network:7744 unreachable (timeout)
    ✓ AutoNAT: publicly reachable

  API
    ✓ Local API on port 7743

  Service
    ✓ Registered for startup (systemd user unit)
    ✓ Currently running

  Config
    ✓ config.toml valid
    ⚠ Unknown key: node.experimental_flag (ignored)
```

---

## Node roles

| Role | Tier 1 | Tier 2 | Tier 3 | Public port |
|---|---|---|---|---|
| **Light** | ✓ | small | own content | No |
| **Full** | ✓ | ✓ full | own content | Yes |
| **Bootstrap** | ✓ | — | — | Yes (stable) |
| **Relay** | ✓ | — | — | Yes |
| **STUN** | — | — | — | Yes |
| **Scraper** | ✓ | announces | scraped | No |
| **Archive** | ✓ | ✓ | own + replicated | Yes |
| **Indexer** | ✓ | ✓ full | own content | Yes |
| **Gateway** | ✓ | ✓ | own content | Yes (HTTP) |
| **Observer** | ✓ | — | — | No |

### AutoNAT detection
On startup: ask connected peers "can you dial me back?"
- Reachable → suggest Full (user accepts or declines)
- Not reachable → Light (user can override)
- Result shown in UI status bar and `dsearch role autodetect`

---

## Cross-platform targets

| Platform | Install | Data dir |
|---|---|---|
| macOS | `.dmg` | `~/Library/Application Support/dsearch/` |
| Linux | `.deb` / `.rpm` / AppImage | `~/.local/share/dsearch/` |
| Windows | NSIS `.exe` | `%APPDATA%\dsearch\` |

Resolved via `dirs-next = "2"`. Data dir always shown as clickable path in UI.

---

## Run modes

Three distinct ways to run the same node binary — not Linux-specific,
though headless is the most common case there:

| Mode | What's running | Typical use |
|---|---|---|
| **Full UI** | egui window, full Settings | Desktop, interactive use |
| **Tray** | No main window; background process + system tray icon | Desktop, "always on" without a window in the way |
| **Headless service** | No UI at all, registered with the OS to survive reboot | Servers, NAS boxes, always-on Linux boxes especially |

All three are the same binary and the same local API underneath — the
only difference is what's attached to the front of it (see **UI ↔ CLI
parity**).

### Tray icon

Status dot (color = connected / connecting / offline), right-click menu:
- **Open dsearch** — opens the full UI window
- **Pause node** / **Resume node** — stops/restarts DHT participation
  and scraper jobs without fully exiting; status dot reflects paused state
- **Quit** — clean shutdown (same `Goodbye`-then-close path as
  `dsearch node stop`)

Kept intentionally minimal — no per-job controls or search from the
tray; anything beyond status/pause/quit opens the full UI instead of
growing the tray menu. Built with `tray-icon` (egui has no native tray
support); same process as `dsearch tray start`, just with a window
hidden by default instead of shown.

### Run at startup

`dsearch service install` registers the binary with the OS's native
mechanism so "survives reboot" is a real, verifiable state rather than
just "a unit file exists somewhere in `installers/`":

| Platform | Mechanism | Registered by |
|---|---|---|
| Linux | systemd user or system unit | `dsearch service install` writes + `systemctl enable` |
| macOS | launchd plist | `dsearch service install` writes + `launchctl load` |
| Windows | Windows Service | `dsearch service install` registers via Service Control Manager |

`--headless` on install means no tray icon and no UI process at all —
just the node. Without it, `service install` still runs headless at the
OS level (services don't have a desktop session) but the tray icon
launches separately at user login via each platform's standard
autostart mechanism (XDG autostart / launchd login item / Windows
Startup folder) — so a desktop user gets both "survives reboot" and
"tray icon when I log in," while a headless server only gets the former.

`dsearch service status` reports both registration state and current
running state separately, since a service can be registered-but-stopped
or running-but-not-registered-for-boot — those are different problems
and `doctor` should distinguish them.

---

## UI ↔ CLI parity

This list is the single source of truth for the command surface — every
command below is a thin wrapper over the local HTTP API
(`http://127.0.0.1:{port}`, Phase 7), and **the desktop UI and tray icon
call that same local API, not the node internals directly.** CLI, full
UI, and tray are three renderers of one API; parity isn't a discipline
to maintain by hand, it's structurally guaranteed by all three going
through the same surface. Earlier sections (First-run flow, Bootstrap
peer discovery) describe *why* a command exists; this is the flat
reference for *what* exists — if the two ever disagree, this list wins.

```
# Node lifecycle
dsearch node start
dsearch node start --headless
dsearch node start --role full,scraper
dsearch node stop / status / restart
dsearch init                              # first-run setup, non-interactive
dsearch init --role full --data-dir PATH

# Run at startup (registers with the OS, doesn't just leave a unit file unused)
dsearch service install [--headless]      # systemd unit / launchd plist / Windows Service
dsearch service enable / disable          # start-on-boot toggle
dsearch service status                    # is it registered, is it running
dsearch service uninstall

# Tray (desktop platforms only; no-op / unavailable on headless)
dsearch tray start                        # launches background process + tray icon
dsearch tray stop

# Role management
dsearch role list / set / add / remove / autodetect

# Bootstrap
dsearch bootstrap list
dsearch bootstrap add --id HEX --addr HOST:PORT --note "..."
dsearch bootstrap remove --id HEX
dsearch bootstrap test
dsearch bootstrap reset

# Search
dsearch search "query"
dsearch search "query" --schema rust/crate --limit 20 --output json

# Records
dsearch record get {id} [--output json]
dsearch record list --schema wiki/article --limit 50
dsearch record pin / unpin / delete / announce {id}

# Replication
dsearch replicate push {id} --factor 3
dsearch replicate status {id}

# Schemas
dsearch schema list / show {id}

# Peers
dsearch peers list [--output json]
dsearch peers add {multiaddr}
dsearch peers ban / unban {peer_id}

# Gateway
dsearch gateway key create / list / revoke {nickname}

# Scrapers
dsearch scraper job add --name X --source url|feed|api|keyword \
  --target "..." --refresh once|interval|on-change \
  --interval-secs N --lifecycle ephemeral|pinned --ttl-secs N
dsearch scraper job list / remove / pause / resume / run / status {name}

# Search providers (keyword-discovery backends)
dsearch search-provider list
dsearch search-provider add --name X --kind html_scrape|api --target "..." \
  --priority N [--api-key KEY]
dsearch search-provider enable / disable {name}
dsearch search-provider set-priority {name} N
dsearch search-provider test {name}
dsearch search-provider remove {name}

# Storage
dsearch storage info / vacuum / export {path} / import {path}

# Config
dsearch config show / set KEY VALUE / reset

# Identity
dsearch identity show / export {path} / import {path}

# Diagnostics
dsearch doctor [--output json]
dsearch log tail [--level debug]          # stream log output
```

All commands that produce output support `--output json` for scripting.
All commands return exit code 0 on success, non-zero on error.
If port 7743 is in use, node auto-increments to 7744–7753 and logs the
actual port; CLI reads it from a lock file at `{data_dir}/api.port`.

---

## Agent / Gateway HTTP API

### Local API — `http://127.0.0.1:{port}`
```
GET  /health
GET  /node
GET  /search?q=&schema=&limit=&offset=
GET  /record/{id}
GET  /records?schema=&limit=&offset=
GET  /schema / /schema/{id}
GET  /peers
POST /peers/add
GET  /scraper
POST /scraper/run
GET  /storage
POST /storage/vacuum
GET  /log/stream                          # chunked log stream (debug builds)
GET  /openapi.json
```

### Gateway API — optional, public read-only
`0.0.0.0:7744` (configurable). No POST routes. Rate-limited per key
(falls back to per-IP if no key is presented and `require_api_key` is
false).

**API keys:**
- `dsearch gateway key create` → generates a 256-bit random secret
  (shown once) plus a friendly auto-generated nickname like
  `swift-falcon-7x2` (adjective-animal-suffix, for telling keys apart
  at a glance in logs and `key list` — the nickname is cosmetic, the
  secret is what's checked)
- `dsearch gateway key list` → nickname, created date, last used,
  request count — never the secret itself
- `dsearch gateway key revoke {nickname}` → immediate, no grace period
- Rate limit and request counts are tracked per key, not globally, so
  one busy client can't starve everyone else sharing the gateway
- Keys stored hashed (Blake3) in the `meta` table; the raw secret is
  never persisted or logged after creation

### Response conventions
- `application/json; charset=utf-8`
- Errors: `{ "error": "...", "code": 404 }`
- Search: chunked transfer, one JSON object per chunk
- Headers: `X-Node-Id`, `X-Schema`, `X-Record-Count`, `X-Holder-Count`
- `X-Protocol-Version` on all responses

---

## Phase 1 — Core networking + identity
- QUIC transport (`quinn 0.11.9`) with self-signed cert verifier
- Ed25519 keypair + TLS cert generation on first run
- Versioned wire protocol (`src/proto/`)
- Kademlia DHT — Tier 1 routing table
- STUN role (~50 LOC, stateless)
- Relay role — forward encrypted QUIC streams
- Bootstrap role — stable DHT entry point
- AutoNAT probe
- Bootstrap peer resolution: `bootstrap.toml` → DNS → compiled defaults
- Protocol version handshake on every connection
- Graceful shutdown: drain in-flight streams, send `Goodbye`, close

**Exit test:** start two local node instances with separate data dirs,
point one's `bootstrap.toml` at the other, confirm via `dsearch peers
list` on each that they see each other in Tier 1, then `dsearch node
stop` one and confirm the other logs a clean disconnect (not a timeout
or panic). This is the phase that catches a broken cert verifier or
wire-format mismatch — do not proceed to Phase 2 until two real
processes can complete a handshake.

---

## Phase 2 — Data model + config
- `ContentRecord`: `id`, `source_url`, `source_hash`, `schema`, `tags`,
  `body`, `created_at`, `expires_at`, `scrape_source`, `refresh_policy`,
  `sig`
- `Announcement`: `record_id`, `source_hash`, `schema`, `tags`,
  `holder_addr`, `expires_at`, `sig`
- `ScrapeJob` config object: source/refresh/lifecycle axes (see
  **Scrape sources**), `[[scraper.jobs]]` array in `config.toml`
- Signature scheme for both structs: signer, canonical encoding, and
  verification points (see **Trust model**)
- Blake3 content addressing
- Plain UTF-8 JSON, `serde_json` with `#[serde(default)]` on every field
  added after v1 (NOT `deny_unknown_fields` — that would make
  `ContentRecord`/`Announcement` reject any newer-version peer's extra
  fields outright, contradicting the wire protocol's own forward-compat
  rule one layer down). Truly unrecognized top-level keys are ignored,
  not rejected.
- Schemas: `wiki/article`, `rust/crate`, `link/media`, `generic/kv`
- Hard cap: 1 MB record, 256 B announcement entry
- `config.toml` with full key set and defaults
- Config migration framework (`config_version` in meta table)

**Exit test:** `dsearch config show` round-trips every key in the
**Config file** section back out correctly on a fresh data dir; hand-edit
`config.toml` to bump `config_version` past current, confirm `dsearch
node start` refuses to open with a clear error rather than silently
corrupting state. Construct one `ContentRecord` and one `Announcement`
by hand (a small test fixture is fine here), sign and verify both
round-trip through the canonical encoding without altering the bytes.

---

## Phase 3 — Local storage
- `redb 2.6.0` — `{data_dir}/store.redb`
- Tables: `records`, `source_index`, `announcements`, `routing`,
  `pins`, `peers`, `banned_peers`, `meta`
- Inverted index on `(schema, tag_key, tag_value)`
- `source_index`: `source_hash → record_id` for dedup
- Tier 2 TTL enforcement
- Storage quota + eviction policy
- Async expiry sweeper
- Schema version check + migration runner on open

**Exit test:** `dsearch record list` against a freshly-seeded store
returns what was inserted; insert two records with the same
`source_hash` and different `created_at`, confirm only the newer
survives in `source_index`; manually set one record's `expires_at` to
the past, confirm the next sweep cycle removes it from `dsearch record
list` without restarting the node.

---

## Phase 4 — Search
- Local Tier 3 search (sub-ms)
- Query language parser (`src/search/query.rs`)
- Tier 2 pointer lookup → on-demand fetch
- DHT fan-out K=20, streaming results
- Light nodes: query connected full nodes only
- Dedup by record ID, ranked by field weight + freshness + holder count

**Exit test:** on the two-node setup from Phase 1's exit test, scrape
one record on node A (Phase 5 not built yet — insert it directly via
the storage layer for this test), confirm `dsearch search "..."` on
node B finds it via DHT fan-out, not just locally. Run every example
from **Search query language** (phrase match, `-exclude`, `title:`,
`since:`/`before:`) against a small fixture set and confirm each
returns the expected subset.

---

## Phase 5 — Scraper + announcement + dedup
- Source hash dedup before write
- Announcement includes `source_hash` for cross-node dedup
- Periodic re-announcement for pinned content
- Optional Archive replication push
- `ScrapeJob` runner: `url`/`feed`/`api`/`keyword` sources ×
  `once`/`interval`/`on-change` refresh × `ephemeral`/`pinned` lifecycle
  (see **Scrape sources**)
- Pluggable search providers for keyword discovery: `search_providers.toml`,
  priority-ordered fallback, per-provider UA rotation/cookie jar/backoff
  for `html_scrape` kind, DDG shipped as the default provider
- Transforms: `crates_io_v1`, `wikimedia_dump`, `html_strip`, `weather_v1`
- Hand-written HTTP/1.1 GET over `tokio::net::TcpStream` for plain `url`/
  `feed`/`api` sources. Keyword-provider requests need real HTTPS,
  cookie-jar persistence, and redirect handling that a hand-rolled
  client makes painful — these use a real HTTP client (`reqwest`,
  Phase 5 dependency addition) scoped to `scraper/discovery/` only, not
  a network-wide swap of the hand-written client

**Exit test:** `dsearch scraper job add` a `url`-source job against a
real public page, `dsearch scraper job run` it, confirm the record
shows up in `dsearch record list` and gets announced (visible to a
second local node via `dsearch search`). Run the same `url` job twice
in a row and confirm the second run dedups against `source_hash`
instead of creating a duplicate. Add one `keyword` job, confirm it
resolves through the default provider and produces multiple `url`-style
records, capped at `max_results`.

---

## Phase 6 — Sanitization
- Single `sanitize()` — all ingest paths
- Valid UTF-8, no 0x00–0x1F except 0x0A, no Unicode Cf/Cc
- Caps: 1 MB record, 256 B key, 64 KB value
- Unknown schema → raw k/v render only
- Malformed → silent drop + peer penalty

**Exit test:** feed `sanitize()` a deliberately malformed record
(invalid UTF-8, a control byte outside the allowed set, an oversized
field) for each rejection rule above and confirm each is dropped with
the correct reason logged, not silently accepted or panicking.

---

## Phase 7 — Agent API + CLI
- Hand-written async HTTP/1.1, `tokio::net::TcpListener`
- Port conflict handling: auto-increment 7743→7753, write actual port to
  `{data_dir}/api.port`; CLI reads this file
- All CLI subcommands proxy to local HTTP API
- `--output json` on all list/get commands
- Non-zero exit codes on error
- Gateway API as optional public surface, with per-key rate limiting
  and key create/list/revoke
- OpenAPI 3.1 at `/openapi.json`

**Exit test:** start two node instances on the same machine
simultaneously, confirm the second logs the port auto-increment and
`dsearch` (second instance) reads the right port from `api.port`
rather than guessing 7743. Hit every route in **Agent / Gateway HTTP
API** with a raw HTTP client (not the CLI) and confirm each matches its
documented response shape, including the `--output json` parity check:
CLI output and raw API JSON output for the same query should carry the
same data.

---

## Phase 8 — First-run + UI (egui 0.34)
- First-run wizard: data dir → identity gen → role picker → bootstrap connect
- `fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame)` entry point
- `skrifa` font backend, `ScrollArea` fade-out, virtual scroll (`egui_extras`)
- Bootstrap peers panel with source labels + test button
- Role picker with AutoNAT result inline
- Meaningful empty states (no results yet, connecting, etc.)
- Status bar: role, peers, records, Tier 2 size, bandwidth
- Settings: all config keys, data dir (clickable), identity, gateway
  (incl. key create/list/revoke with nickname shown), scrapers,
  bootstrap peers (incl. manual peer-add fallback), search providers
  panel (priority order, enable/disable, test button)
- Tray icon (`tray-icon` crate): status dot, open UI, pause/resume
  node, quit — see **Run modes**

**Exit test:** delete `identity.key` to simulate a true first run,
launch the full UI, walk all 5 onboarding steps to completion, confirm
the resulting `config.toml`/`identity.key`/`node.crt` match what
`dsearch init` would have produced non-interactively from the same
inputs — UI and CLI onboarding must converge on identical on-disk
state. Click every Settings panel once and confirm each reflects live
`config show` values, not stale defaults.

---

## Phase 9 — Hardening + systemd/service
- Connection pool cap (default 200 QUIC)
- Bounded Tokio channels for backpressure
- DHT dead-peer pruning, Tier 2 TTL at scale
- Peer reputation — penalize flood/malformed/slow, and the four
  signature/hash failure cases defined in **Trust model**; penalties
  decay over 24h, bans are manual-only
- Memory: <50 MB idle, <200 MB loaded
- Sybil resistance: PoW node ID
- Scraper subprocess isolation (seccomp / job object / sandbox)
- IPv4 + IPv6 dual-stack (both bound by default)
- Relay bandwidth accounting persisted across restarts
- `dsearch doctor` command
- systemd unit file (Linux), launchd plist (macOS), Windows Service
  wrapper, plus `dsearch service install/enable/disable/status/uninstall`
  driving actual OS registration — see **Run modes**
- Full integration + property-based test suite
- **Scale mitigations (thousands of nodes):**
  - Search result caching at the querying node, short TTL, so repeat
    queries don't re-trigger full K=20 fan-out
  - Per-node concurrent-query cap (`node.max_concurrent_queries`) so a
    handful of busy nodes can't be driven to saturation by query volume
    landing near their `record_id` space
  - Jittered re-announcement: pinned-record re-announce at `TTL/2` is
    spread with random jitter per record, not fired as one batch on
    rejoin, to avoid an announcement storm hitting the same K-closest
    peers simultaneously
  - Tier 2 write-rate limiting per peer (separate from the existing
    size cap) — caps announcement *churn*, since high-frequency
    `interval` scrape jobs across many nodes stress write throughput
    before they stress storage size
  - Gateway/Indexer nodes are optional conveniences, never a required
    hop for search or discovery — keyword-discovery scrapers run
    locally per-node against DDG, not through a shared gateway, so
    they don't become an unintended centralization point

**Exit test:** `dsearch service install` then `enable` on the target
OS, reboot (or simulate via the OS's own service-manager restart),
confirm `dsearch service status` reports both registered and running
without manual intervention. Run `dsearch doctor` and confirm every
check in **`dsearch doctor`**'s sample output reflects a real
underlying check, not a hardcoded pass. Idle-memory-check the running
process against the <50 MB target before calling Phase 9 done — this
is the last phase, so it's also the final gate on the resource-friendly
constraint from **Build philosophy**.

---

## Cargo.toml

```toml
[dependencies]
quinn              = "0.11.9"
tokio              = { version = "1.52.3", features = ["full"] }
redb               = "2.6.0"
blake3             = "1.8.5"
ed25519-dalek      = "2.2.0"
serde              = { version = "1", features = ["derive"] }
serde_json         = "1"
eframe             = "0.34.2"
tray-icon          = "0.19"
toml               = "0.8"
dirs-next          = "2"
clap               = { version = "4", features = ["derive"] }
tracing            = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
reqwest            = { version = "0.12", features = ["cookies"] } # scraper/discovery/ only — see Phase 5
```

---

## Project structure

```
dsearch/
├── src/
│   ├── main.rs                    # routes: UI / headless / CLI
│   ├── proto/
│   │   ├── msg_type.rs            # message enum
│   │   ├── frame.rs               # versioned framing
│   │   └── cert.rs                # self-signed TLS cert from Ed25519
│   ├── trust/
│   │   ├── sign.rs                # canonical encoding + signing
│   │   └── verify.rs              # signature + record_id verification,
│   │                               # peer penalty hooks
│   ├── bootstrap/
│   │   ├── defaults.rs            # compiled-in peer list
│   │   └── resolver.rs            # file → DNS → defaults priority chain
│   ├── cli/                       # clap subcommands (proxy to HTTP API)
│   ├── node/
│   │   ├── roles.rs
│   │   ├── dht.rs                 # Tier 1
│   │   ├── announce.rs            # Tier 2 protocol
│   │   ├── autonat.rs             # reachability probe
│   │   ├── relay.rs
│   │   └── stun.rs
│   ├── storage/
│   │   ├── records.rs             # Tier 3
│   │   ├── index.rs               # inverted + source index
│   │   ├── expiry.rs              # Tier 2 + Tier 3 sweep
│   │   ├── quota.rs               # enforcement + eviction
│   │   └── migrations.rs          # versioned schema migrations
│   ├── search/
│   │   └── query.rs               # query language parser
│   ├── scraper/
│   │   ├── job.rs                 # ScrapeJob runner: source × refresh × lifecycle
│   │   ├── discovery/
│   │   │   ├── providers.rs       # search_providers.toml parsing + priority/fallback
│   │   │   ├── html_scrape.rs     # generic results-page scraper (UA rotation, cookie jar, backoff)
│   │   │   └── api.rs             # generic JSON search-API client (SearXNG, Brave, etc.)
│   │   └── replicate.rs
│   ├── config/
│   │   └── migrations.rs          # config version migrations
│   ├── sanitize.rs
│   ├── api/
│   │   ├── local.rs                # local HTTP API (Phase 7 routes)
│   │   ├── gateway.rs              # public read-only gateway server
│   │   └── gateway_keys.rs         # key create/list/revoke, Blake3-hashed storage
│   ├── service/
│   │   ├── install.rs             # systemd unit / launchd plist / Windows Service
│   │   └── status.rs              # registered-state vs running-state check
│   └── ui/                        # egui panels
│       ├── onboarding.rs          # first-run wizard
│       ├── search.rs
│       ├── settings.rs
│       ├── bootstrap_panel.rs
│       └── tray.rs                # tray-icon menu: status, open, pause/resume, quit
├── assets/
│   └── icon.png
├── installers/
│   ├── windows/                   # NSIS script
│   ├── linux/                     # .desktop, systemd unit
│   └── macos/                     # entitlements, launchd plist
├── .github/workflows/
│   └── release.yml                # matrix: ubuntu / macos / windows
└── Cargo.toml
```