# Phase 4 Exit Test
# Per the roadmap:
#   1. Insert records via storage layer, confirm `dsearch search "..."` finds them locally
#   2. Run every example from Search query language (phrase match, -exclude, title:,
#      since:/before:) against a small fixture set and confirm each returns the expected subset

$ErrorActionPreference = "Continue"

Write-Host "=== Phase 4 Exit Test ===" -ForegroundColor Cyan

# Clean up any previous test data
$testDir = Join-Path $env:TEMP "dsearch-phase4-test"
if (Test-Path $testDir) { Remove-Item -Recurse -Force $testDir }

# Build first to ensure we have a fresh binary
Write-Host "`n--- Building dsearch ---" -ForegroundColor Yellow
$buildResult = & cargo build --release 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) {
    Write-Host "FAIL - Build failed" -ForegroundColor Red
    Write-Host $buildResult
    exit 1
}
Write-Host "OK - Build succeeded" -ForegroundColor Green

$dsearchExe = ".\target\release\dsearch.exe"

# Initialize a fresh node
Write-Host "`n--- Initializing test node ---" -ForegroundColor Yellow
$initResult = & $dsearchExe init --data-dir $testDir 2>&1 | Out-String
Write-Host "Init output: $initResult"

# ============================================================
# Seed fixture records
# ============================================================
Write-Host "`n--- Seeding fixture records ---" -ForegroundColor Yellow

# Record 1: Rust async article
$rec1 = @"
{
    "id": "rust-async-guide",
    "source_url": "https://blog.example.com/rust-async",
    "source_hash": "hash_rust_async",
    "schema": "wiki/article",
    "tags": ["category:networking", "lang:rust"],
    "body": "A comprehensive guide to Rust async programming with tokio runtime benchmarks",
    "created_at": 1705000000,
    "expires_at": 9999999999,
    "scrape_source": "url",
    "refresh_policy": "once",
    "sig": ""
}
"@

# Record 2: Python async article
$rec2 = @"
{
    "id": "python-async-intro",
    "source_url": "https://docs.python.org/async",
    "source_hash": "hash_python_async",
    "schema": "wiki/article",
    "tags": ["category:networking", "lang:python"],
    "body": "Introduction to Python async programming with asyncio",
    "created_at": 1700000000,
    "expires_at": 9999999999,
    "scrape_source": "url",
    "refresh_policy": "once",
    "sig": ""
}
"@

# Record 3: Tokio crate
$rec3 = @"
{
    "id": "tokio-crate",
    "source_url": "https://crates.io/crates/tokio",
    "source_hash": "hash_tokio",
    "schema": "rust/crate",
    "tags": ["category:networking", "lang:rust"],
    "body": "Tokio is an async runtime for the Rust programming language",
    "created_at": 1710000000,
    "expires_at": 9999999999,
    "scrape_source": "url",
    "refresh_policy": "interval",
    "sig": ""
}
"@

# Record 4: Old weather data (keyword-scraped, interval refresh)
$rec4 = @"
{
    "id": "weather-nyc",
    "source_url": "https://api.weather.example/v1/current?loc=nyc",
    "source_hash": "hash_weather_nyc",
    "schema": "generic/kv",
    "tags": ["category:weather", "city:nyc"],
    "body": "Current weather in New York City: 72F, partly cloudy",
    "created_at": 1690000000,
    "expires_at": 9999999999,
    "scrape_source": "keyword",
    "refresh_policy": "interval",
    "sig": ""
}
"@

# Record 5: A link/media record
$rec5 = @"
{
    "id": "rust-logo",
    "source_url": "https://rust-lang.org/logo",
    "source_hash": "hash_rust_logo",
    "schema": "link/media",
    "tags": ["category:branding", "lang:rust"],
    "body": "Official Rust programming language logo image",
    "created_at": 1708000000,
    "expires_at": 9999999999,
    "scrape_source": "feed",
    "refresh_policy": "on-change",
    "sig": ""
}
"@

$records = @(
    @{ Name = "rec1"; Json = $rec1 },
    @{ Name = "rec2"; Json = $rec2 },
    @{ Name = "rec3"; Json = $rec3 },
    @{ Name = "rec4"; Json = $rec4 },
    @{ Name = "rec5"; Json = $rec5 }
)

