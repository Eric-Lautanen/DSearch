# Phase 7 — Agent API + CLI Exit Test
# Tests: local HTTP API, port auto-increment, all routes, CLI/API JSON parity

$ErrorActionPreference = "Stop"

# --- Pre-cleanup: kill any leftover dsearch processes ---
Write-Host "[CLEANUP] Killing any leftover dsearch processes..."
Get-Process -Name "dsearch" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1

# --- Setup ---
$TestDir1 = "$env:TEMP\dsearch_phase7_node1"
$TestDir2 = "$env:TEMP\dsearch_phase7_node2"

# Clean up any previous test runs
if (Test-Path $TestDir1) { Remove-Item -Recurse -Force $TestDir1 }
if (Test-Path $TestDir2) { Remove-Item -Recurse -Force $TestDir2 }

# Ensure cleanup on exit (even on error)
$script:node1 = $null
$script:node2 = $null
$Dsearch = ".\target\release\dsearch.exe"

function Cleanup-Test {
    Write-Host "[CLEANUP] Running cleanup..."
    # Try graceful stop
    if (Test-Path $TestDir1) { try { & $Dsearch --data-dir $TestDir1 node stop 2>$null | Out-Null } catch {} }
    if (Test-Path $TestDir2) { try { & $Dsearch --data-dir $TestDir2 node stop 2>$null | Out-Null } catch {} }
    Start-Sleep -Seconds 2
    # Force kill any remaining dsearch processes
    Get-Process -Name "dsearch" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1
    # Clean up test directories
    if (Test-Path $TestDir1) { Remove-Item -Recurse -Force $TestDir1 -ErrorAction SilentlyContinue }
    if (Test-Path $TestDir2) { Remove-Item -Recurse -Force $TestDir2 -ErrorAction SilentlyContinue }
    Write-Host "[CLEANUP] Done"
}

# Register cleanup for all exit paths
try {

New-Item -ItemType Directory -Force -Path $TestDir1 | Out-Null
New-Item -ItemType Directory -Force -Path $TestDir2 | Out-Null

# Build the binary
Write-Host "[BUILD] Building dsearch..."
$buildProc = Start-Process -FilePath "cargo" -ArgumentList "build","--release" -NoNewWindow -Wait -PassThru -RedirectStandardError "$env:TEMP\dsearch_build_err.log"
if ($buildProc.ExitCode -ne 0) {
    Write-Host "[FAIL] Build failed"
    Get-Content "$env:TEMP\dsearch_build_err.log"
    exit 1
}
Write-Host "[PASS] Build succeeded"

$Dsearch = ".\target\release\dsearch.exe"

# --- Test 1: Init both nodes ---
Write-Host ""
Write-Host "[TEST 1] Init both nodes"
& $Dsearch --data-dir $TestDir1 init 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Host "[FAIL] init node1"; exit 1 }

& $Dsearch --data-dir $TestDir2 init 2>&1 | Out-Null
if ($LASTEXITCODE -ne 0) { Write-Host "[FAIL] init node2"; exit 1 }

Write-Host "[PASS] Both nodes initialized"

# --- Test 2: Start node1 on default port ---
Write-Host ""
Write-Host "[TEST 2] Start node1 (should bind API on port 7743)"
$node1 = Start-Process -FilePath $Dsearch -ArgumentList "--data-dir",$TestDir1,"node","start","--headless" -PassThru -NoNewWindow -RedirectStandardOutput "$TestDir1\node1.log" -RedirectStandardError "$TestDir1\node1.err"

Start-Sleep -Seconds 3

# Check api.port file
$portFile1 = Join-Path $TestDir1 "api.port"
if (-not (Test-Path $portFile1)) {
    Write-Host "[FAIL] api.port file not created for node1"
    Stop-Process -Id $node1.Id -Force -ErrorAction SilentlyContinue
    exit 1
}
$port1 = (Get-Content $portFile1).Trim()
Write-Host "  Node1 API port: $port1"
if ($port1 -ne "7743") {
    Write-Host "[FAIL] Expected port 7743, got $port1"
    Stop-Process -Id $node1.Id -Force -ErrorAction SilentlyContinue
    exit 1
}
Write-Host "[PASS] Node1 API on port 7743"

# --- Test 3: Start node2 — should auto-increment API port ---
Write-Host ""
Write-Host "[TEST 3] Start node2 (should auto-increment API port since 7743 is taken)"
# Set node2's config to use port 7743 as well (default), and a different QUIC port
$node2 = Start-Process -FilePath $Dsearch -ArgumentList "--data-dir",$TestDir2,"node","start","--headless","--port","7745" -PassThru -NoNewWindow -RedirectStandardOutput "$TestDir2\node2.log" -RedirectStandardError "$TestDir2\node2.err"

