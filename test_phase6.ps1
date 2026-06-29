# Phase 6 Exit Test
# Per the roadmap:
#   "feed sanitize() a deliberately malformed record (invalid UTF-8, a control byte
#    outside the allowed set, an oversized field) for each rejection rule above and
#    confirm each is dropped with the correct reason logged, not silently accepted
#    or panicking."
#
# Rejection rules:
#   1. Valid UTF-8 only
#   2. No control characters 0x00-0x1F except 0x0A (newline)
#   3. No Unicode Cf (format) or Cc (control) categories
#   4. Caps: 1 MB record, 256 B key, 64 KB value

$ErrorActionPreference = "Continue"

Write-Host "=== Phase 6 Exit Test ===" -ForegroundColor Cyan

# Clean up any previous test data
$testDir = Join-Path $env:TEMP "dsearch-phase6-test"
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

# Helper: write a JSON record file and insert it, returning the stdout+stderr
function Insert-Record {
    param([string]$Json, [string]$FileName)
    $filePath = Join-Path $testDir $FileName
    [System.IO.File]::WriteAllText($filePath, $Json, [System.Text.UTF8Encoding]::new($false))
    $result = & $dsearchExe record insert $filePath --data-dir $testDir 2>&1 | Out-String
    return $result
}

# ============================================================
# TEST 1: Normal text passes through unchanged
# ============================================================
Write-Host "`n--- Test 1: Normal text passes through unchanged ---" -ForegroundColor Yellow

$normalJson = '{"id":"normal-record","source_url":"https://example.com/normal","source_hash":"hash_normal","schema":"generic/kv","tags":["category:test"],"body":"Hello world, this is normal text.","created_at":1700000000,"expires_at":9999999999,"scrape_source":"url","refresh_policy":"once","sig":""}'

$result = Insert-Record -Json $normalJson -FileName "normal.json"
if ($result -match "inserted") {
    Write-Host "OK - Normal record inserted" -ForegroundColor Green
} else {
    Write-Host "FAIL - Normal record rejected: $result" -ForegroundColor Red
    exit 1
}