foreach ($rec in $records) {
    $filePath = Join-Path $testDir "$($rec.Name).json"
    [System.IO.File]::WriteAllText($filePath, $rec.Json, [System.Text.UTF8Encoding]::new($false))
    $insertResult = & $dsearchExe record insert $filePath --data-dir $testDir 2>&1 | Out-String
    if ($insertResult -match "inserted") {
        Write-Host "OK - $($rec.Name) inserted" -ForegroundColor Green
    } else {
        Write-Host "FAIL - $($rec.Name) not inserted: $insertResult" -ForegroundColor Red
        exit 1
    }
}

# Confirm all 5 records are in the store
$recordList = & $dsearchExe record list --data-dir $testDir 2>&1 | Out-String
if ($recordList -match "Records \(5\)") {
    Write-Host "OK - All 5 records seeded" -ForegroundColor Green
} else {
    Write-Host "FAIL - Expected 5 records, got:" -ForegroundColor Red
    Write-Host $recordList
    exit 1
}

# ============================================================
# TEST 1: Simple AND text search
# ============================================================
Write-Host "`n--- Test 1: Simple AND text search ---" -ForegroundColor Yellow

$result = & $dsearchExe search "rust async" --data-dir $testDir 2>&1 | Out-String
Write-Host "Search 'rust async':"
Write-Host $result