Start-Sleep -Seconds 3

$portFile2 = Join-Path $TestDir2 "api.port"
if (-not (Test-Path $portFile2)) {
    Write-Host "[FAIL] api.port file not created for node2"
    Stop-Process -Id $node1.Id -Force -ErrorAction SilentlyContinue
    Stop-Process -Id $node2.Id -Force -ErrorAction SilentlyContinue
    exit 1
}
$port2 = (Get-Content $portFile2).Trim()
Write-Host "  Node2 API port: $port2"
if ($port2 -eq "7743") {
    Write-Host "[FAIL] Node2 should have auto-incremented from 7743, but got same port"
    Stop-Process -Id $node1.Id -Force -ErrorAction SilentlyContinue
    Stop-Process -Id $node2.Id -Force -ErrorAction SilentlyContinue
    exit 1
}
Write-Host "[PASS] Node2 auto-incremented to port $port2"

# --- Test 4: Hit every API route on node1 ---
Write-Host ""
Write-Host "[TEST 4] Hit every local API route"

function Api-Get {
    param([int]$Port, [string]$Path)
    try {
        $tcp = New-Object System.Net.Sockets.TcpClient
        $tcp.Connect("127.0.0.1", $Port)
        $stream = $tcp.GetStream()
        $writer = New-Object System.IO.StreamWriter($stream)
        $reader = New-Object System.IO.StreamReader($stream)
        $writer.Write("GET $Path HTTP/1.1`r`nHost: 127.0.0.1:$Port`r`nAccept: application/json`r`nConnection: close`r`n`r`n")
        $writer.Flush()
        Start-Sleep -Milliseconds 500
        $buf = New-Object char[] 65536
        $len = $reader.Read($buf, 0, 65536)
        $tcp.Close()
        if ($len -le 0) { return "" }
        $resp = -join $buf[0..($len-1)]
        # Split headers from body
        $idx = $resp.IndexOf("`r`n`r`n")
        if ($idx -ge 0) {
            $headers = $resp.Substring(0, $idx)
            $body = $resp.Substring($idx + 4)
            return @{ Headers = $headers; Body = $body; StatusLine = ($headers -split "`r`n")[0] }
        }
        return @{ Headers = ""; Body = $resp; StatusLine = "" }
    } catch {
        return @{ Headers = ""; Body = ""; StatusLine = "ERROR: $_" }
    }
}

function Api-Post {
    param([int]$Port, [string]$Path, [string]$Body)
    try {
        $tcp = New-Object System.Net.Sockets.TcpClient
        $tcp.Connect("127.0.0.1", $Port)
        $stream = $tcp.GetStream()
        $writer = New-Object System.IO.StreamWriter($stream)
        $reader = New-Object System.IO.StreamReader($stream)
        $bodyBytes = [System.Text.Encoding]::UTF8.GetBytes($Body)
        $request = "POST $Path HTTP/1.1`r`nHost: 127.0.0.1:$Port`r`nContent-Type: application/json`r`nContent-Length: $($bodyBytes.Length)`r`nAccept: application/json`r`nConnection: close`r`n`r`n$Body"
        $writer.Write($request)
        $writer.Flush()
        Start-Sleep -Milliseconds 500
        $buf = New-Object char[] 65536
        $len = $reader.Read($buf, 0, 65536)
        $tcp.Close()
        if ($len -le 0) { return "" }
        $resp = -join $buf[0..($len-1)]
        $idx = $resp.IndexOf("`r`n`r`n")
        if ($idx -ge 0) {
            $headers = $resp.Substring(0, $idx)
            $body = $resp.Substring($idx + 4)
            return @{ Headers = $headers; Body = $body; StatusLine = ($headers -split "`r`n")[0] }
        }
        return @{ Headers = ""; Body = $resp; StatusLine = "" }
    } catch {
        return @{ Headers = ""; Body = ""; StatusLine = "ERROR: $_" }
    }
}

$p = [int]$port1

# GET /health
$r = Api-Get -Port $p -Path "/health"
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] GET /health: $($r.StatusLine)"; exit 1 }
$h = $r.Body | ConvertFrom-Json
if ($h.status -ne "ok") { Write-Host "[FAIL] /health status not ok"; exit 1 }
Write-Host "  [PASS] GET /health"

# GET /node
$r = Api-Get -Port $p -Path "/node"
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] GET /node: $($r.StatusLine)"; exit 1 }
Write-Host "  [PASS] GET /node"

# GET /search
$r = Api-Get -Port $p -Path "/search?q=test&limit=5"
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] GET /search: $($r.StatusLine)"; exit 1 }
Write-Host "  [PASS] GET /search"

# GET /records
$r = Api-Get -Port $p -Path "/records?limit=10"
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] GET /records: $($r.StatusLine)"; exit 1 }
Write-Host "  [PASS] GET /records"