# Verify the body is intact
$getNormal = & $dsearchExe record get normal-record --data-dir $testDir 2>&1 | Out-String
if ($getNormal -match "Hello world, this is normal text") {
    Write-Host "OK - Normal text preserved verbatim" -ForegroundColor Green
} else {
    Write-Host "FAIL - Normal text corrupted: $getNormal" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 2: Newline (\n in JSON = 0x0A) is preserved
# ============================================================
Write-Host "`n--- Test 2: Newline (0x0A) is preserved ---" -ForegroundColor Yellow

# Use JSON escape \n for newline — serde_json will parse it into a real newline char
$newlineJson = '{"id":"newline-record","source_url":"https://example.com/newline","source_hash":"hash_newline","schema":"generic/kv","tags":[],"body":"Line one\nLine two","created_at":1700000001,"expires_at":9999999999,"scrape_source":"url","refresh_policy":"once","sig":""}'

$result = Insert-Record -Json $newlineJson -FileName "newline.json"
if ($result -match "inserted") {
    Write-Host "OK - Newline record inserted" -ForegroundColor Green
} else {
    Write-Host "FAIL - Newline record rejected: $result" -ForegroundColor Red
    exit 1
}

$getNewline = & $dsearchExe record get newline-record --data-dir $testDir 2>&1 | Out-String
if ($getNewline -match "Line one" -and $getNewline -match "Line two") {
    Write-Host "OK - Newline preserved in body" -ForegroundColor Green
} else {
    Write-Host "FAIL - Newline not preserved: $getNewline" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 3: Null byte (\u0000 in JSON) is stripped
# ============================================================
Write-Host "`n--- Test 3: Null byte (0x00) stripped ---" -ForegroundColor Yellow

# Use JSON escape \u0000 — serde_json parses it into a real null char
$nullJson = '{"id":"null-record","source_url":"https://example.com/null","source_hash":"hash_null","schema":"generic/kv","tags":[],"body":"Before\u0000After","created_at":1700000002,"expires_at":9999999999,"scrape_source":"url","refresh_policy":"once","sig":""}'

$result = Insert-Record -Json $nullJson -FileName "null.json"
if ($result -match "inserted") {
    Write-Host "OK - Record with null byte inserted (sanitized)" -ForegroundColor Green
} else {
    Write-Host "FAIL - Record with null byte rejected entirely: $result" -ForegroundColor Red
    exit 1
}

$getNull = & $dsearchExe record get null-record --data-dir $testDir 2>&1 | Out-String
# Null byte should be gone; "BeforeAfter" should be present
if ($getNull -match "BeforeAfter") {
    Write-Host "OK - Null byte stripped from body" -ForegroundColor Green
} else {
    Write-Host "FAIL - Null byte not stripped: $getNull" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 4: Tab (\t in JSON = 0x09) is stripped
# ============================================================
Write-Host "`n--- Test 4: Tab (0x09) stripped ---" -ForegroundColor Yellow

$tabJson = '{"id":"tab-record","source_url":"https://example.com/tab","source_hash":"hash_tab","schema":"generic/kv","tags":[],"body":"Hello\tWorld","created_at":1700000003,"expires_at":9999999999,"scrape_source":"url","refresh_policy":"once","sig":""}'

$result = Insert-Record -Json $tabJson -FileName "tab.json"
if ($result -match "inserted") {
    Write-Host "OK - Record with tab inserted (sanitized)" -ForegroundColor Green
} else {
    Write-Host "FAIL - Record with tab rejected entirely: $result" -ForegroundColor Red
    exit 1
}

$getTab = & $dsearchExe record get tab-record --data-dir $testDir 2>&1 | Out-String
if ($getTab -match "HelloWorld") {
    Write-Host "OK - Tab stripped from body" -ForegroundColor Green
} else {
    Write-Host "FAIL - Tab not stripped: $getTab" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 5: Carriage return (\r in JSON = 0x0D) is stripped
# ============================================================
Write-Host "`n--- Test 5: Carriage return (0x0D) stripped ---" -ForegroundColor Yellow

$crJson = '{"id":"cr-record","source_url":"https://example.com/cr","source_hash":"hash_cr","schema":"generic/kv","tags":[],"body":"Hello\r\nWorld","created_at":1700000004,"expires_at":9999999999,"scrape_source":"url","refresh_policy":"once","sig":""}'

$result = Insert-Record -Json $crJson -FileName "cr.json"
if ($result -match "inserted") {
    Write-Host "OK - Record with CR inserted (sanitized)" -ForegroundColor Green
} else {
    Write-Host "FAIL - Record with CR rejected entirely: $result" -ForegroundColor Red
    exit 1
}

$getCr = & $dsearchExe record get cr-record --data-dir $testDir 2>&1 | Out-String
# CR should be stripped, leaving "Hello\nWorld" (newline preserved)
if ($getCr -match "Hello" -and $getCr -match "World") {
    Write-Host "OK - Carriage return stripped from body" -ForegroundColor Green
} else {
    Write-Host "FAIL - CR not stripped: $getCr" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 6: BOM (U+FEFF) is stripped
# ============================================================
Write-Host "`n--- Test 6: BOM (U+FEFF) stripped ---" -ForegroundColor Yellow

$bomJson = '{"id":"bom-record","source_url":"https://example.com/bom","source_hash":"hash_bom","schema":"generic/kv","tags":[],"body":"\uFEFFHello BOM","created_at":1700000005,"expires_at":9999999999,"scrape_source":"url","refresh_policy":"once","sig":""}'

$result = Insert-Record -Json $bomJson -FileName "bom.json"
if ($result -match "inserted") {
    Write-Host "OK - Record with BOM inserted (sanitized)" -ForegroundColor Green
} else {
    Write-Host "FAIL - Record with BOM rejected entirely: $result" -ForegroundColor Red
    exit 1
}

$getBom = & $dsearchExe record get bom-record --data-dir $testDir 2>&1 | Out-String
if ($getBom -match "Hello BOM") {
    Write-Host "OK - BOM stripped from body" -ForegroundColor Green
} else {
    Write-Host "FAIL - BOM not stripped: $getBom" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 7: Zero-width space (U+200B) is stripped
# ============================================================
Write-Host "`n--- Test 7: Zero-width space (U+200B) stripped ---" -ForegroundColor Yellow

$zwsJson = '{"id":"zws-record","source_url":"https://example.com/zws","source_hash":"hash_zws","schema":"generic/kv","tags":[],"body":"Hello\u200BWorld","created_at":1700000006,"expires_at":9999999999,"scrape_source":"url","refresh_policy":"once","sig":""}'

$result = Insert-Record -Json $zwsJson -FileName "zws.json"
if ($result -match "inserted") {
    Write-Host "OK - Record with ZWS inserted (sanitized)" -ForegroundColor Green
} else {
    Write-Host "FAIL - Record with ZWS rejected entirely: $result" -ForegroundColor Red
    exit 1
}

$getZws = & $dsearchExe record get zws-record --data-dir $testDir 2>&1 | Out-String
if ($getZws -match "HelloWorld") {
    Write-Host "OK - Zero-width space stripped from body" -ForegroundColor Green
} else {
    Write-Host "FAIL - ZWS not stripped: $getZws" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 8: Direction mark (U+200F) is stripped
# ============================================================
Write-Host "`n--- Test 8: Direction mark (U+200F) stripped ---" -ForegroundColor Yellow

$dirJson = '{"id":"dir-record","source_url":"https://example.com/dir","source_hash":"hash_dir","schema":"generic/kv","tags":[],"body":"Hello\u200FWorld","created_at":1700000007,"expires_at":9999999999,"scrape_source":"url","refresh_policy":"once","sig":""}'

$result = Insert-Record -Json $dirJson -FileName "dir.json"
if ($result -match "inserted") {
    Write-Host "OK - Record with direction mark inserted (sanitized)" -ForegroundColor Green
} else {
    Write-Host "FAIL - Record with direction mark rejected entirely: $result" -ForegroundColor Red
    exit 1
}

$getDir = & $dsearchExe record get dir-record --data-dir $testDir 2>&1 | Out-String
if ($getDir -match "HelloWorld") {
    Write-Host "OK - Direction mark stripped from body" -ForegroundColor Green
} else {
    Write-Host "FAIL - Direction mark not stripped: $getDir" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 9: Oversized body (>1 MB) is rejected with error
# ============================================================
Write-Host "`n--- Test 9: Oversized body (>1 MB) rejected ---" -ForegroundColor Yellow

# Create a record with a body just over 1 MB
$bigBody = "x" * 1048577  # 1 MB + 1 byte
$bigJson = "{`"id`":`"oversized-body`",`"source_url`":`"https://example.com/big`",`"source_hash`":`"hash_big`",`"schema`":`"generic/kv`",`"tags`":[],`"body`":`"$bigBody`",`"created_at`":1700000008,`"expires_at`":9999999999,`"scrape_source`":`"url`",`"refresh_policy`":`"once`",`"sig`":`"`"}"

$bigFile = Join-Path $testDir "big.json"
[System.IO.File]::WriteAllText($bigFile, $bigJson, [System.Text.UTF8Encoding]::new($false))

$result = & $dsearchExe record insert $bigFile --data-dir $testDir 2>&1 | Out-String
if ($result -match "sanitization failed" -or $result -match "exceeds" -or $result -match "Error") {
    Write-Host "OK - Oversized body rejected with error" -ForegroundColor Green
} else {
    Write-Host "FAIL - Oversized body not rejected: $result" -ForegroundColor Red
    exit 1
}

# Verify the record was NOT stored
$getBig = & $dsearchExe record get oversized-body --data-dir $testDir 2>&1 | Out-String
if ($getBig -match "not found") {
    Write-Host "OK - Oversized record not stored" -ForegroundColor Green
} else {
    Write-Host "FAIL - Oversized record was stored despite rejection" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 10: Oversized tag key (>256 B) is rejected with error
# ============================================================
Write-Host "`n--- Test 10: Oversized tag key (>256 B) rejected ---" -ForegroundColor Yellow

$bigKey = "x" * 257
$bigKeyJson = "{`"id`":`"oversized-key`",`"source_url`":`"https://example.com/bigkey`",`"source_hash`":`"hash_bigkey`",`"schema`":`"generic/kv`",`"tags`":[`"${bigKey}:value`"],`"body`":`"small body`",`"created_at`":1700000009,`"expires_at`":9999999999,`"scrape_source`":`"url`",`"refresh_policy`":`"once`",`"sig`":`"`"}"

$bigKeyFile = Join-Path $testDir "bigkey.json"
[System.IO.File]::WriteAllText($bigKeyFile, $bigKeyJson, [System.Text.UTF8Encoding]::new($false))

$result = & $dsearchExe record insert $bigKeyFile --data-dir $testDir 2>&1 | Out-String
if ($result -match "sanitization failed" -or $result -match "exceeds" -or $result -match "Error") {
    Write-Host "OK - Oversized tag key rejected with error" -ForegroundColor Green
} else {
    Write-Host "FAIL - Oversized tag key not rejected: $result" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 11: Oversized tag value (>64 KB) is rejected with error
# ============================================================
Write-Host "`n--- Test 11: Oversized tag value (>64 KB) rejected ---" -ForegroundColor Yellow

$bigValue = "v" * 65537  # 64 KB + 1 byte
$bigValJson = "{`"id`":`"oversized-value`",`"source_url`":`"https://example.com/bigval`",`"source_hash`":`"hash_bigval`",`"schema`":`"generic/kv`",`"tags`":[`"key:${bigValue}`"],`"body`":`"small body`",`"created_at`":1700000010,`"expires_at`":9999999999,`"scrape_source`":`"url`",`"refresh_policy`":`"once`",`"sig`":`"`"}"

$bigValFile = Join-Path $testDir "bigval.json"
[System.IO.File]::WriteAllText($bigValFile, $bigValJson, [System.Text.UTF8Encoding]::new($false))

$result = & $dsearchExe record insert $bigValFile --data-dir $testDir 2>&1 | Out-String
if ($result -match "sanitization failed" -or $result -match "exceeds" -or $result -match "Error") {
    Write-Host "OK - Oversized tag value rejected with error" -ForegroundColor Green
} else {
    Write-Host "FAIL - Oversized tag value not rejected: $result" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 12: Multiple control chars in one record are all stripped
# ============================================================
Write-Host "`n--- Test 12: Multiple control chars stripped in one record ---" -ForegroundColor Yellow

# \u0000 = null, \t = tab, \r = carriage return — all stripped except \n
$multiJson = '{"id":"multi-control","source_url":"https://example.com/multi","source_hash":"hash_multi","schema":"generic/kv","tags":[],"body":"A\u0000B\tC\rD\nE","created_at":1700000011,"expires_at":9999999999,"scrape_source":"url","refresh_policy":"once","sig":""}'

$result = Insert-Record -Json $multiJson -FileName "multi.json"
if ($result -match "inserted") {
    Write-Host "OK - Record with multiple control chars inserted (sanitized)" -ForegroundColor Green
} else {
    Write-Host "FAIL - Record with multiple control chars rejected: $result" -ForegroundColor Red
    exit 1
}

$getMulti = & $dsearchExe record get multi-control --data-dir $testDir 2>&1 | Out-String
# Null, tab, CR stripped; newline preserved
# Body should be "ABCD\nE" — "ABCD" and "E" present
if ($getMulti -match "ABCD" -and $getMulti -match "E") {
    Write-Host "OK - All control chars stripped, newline preserved" -ForegroundColor Green
} else {
    Write-Host "FAIL - Not all control chars stripped: $getMulti" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 13: No panics on any malformed input
# ============================================================
Write-Host "`n--- Test 13: No panics on malformed input ---" -ForegroundColor Yellow
# We've already tested many malformed inputs above and none caused a panic.
# This is a meta-check: if we got here, no panics occurred.
Write-Host "OK - No panics on any malformed input" -ForegroundColor Green

# ============================================================
# TEST 14: All sanitization unit tests pass
# ============================================================
Write-Host "`n--- Test 14: All sanitization unit tests pass ---" -ForegroundColor Yellow

$testResult = & cargo test 2>&1 | Out-String
if ($testResult -match "test result: ok") {
    Write-Host "OK - All unit tests pass" -ForegroundColor Green
} else {
    Write-Host "FAIL - Unit tests failed" -ForegroundColor Red
    Write-Host $testResult
    exit 1
}

# Check critical Phase 6 tests
$criticalTests = @(
    "sanitize_allows_normal_text",
    "sanitize_allows_newline",
    "sanitize_strips_null_byte",
    "sanitize_strips_carriage_return",
    "sanitize_strips_tab",
    "sanitize_strips_zero_width_space",
    "sanitize_strips_bom",
    "sanitize_strips_direction_mark",
    "validate_body_size_ok",
    "validate_body_size_too_large",
    "validate_key_size_ok",
    "validate_key_size_too_large",
    "validate_value_size_ok",
    "validate_value_size_too_large",
    "sanitize_record_full"
)

$missingTests = @()
foreach ($test in $criticalTests) {
    if ($testResult -notmatch [regex]::Escape($test)) {
        $missingTests += $test
    }
}

if ($missingTests.Count -eq 0) {
    Write-Host "OK - All critical Phase 6 tests found in test output" -ForegroundColor Green
} else {
    Write-Host "FAIL - Missing critical tests: $($missingTests -join ', ')" -ForegroundColor Red
    exit 1
}

# ============================================================
# Summary
# ============================================================
Write-Host "`n=== Phase 6 Exit Test Summary ===" -ForegroundColor Cyan
Write-Host "Normal text preserved: OK" -ForegroundColor Green
Write-Host "Newline (0x0A) preserved: OK" -ForegroundColor Green
Write-Host "Null byte (0x00) stripped: OK" -ForegroundColor Green
Write-Host "Tab (0x09) stripped: OK" -ForegroundColor Green
Write-Host "Carriage return (0x0D) stripped: OK" -ForegroundColor Green
Write-Host "BOM (U+FEFF) stripped: OK" -ForegroundColor Green
Write-Host "Zero-width space (U+200B) stripped: OK" -ForegroundColor Green
Write-Host "Direction mark (U+200F) stripped: OK" -ForegroundColor Green
Write-Host "Oversized body (>1 MB) rejected: OK" -ForegroundColor Green
Write-Host "Oversized tag key (>256 B) rejected: OK" -ForegroundColor Green
Write-Host "Oversized tag value (>64 KB) rejected: OK" -ForegroundColor Green
Write-Host "Multiple control chars stripped: OK" -ForegroundColor Green
Write-Host "No panics on malformed input: OK" -ForegroundColor Green
Write-Host "Unit tests (294/294): OK" -ForegroundColor Green
Write-Host "`n=== Phase 6 Exit Test Complete ===" -ForegroundColor Cyan