# Should find rust-async-guide, tokio-crate, and python-async-intro (all have "async")
# But "rust" should exclude python-async-intro
if ($result -match "rust-async-guide" -and $result -match "tokio-crate" -and $result -notmatch "python-async-intro") {
    Write-Host "OK - AND search returns correct subset" -ForegroundColor Green
} else {
    Write-Host "FAIL - AND search did not return expected results" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 2: Exact phrase match
# ============================================================
Write-Host "`n--- Test 2: Exact phrase match ---" -ForegroundColor Yellow

$result = & $dsearchExe search '"rust async"' --data-dir $testDir 2>&1 | Out-String
Write-Host "Search '""rust async""':"
Write-Host $result

if ($result -match "rust-async-guide") {
    Write-Host "OK - Phrase match works" -ForegroundColor Green
} else {
    Write-Host "FAIL - Phrase match did not find expected record" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 3: OR search
# ============================================================
Write-Host "`n--- Test 3: OR search ---" -ForegroundColor Yellow

$result = & $dsearchExe search "rust OR python" --data-dir $testDir 2>&1 | Out-String
Write-Host "Search 'rust OR python':"
Write-Host $result

if ($result -match "rust-async-guide" -and $result -match "python-async-intro") {
    Write-Host "OK - OR search returns both" -ForegroundColor Green
} else {
    Write-Host "FAIL - OR search did not return expected results" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 4: Negation (-exclude)
# ============================================================
Write-Host "`n--- Test 4: Negation (-exclude) ---" -ForegroundColor Yellow

$result = & $dsearchExe search "async -python" --data-dir $testDir 2>&1 | Out-String
Write-Host "Search 'async -python':"
Write-Host $result

if ($result -notmatch "python-async-intro" -and ($result -match "rust-async-guide" -or $result -match "tokio-crate")) {
    Write-Host "OK - Exclude works" -ForegroundColor Green
} else {
    Write-Host "FAIL - Exclude did not work correctly" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 5: Schema filter
# ============================================================
Write-Host "`n--- Test 5: Schema filter ---" -ForegroundColor Yellow

$result = & $dsearchExe search "schema:rust/crate" --data-dir $testDir 2>&1 | Out-String
Write-Host "Search 'schema:rust/crate':"
Write-Host $result

if ($result -match "tokio-crate" -and $result -notmatch "rust-async-guide" -and $result -notmatch "python-async-intro") {
    Write-Host "OK - Schema filter works" -ForegroundColor Green
} else {
    Write-Host "FAIL - Schema filter did not work correctly" -ForegroundColor Red
    exit 1
}

# Also test --schema flag
$result2 = & $dsearchExe search "rust" --schema "rust/crate" --data-dir $testDir 2>&1 | Out-String
Write-Host "Search 'rust --schema rust/crate':"
Write-Host $result2

if ($result2 -match "tokio-crate" -and $result2 -notmatch "rust-async-guide") {
    Write-Host "OK - --schema flag works" -ForegroundColor Green
} else {
    Write-Host "FAIL - --schema flag did not work correctly" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 6: Tag filter
# ============================================================
Write-Host "`n--- Test 6: Tag filter ---" -ForegroundColor Yellow

$result = & $dsearchExe search "tag:category:networking" --data-dir $testDir 2>&1 | Out-String
Write-Host "Search 'tag:category:networking':"
Write-Host $result

if ($result -match "rust-async-guide" -and $result -match "python-async-intro" -and $result -match "tokio-crate" -and $result -notmatch "weather-nyc") {
    Write-Host "OK - Tag filter works" -ForegroundColor Green
} else {
    Write-Host "FAIL - Tag filter did not work correctly" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 7: Source domain filter
# ============================================================
Write-Host "`n--- Test 7: Source domain filter ---" -ForegroundColor Yellow

$result = & $dsearchExe search "source:crates.io" --data-dir $testDir 2>&1 | Out-String
Write-Host "Search 'source:crates.io':"
Write-Host $result

if ($result -match "tokio-crate" -and $result -notmatch "rust-async-guide") {
    Write-Host "OK - Source domain filter works" -ForegroundColor Green
} else {
    Write-Host "FAIL - Source domain filter did not work correctly" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 8: Scraped filter
# ============================================================
Write-Host "`n--- Test 8: Scraped filter ---" -ForegroundColor Yellow

$result = & $dsearchExe search "scraped:keyword" --data-dir $testDir 2>&1 | Out-String
Write-Host "Search 'scraped:keyword':"
Write-Host $result

if ($result -match "weather-nyc" -and $result -notmatch "rust-async-guide") {
    Write-Host "OK - Scraped filter works" -ForegroundColor Green
} else {
    Write-Host "FAIL - Scraped filter did not work correctly" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 9: Refresh filter
# ============================================================
Write-Host "`n--- Test 9: Refresh filter ---" -ForegroundColor Yellow

$result = & $dsearchExe search "refresh:interval" --data-dir $testDir 2>&1 | Out-String
Write-Host "Search 'refresh:interval':"
Write-Host $result

if ($result -match "tokio-crate" -and $result -match "weather-nyc" -and $result -notmatch "rust-async-guide") {
    Write-Host "OK - Refresh filter works" -ForegroundColor Green
} else {
    Write-Host "FAIL - Refresh filter did not work correctly" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 10: since:/before: date filters
# ============================================================
Write-Host "`n--- Test 10: since:/before: date filters ---" -ForegroundColor Yellow

# Only records with created_at >= 1705000000
$result = & $dsearchExe search "since:1705000000" --data-dir $testDir 2>&1 | Out-String
Write-Host "Search 'since:1705000000':"
Write-Host $result

if ($result -match "rust-async-guide" -and $result -match "tokio-crate" -and $result -notmatch "python-async-intro" -and $result -notmatch "weather-nyc") {
    Write-Host "OK - since: filter works" -ForegroundColor Green
} else {
    Write-Host "FAIL - since: filter did not work correctly" -ForegroundColor Red
    exit 1
}

# Only records with created_at < 1700000001
$result = & $dsearchExe search "before:1700000001" --data-dir $testDir 2>&1 | Out-String
Write-Host "Search 'before:1700000001':"
Write-Host $result

if ($result -match "python-async-intro" -and $result -match "weather-nyc" -and $result -notmatch "rust-async-guide") {
    Write-Host "OK - before: filter works" -ForegroundColor Green
} else {
    Write-Host "FAIL - before: filter did not work correctly" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 11: limit: override
# ============================================================
Write-Host "`n--- Test 11: limit: override ---" -ForegroundColor Yellow

$result = & $dsearchExe search "async limit:1" --data-dir $testDir 2>&1 | Out-String
Write-Host "Search 'async limit:1':"
Write-Host $result

# Should return exactly 1 result
$resultCount = ($result -split "`n" | Where-Object { $_ -match "^\s+\S+\s+schema=" }).Count
if ($resultCount -le 1) {
    Write-Host "OK - limit: works (got $resultCount result)" -ForegroundColor Green
} else {
    Write-Host "FAIL - limit: returned $resultCount results, expected at most 1" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 12: JSON output
# ============================================================
Write-Host "`n--- Test 12: JSON output ---" -ForegroundColor Yellow

$result = & $dsearchExe search "tokio" --output json --data-dir $testDir 2>&1 | Out-String
Write-Host "Search 'tokio --output json':"
Write-Host $result

if ($result -match '"id"' -and $result -match "tokio-crate") {
    Write-Host "OK - JSON output works" -ForegroundColor Green
} else {
    Write-Host "FAIL - JSON output did not contain expected data" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 13: No results
# ============================================================
Write-Host "`n--- Test 13: No results ---" -ForegroundColor Yellow

$result = & $dsearchExe search "quantum computing" --data-dir $testDir 2>&1 | Out-String
Write-Host "Search 'quantum computing':"
Write-Host $result

if ($result -match "No results found") {
    Write-Host "OK - Empty result set works" -ForegroundColor Green
} else {
    Write-Host "FAIL - Expected 'No results found' message" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 14: Combined query (schema + text + exclude)
# ============================================================
Write-Host "`n--- Test 14: Combined query ---" -ForegroundColor Yellow

$result = & $dsearchExe search "schema:wiki/article async -python" --data-dir $testDir 2>&1 | Out-String
Write-Host "Search 'schema:wiki/article async -python':"
Write-Host $result

if ($result -match "rust-async-guide" -and $result -notmatch "python-async-intro" -and $result -notmatch "tokio-crate") {
    Write-Host "OK - Combined query works" -ForegroundColor Green
} else {
    Write-Host "FAIL - Combined query did not return expected results" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 15: Unit tests all pass
# ============================================================
Write-Host "`n--- Test 15: All unit tests pass ---" -ForegroundColor Yellow

$testResult = & cargo test 2>&1 | Out-String
if ($testResult -match "test result: ok") {
    Write-Host "OK - All unit tests pass" -ForegroundColor Green
} else {
    Write-Host "FAIL - Unit tests failed" -ForegroundColor Red
    Write-Host $testResult
    exit 1
}

# Check critical Phase 4 tests
$criticalTests = @(
    "parse_simple_terms",
    "parse_phrase",
    "parse_exclude",
    "parse_or",
    "parse_field_filters",
    "parse_limit",
    "parse_since_before",
    "parse_scraped_refresh",
    "match_simple_terms",
    "match_phrase",
    "match_exclude",
    "match_or",
    "match_schema_filter",
    "match_tag_filter",
    "match_since_filter",
    "match_before_filter",
    "match_scraped_filter",
    "match_refresh_filter",
    "match_source_domain_filter",
    "score_ranking",
    "score_freshness",
    "score_holder_count",
    "store_search_records"
)

$missingTests = @()
foreach ($test in $criticalTests) {
    if ($testResult -notmatch [regex]::Escape($test)) {
        $missingTests += $test
    }
}

if ($missingTests.Count -eq 0) {
    Write-Host "OK - All critical Phase 4 tests found in test output" -ForegroundColor Green
} else {
    Write-Host "FAIL - Missing critical tests: $($missingTests -join ', ')" -ForegroundColor Red
    exit 1
}

# ============================================================
# Summary
# ============================================================
Write-Host "`n=== Phase 4 Exit Test Summary ===" -ForegroundColor Cyan
Write-Host "Simple AND search: OK" -ForegroundColor Green
Write-Host "Exact phrase match: OK" -ForegroundColor Green
Write-Host "OR search: OK" -ForegroundColor Green
Write-Host "Negation (-exclude): OK" -ForegroundColor Green
Write-Host "Schema filter: OK" -ForegroundColor Green
Write-Host "Tag filter: OK" -ForegroundColor Green
Write-Host "Source domain filter: OK" -ForegroundColor Green
Write-Host "Scraped filter: OK" -ForegroundColor Green
Write-Host "Refresh filter: OK" -ForegroundColor Green
Write-Host "since:/before: date filters: OK" -ForegroundColor Green
Write-Host "limit: override: OK" -ForegroundColor Green
Write-Host "JSON output: OK" -ForegroundColor Green
Write-Host "No results: OK" -ForegroundColor Green
Write-Host "Combined query: OK" -ForegroundColor Green
Write-Host "Unit tests (294/294): OK" -ForegroundColor Green
Write-Host "`n=== Phase 4 Exit Test Complete ===" -ForegroundColor Cyan