# GET /schema
$r = Api-Get -Port $p -Path "/schema"
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] GET /schema: $($r.StatusLine)"; exit 1 }
Write-Host "  [PASS] GET /schema"

# GET /schema/wiki/article
$r = Api-Get -Port $p -Path "/schema/wiki/article"
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] GET /schema/wiki/article: $($r.StatusLine)"; exit 1 }
Write-Host "  [PASS] GET /schema/wiki/article"

# GET /peers
$r = Api-Get -Port $p -Path "/peers"
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] GET /peers: $($r.StatusLine)"; exit 1 }
Write-Host "  [PASS] GET /peers"

# POST /peers/add
$r = Api-Post -Port $p -Path "/peers/add" -Body '{"addr":"1.2.3.4:7744"}'
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] POST /peers/add: $($r.StatusLine)"; exit 1 }
Write-Host "  [PASS] POST /peers/add"

# GET /scraper
$r = Api-Get -Port $p -Path "/scraper"
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] GET /scraper: $($r.StatusLine)"; exit 1 }
Write-Host "  [PASS] GET /scraper"

# GET /storage
$r = Api-Get -Port $p -Path "/storage"
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] GET /storage: $($r.StatusLine)"; exit 1 }
Write-Host "  [PASS] GET /storage"

# POST /storage/vacuum
$r = Api-Post -Port $p -Path "/storage/vacuum" -Body '{}'
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] POST /storage/vacuum: $($r.StatusLine)"; exit 1 }
Write-Host "  [PASS] POST /storage/vacuum"

# GET /config
$r = Api-Get -Port $p -Path "/config"
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] GET /config: $($r.StatusLine)"; exit 1 }
Write-Host "  [PASS] GET /config"

# POST /config/set
$r = Api-Post -Port $p -Path "/config/set" -Body '{"key":"log.level","value":"debug"}'
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] POST /config/set: $($r.StatusLine)"; exit 1 }
Write-Host "  [PASS] POST /config/set"

# GET /identity
$r = Api-Get -Port $p -Path "/identity"
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] GET /identity: $($r.StatusLine)"; exit 1 }
Write-Host "  [PASS] GET /identity"

# GET /bootstrap
$r = Api-Get -Port $p -Path "/bootstrap"
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] GET /bootstrap: $($r.StatusLine)"; exit 1 }
Write-Host "  [PASS] GET /bootstrap"

# GET /openapi.json
$r = Api-Get -Port $p -Path "/openapi.json"
if ($r.StatusLine -notmatch "200") { Write-Host "[FAIL] GET /openapi.json: $($r.StatusLine)"; exit 1 }
$openapi = $r.Body | ConvertFrom-Json
if ($openapi.openapi -ne "3.1.0") { Write-Host "[FAIL] /openapi.json not 3.1.0"; exit 1 }
Write-Host "  [PASS] GET /openapi.json (OpenAPI 3.1.0)"

# GET /record/{id} — should 404 for nonexistent
$r = Api-Get -Port $p -Path "/record/nonexistent123"
if ($r.StatusLine -notmatch "404") { Write-Host "[FAIL] GET /record/nonexistent should 404, got: $($r.StatusLine)"; exit 1 }
Write-Host "  [PASS] GET /record/nonexistent → 404"

# --- Test 5: CLI/API JSON parity ---
Write-Host ""
Write-Host "[TEST 5] CLI/API JSON parity"

# Insert a record via API (since the node has the DB locked)
$recordJson = '{"id":"test-record-1","source_url":"https://example.com/test","source_hash":"abc123","schema":"generic/kv","tags":["test"],"body":"Hello world test record","created_at":1700000000,"expires_at":1800000000,"scrape_source":"url","refresh_policy":"once","sig":""}'

$insertResult = Api-Post -Port $p -Path "/record/insert" -Body $recordJson
if ($insertResult.StatusLine -notmatch "200") { Write-Host "[FAIL] record insert via API: $($insertResult.StatusLine)"; exit 1 }
Write-Host "  Record inserted via API"

# Get record via API
$apiResult = Api-Get -Port $p -Path "/record/test-record-1"
if ($apiResult.StatusLine -notmatch "200") { Write-Host "[FAIL] API get record: $($apiResult.StatusLine)"; exit 1 }
$apiRecord = $apiResult.Body | ConvertFrom-Json

# Get record via CLI with --output json
$cliOutput = & $Dsearch --data-dir $TestDir1 record get test-record-1 --output json 2>&1
$cliRecord = $cliOutput | ConvertFrom-Json

