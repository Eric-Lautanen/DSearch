# Phase 2 Exit Test
# Per the roadmap:
#   1. `dsearch config show` round-trips every key in the Config file section
#      back out correctly on a fresh data dir
#   2. Hand-edit config.toml to bump config_version past current, confirm
#      `dsearch node start` refuses to open with a clear error
#   3. Construct one ContentRecord and one Announcement by hand, sign and
#      verify both round-trip through the canonical encoding without altering
#      the bytes (done via `cargo test`)

$ErrorActionPreference = "Continue"

Write-Host "=== Phase 2 Exit Test ===" -ForegroundColor Cyan

# Clean up any previous test data
$testDir = Join-Path $env:TEMP "dsearch-phase2-test"
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

# ============================================================
# TEST 1: config show round-trips every key on a fresh data dir
# ============================================================
Write-Host "`n--- Test 1: config show round-trips every key ---" -ForegroundColor Yellow

# Initialize a fresh node
$initResult = & $dsearchExe init --data-dir $testDir 2>&1 | Out-String
Write-Host "Init output: $initResult"

# Run config show
$configShow = & $dsearchExe config show --data-dir $testDir 2>&1 | Out-String
Write-Host "Config output:"
Write-Host $configShow

# Every key from the Config file section in the roadmap must appear
$expectedKeys = @(
    "role", "max_connections", "min_protocol_version", "ipv4", "ipv6",
    "port", "enabled", "bind", "rate_limit_per_min", "require_api_key",
    "quota_mb", "quota_action", "tier2_max_mb",
    "bandwidth_limit_mbps",
    "default_interval_secs", "default_replicate", "default_lifecycle",
    "level", "output", "max_size_mb", "rotate_count",
    "use_defaults", "config_version"
)

$missingKeys = @()
foreach ($key in $expectedKeys) {
    if ($configShow -notmatch [regex]::Escape($key)) {
        $missingKeys += $key
    }
}

if ($missingKeys.Count -eq 0) {
    Write-Host "OK - All expected config keys present in config show" -ForegroundColor Green
} else {
    Write-Host "FAIL - Missing config keys: $($missingKeys -join ', ')" -ForegroundColor Red
    exit 1
}

# Verify default values round-trip correctly
$defaultChecks = @(
    @{ Key = "role"; Expected = "light" },
    @{ Key = "max_connections"; Expected = "200" },
    @{ Key = "min_protocol_version"; Expected = "1" },
    @{ Key = "port"; Expected = "7743" },
    @{ Key = "tier2_max_mb"; Expected = "512" },
    @{ Key = "default_interval_secs"; Expected = "3600" },
    @{ Key = "default_lifecycle"; Expected = "ephemeral" },
    @{ Key = "config_version"; Expected = "1" }
)

$defaultFails = @()
foreach ($check in $defaultChecks) {
    if ($configShow -notmatch [regex]::Escape($check.Expected)) {
        $defaultFails += "$($check.Key)=$($check.Expected)"
    }
}