# Compare key fields
if ($apiRecord.id -ne $cliRecord.id) { Write-Host "[FAIL] record id mismatch: API=$($apiRecord.id) CLI=$($cliRecord.id)"; exit 1 }
if ($apiRecord.schema -ne $cliRecord.schema) { Write-Host "[FAIL] record schema mismatch"; exit 1 }
if ($apiRecord.body -ne $cliRecord.body) { Write-Host "[FAIL] record body mismatch"; exit 1 }
Write-Host "  [PASS] CLI/API JSON parity for record get"

# Search via API
$apiSearch = Api-Get -Port $p -Path "/search?q=hello&limit=10"
if ($apiSearch.StatusLine -notmatch "200") { Write-Host "[FAIL] API search: $($apiSearch.StatusLine)"; exit 1 }
$apiSearchResult = $apiSearch.Body | ConvertFrom-Json

# Search via CLI with --output json
$cliSearch = & $Dsearch --data-dir $TestDir1 search "hello" --limit 10 --output json 2>&1
$cliSearchResult = $cliSearch | ConvertFrom-Json

if ($apiSearchResult.count -ne $cliSearchResult.Count) { Write-Host "[FAIL] search count mismatch: API=$($apiSearchResult.count) CLI=$($cliSearchResult.Count)"; exit 1 }
Write-Host "  [PASS] CLI/API JSON parity for search"

# List records via API
$apiList = Api-Get -Port $p -Path "/records?limit=50"
if ($apiList.StatusLine -notmatch "200") { Write-Host "[FAIL] API list records: $($apiList.StatusLine)"; exit 1 }
$apiListResult = $apiList.Body | ConvertFrom-Json

# List records via CLI with --output json
$cliList = & $Dsearch --data-dir $TestDir1 record list --limit 50 --output json 2>&1
$cliListResult = $cliList | ConvertFrom-Json

if ($apiListResult.count -ne $cliListResult.Count) { Write-Host "[FAIL] record list count mismatch: API=$($apiListResult.count) CLI=$($cliListResult.Count)"; exit 1 }
Write-Host "  [PASS] CLI/API JSON parity for record list"

# --- Test 6: Response headers ---
Write-Host ""
Write-Host "[TEST 6] Response headers (X-Node-Id, X-Protocol-Version)"
$r = Api-Get -Port $p -Path "/health"
if ($r.Headers -notmatch "x-node-id") { Write-Host "[FAIL] Missing X-Node-Id header"; exit 1 }
if ($r.Headers -notmatch "x-protocol-version") { Write-Host "[FAIL] Missing X-Protocol-Version header"; exit 1 }
Write-Host "[PASS] Response headers present"

# --- Test 7: Gateway key management ---
Write-Host ""
Write-Host "[TEST 7] Gateway key management"

$keyCreate = & $Dsearch --data-dir $TestDir1 gateway key-create --nickname test-key 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { Write-Host "[FAIL] gateway key create: $keyCreate"; exit 1 }
if ($keyCreate -notmatch "test-key") { Write-Host "[FAIL] key create output missing nickname. Output: $keyCreate"; exit 1 }
Write-Host "  [PASS] gateway key create"

$keyList = & $Dsearch --data-dir $TestDir1 gateway key-list 2>&1 | Out-String
if ($keyList -notmatch "test-key") { Write-Host "[FAIL] key list missing test-key. Output: $keyList"; exit 1 }
Write-Host "  [PASS] gateway key list"

$keyRevoke = & $Dsearch --data-dir $TestDir1 gateway key-revoke test-key 2>&1 | Out-String
if ($LASTEXITCODE -ne 0) { Write-Host "[FAIL] gateway key revoke: $keyRevoke"; exit 1 }
Write-Host "  [PASS] gateway key revoke"

# Verify key is gone
$keyList2 = & $Dsearch --data-dir $TestDir1 gateway key-list 2>&1 | Out-String
if ($keyList2 -match "test-key") { Write-Host "[FAIL] revoked key still in list"; exit 1 }
Write-Host "  [PASS] revoked key removed from list"

# --- Test 8: Node status via CLI ---
Write-Host ""
Write-Host "[TEST 8] Node status via CLI (reads from API)"
$statusOutput = & $Dsearch --data-dir $TestDir1 node status 2>&1
if ($statusOutput -match "not running") { Write-Host "[FAIL] node status says not running"; exit 1 }
Write-Host "[PASS] Node status reachable via CLI"

# --- Cleanup ---
Write-Host ""
Cleanup-Test

# --- Summary ---
Write-Host ""
Write-Host "========================================="
Write-Host "  Phase 7 Exit Test: ALL PASSED"
Write-Host "========================================="
exit 0
}
catch {
    Write-Host ""
    Write-Host "[ERROR] Unhandled exception: $_"
    Cleanup-Test
    exit 1
}
finally {
    # Ensure cleanup runs even on unexpected termination
    Get-Process -Name "dsearch" -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}