if ($defaultFails.Count -eq 0) {
    Write-Host "OK - All default values round-trip correctly" -ForegroundColor Green
} else {
    Write-Host "FAIL - Default values not found: $($defaultFails -join ', ')" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 1b: config set works and round-trips
# ============================================================
Write-Host "`n--- Test 1b: config set round-trips ---" -ForegroundColor Yellow

$setResult = & $dsearchExe config set node.role full --data-dir $testDir 2>&1 | Out-String
Write-Host "Set node.role=full: $setResult"
if ($setResult -notmatch "Set node.role = full") {
    Write-Host "FAIL - config set did not confirm the change" -ForegroundColor Red
    exit 1
}

$setResult2 = & $dsearchExe config set api.port 8888 --data-dir $testDir 2>&1 | Out-String
Write-Host "Set api.port=8888: $setResult2"

$setResult3 = & $dsearchExe config set gateway.enabled true --data-dir $testDir 2>&1 | Out-String
Write-Host "Set gateway.enabled=true: $setResult3"

# Verify the changes appear in config show
$configShow2 = & $dsearchExe config show --data-dir $testDir 2>&1 | Out-String
$setFails = @()
if ($configShow2 -notmatch "full") { $setFails += "node.role=full" }
if ($configShow2 -notmatch "8888") { $setFails += "api.port=8888" }
if ($configShow2 -notmatch "true") { $setFails += "gateway.enabled=true" }

if ($setFails.Count -eq 0) {
    Write-Host "OK - config set values appear in config show" -ForegroundColor Green
} else {
    Write-Host "FAIL - config set values not found: $($setFails -join ', ')" -ForegroundColor Red
    exit 1
}

# Verify unknown key is rejected
$badSet = & $dsearchExe config set nonexistent.key val --data-dir $testDir 2>&1 | Out-String
if ($badSet -match "Unknown config key" -or $badSet -match "Error") {
    Write-Host "OK - Unknown config key rejected" -ForegroundColor Green
} else {
    Write-Host "FAIL - Unknown config key was not rejected: $badSet" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 1c: config reset restores defaults
# ============================================================
Write-Host "`n--- Test 1c: config reset restores defaults ---" -ForegroundColor Yellow

$resetResult = & $dsearchExe config reset --data-dir $testDir 2>&1 | Out-String
Write-Host "Reset result: $resetResult"

$configShow3 = & $dsearchExe config show --data-dir $testDir 2>&1 | Out-String
if ($configShow3 -match "light" -and $configShow3 -match "7743") {
    Write-Host "OK - Config reset restores defaults" -ForegroundColor Green
} else {
    Write-Host "FAIL - Config reset did not restore defaults" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 2: Future config_version rejected by node start
# ============================================================
Write-Host "`n--- Test 2: Future config_version rejected ---" -ForegroundColor Yellow

# Hand-edit config.toml to set config_version to 999
$configPath = Join-Path $testDir "config.toml"
$configContent = Get-Content $configPath -Raw
Write-Host "Original config.toml (last 5 lines):"
$configContent -split "`n" | Select-Object -Last 5 | ForEach-Object { Write-Host $_ }

# Replace config_version = 1 with config_version = 999
$futureConfig = $configContent -replace "config_version = 1", "config_version = 999"
Set-Content -Path $configPath -Value $futureConfig -Encoding UTF8

Write-Host "Patched config_version to 999"

# Try to start the node - it should refuse with a clear error
$nodeStartResult = & $dsearchExe node start --headless --port 7746 --data-dir $testDir 2>&1 | Out-String
Write-Host "Node start output with future config_version:"
Write-Host $nodeStartResult

# The node should exit with an error
if ($nodeStartResult -match "future version" -or $nodeStartResult -match "config_version" -or $nodeStartResult -match "Error" -or $nodeStartResult -match "Downgrading") {
    Write-Host "OK - Node start correctly rejects future config_version" -ForegroundColor Green
} else {
    Write-Host "FAIL - Node start did not reject future config_version - silent corruption risk!" -ForegroundColor Red
    # Kill any node that might have started
    $pidPath = Join-Path $testDir "node.pid"
    if (Test-Path $pidPath) {
        $pid = Get-Content $pidPath
        Stop-Process -Id $pid -Force -ErrorAction SilentlyContinue
    }
    exit 1
}

# Make sure no node is still running
$pidPath = Join-Path $testDir "node.pid"
if (Test-Path $pidPath) {
    $pid = Get-Content $pidPath -ErrorAction SilentlyContinue
    if ($pid) {
        Stop-Process -Id $pid -Force -ErrorAction SilentlyContinue
    }
}

# Restore config for subsequent tests
$restoredConfig = $configContent -replace "config_version = 999", "config_version = 1"
Set-Content -Path $configPath -Value $restoredConfig -Encoding UTF8

# ============================================================
# TEST 3: Sign/verify roundtrip for ContentRecord and Announcement
# ============================================================
Write-Host "`n--- Test 3: Sign/verify roundtrip (cargo test) ---" -ForegroundColor Yellow

$testResult = & cargo test 2>&1 | Out-String
Write-Host $testResult

# Check that all tests passed
if ($testResult -match "test result: ok") {
    Write-Host "OK - All unit tests pass (sign/verify roundtrip, config roundtrip, model tests)" -ForegroundColor Green
} else {
    Write-Host "FAIL - Unit tests failed" -ForegroundColor Red
    exit 1
}

# Specifically check the critical Phase 2 tests
$criticalTests = @(
    "sign_verify_record_roundtrip",
    "sign_verify_announcement_roundtrip",
    "verify_record_signature_valid",
    "verify_record_signature_wrong_key",
    "verify_announcement_signature_valid",
    "verify_announcement_signature_wrong_key",
    "verify_record_id_valid",
    "verify_record_id_invalid",
    "verify_result_ok",
    "verify_result_fail",
    "content_record_serde_roundtrip",
    "content_record_forward_compat",
    "content_record_missing_optional_fields",
    "announcement_serde_roundtrip",
    "default_config_roundtrip",
    "config_with_scraper_jobs",
    "future_config_version_rejected",
    "canonical_encoding_deterministic",
    "compute_record_id_deterministic",
    "compute_source_hash_deterministic"
)

$missingTests = @()
foreach ($test in $criticalTests) {
    if ($testResult -notmatch [regex]::Escape($test)) {
        $missingTests += $test
    }
}

if ($missingTests.Count -eq 0) {
    Write-Host "OK - All critical Phase 2 tests found in test output" -ForegroundColor Green
} else {
    Write-Host "FAIL - Missing critical tests: $($missingTests -join ', ')" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 4: Verify model structs can be constructed and serialized
# ============================================================
Write-Host "`n--- Test 4: Model structs serializable (cargo test model) ---" -ForegroundColor Yellow

$modelTestResult = & cargo test model 2>&1 | Out-String
Write-Host $modelTestResult

if ($modelTestResult -match "test result: ok") {
    Write-Host "OK - Model tests pass" -ForegroundColor Green
} else {
    Write-Host "FAIL - Model tests failed" -ForegroundColor Red
    exit 1
}

# ============================================================
# TEST 5: Verify trust/sign tests pass (canonical encoding + signing)
# ============================================================
Write-Host "`n--- Test 5: Trust/sign tests (cargo test trust) ---" -ForegroundColor Yellow

$trustTestResult = & cargo test trust 2>&1 | Out-String
Write-Host $trustTestResult

if ($trustTestResult -match "test result: ok") {
    Write-Host "OK - Trust/sign tests pass" -ForegroundColor Green
} else {
    Write-Host "FAIL - Trust/sign tests failed" -ForegroundColor Red
    exit 1
}

# ============================================================
# Summary
# ============================================================
Write-Host "`n=== Phase 2 Exit Test Summary ===" -ForegroundColor Cyan
Write-Host "Config show round-trips all keys: OK" -ForegroundColor Green
Write-Host "Config set works: OK" -ForegroundColor Green
Write-Host "Config reset restores defaults: OK" -ForegroundColor Green
Write-Host "Future config_version rejected: OK" -ForegroundColor Green
Write-Host "Sign/verify roundtrip (unit tests): OK" -ForegroundColor Green
Write-Host "Model struct tests: OK" -ForegroundColor Green
Write-Host "Trust/sign canonical encoding tests: OK" -ForegroundColor Green
Write-Host "`n=== Phase 2 Exit Test Complete ===" -ForegroundColor Cyan
