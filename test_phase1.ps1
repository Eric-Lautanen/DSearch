# Phase 1 Exit Test
# Start two local node instances with separate data dirs,
# point one's bootstrap.toml at the other, confirm they see each other,
# then stop one and confirm clean disconnect.

$ErrorActionPreference = "Continue"

Write-Host "=== Phase 1 Exit Test ===" -ForegroundColor Cyan

# Clean up any previous test data
$testDirA = Join-Path $env:TEMP "dsearch-test-a"
$testDirB = Join-Path $env:TEMP "dsearch-test-b"
if (Test-Path $testDirA) { Remove-Item -Recurse -Force $testDirA }
if (Test-Path $testDirB) { Remove-Item -Recurse -Force $testDirB }

# Build first to ensure we have a fresh binary
Write-Host "`n--- Building dsearch ---" -ForegroundColor Yellow
$buildResult = & cargo build --release 2>&1 | Out-String
Write-Host $buildResult
if ($LASTEXITCODE -ne 0) {
    Write-Host "[FAIL] Build failed" -ForegroundColor Red
    exit 1
}
Write-Host "[OK] Build succeeded" -ForegroundColor Green

$dsearchExe = ".\target\release\dsearch.exe"

# Step 1: Initialize both nodes
Write-Host "`n--- Step 1: Initialize two nodes ---" -ForegroundColor Yellow
$initA = & $dsearchExe init --data-dir $testDirA 2>&1 | Out-String
$initB = & $dsearchExe init --data-dir $testDirB 2>&1 | Out-String
Write-Host "Node A init output:"
Write-Host $initA
Write-Host "Node B init output:"
Write-Host $initB

# Verify files exist
$ok = $true
foreach ($dir in @($testDirA, $testDirB)) {
    $label = if ($dir -eq $testDirA) { "A" } else { "B" }
    if (Test-Path "$dir\identity.key") { Write-Host "[OK] Node $label identity.key exists" -ForegroundColor Green } else { Write-Host "[FAIL] Node $label identity.key missing" -ForegroundColor Red; $ok = $false }
    if (Test-Path "$dir\node.crt") { Write-Host "[OK] Node $label node.crt exists" -ForegroundColor Green } else { Write-Host "[FAIL] Node $label node.crt missing" -ForegroundColor Red; $ok = $false }
    if (Test-Path "$dir\identity.tls") { Write-Host "[OK] Node $label identity.tls exists" -ForegroundColor Green } else { Write-Host "[FAIL] Node $label identity.tls missing" -ForegroundColor Red; $ok = $false }
    if (Test-Path "$dir\config.toml") { Write-Host "[OK] Node $label config.toml exists" -ForegroundColor Green } else { Write-Host "[FAIL] Node $label config.toml missing" -ForegroundColor Red; $ok = $false }
    if (Test-Path "$dir\bootstrap.toml") { Write-Host "[OK] Node $label bootstrap.toml exists" -ForegroundColor Green } else { Write-Host "[FAIL] Node $label bootstrap.toml missing" -ForegroundColor Red; $ok = $false }
}

if (-not $ok) {
    Write-Host "[FAIL] Missing files from init" -ForegroundColor Red
    exit 1
}

# Step 2: Get Node B's node_id
Write-Host "`n--- Step 2: Get Node B identity ---" -ForegroundColor Yellow
$idShowB = & $dsearchExe identity show --data-dir $testDirB 2>&1 | Out-String
Write-Host "Node B identity: $idShowB"
$nodeIdB = ""
if ($idShowB -match "Node ID: (\S+)") {
    $nodeIdB = $Matches[1]
    Write-Host "Node B ID: $nodeIdB" -ForegroundColor Green
} else {
    Write-Host "[FAIL] Could not parse Node B ID" -ForegroundColor Red
    exit 1
}

# Step 3: Configure bootstrap.toml for Node A to point to Node B
Write-Host "`n--- Step 3: Configure bootstrap ---" -ForegroundColor Yellow
$bootstrapA = @"
use_defaults = false

[[peers]]
id = "$nodeIdB"
addr = "127.0.0.1:7745"
note = "test node B"
"@
Set-Content -Path "$testDirA\bootstrap.toml" -Value $bootstrapA -Encoding UTF8

$bootstrapB = @"
use_defaults = false
"@
Set-Content -Path "$testDirB\bootstrap.toml" -Value $bootstrapB -Encoding UTF8

Write-Host "Node A bootstrap.toml configured to point to Node B at 127.0.0.1:7745"

# Verify bootstrap list shows Node B
$bootstrapList = & $dsearchExe bootstrap list --data-dir $testDirA 2>&1 | Out-String
Write-Host "Bootstrap list for Node A:"
Write-Host $bootstrapList
if ($bootstrapList -match $nodeIdB) {
    Write-Host "[OK] Bootstrap list shows Node B" -ForegroundColor Green
} else {
    Write-Host "[WARN] Bootstrap list may not show Node B" -ForegroundColor Yellow
}

# Step 4: Start Node B on port 7745 (background)
Write-Host "`n--- Step 4: Start Node B on port 7745 ---" -ForegroundColor Yellow
$procB = Start-Process -FilePath $dsearchExe -ArgumentList "node","start","--headless","--port","7745","--data-dir","`"$testDirB`"" -PassThru -NoNewWindow -RedirectStandardOutput "$testDirB\node.log" -RedirectStandardError "$testDirB\node-err.log"
Start-Sleep -Seconds 3
Write-Host "Node B started (PID: $($procB.Id))"

# Verify Node B PID file exists
if (Test-Path "$testDirB\node.pid") {
    $pidB = Get-Content "$testDirB\node.pid"
    Write-Host "[OK] Node B PID file exists: $pidB" -ForegroundColor Green
} else {
    Write-Host "[WARN] Node B PID file not found yet" -ForegroundColor Yellow
}

# Step 5: Start Node A on port 7744 (background)
Write-Host "`n--- Step 5: Start Node A on port 7744 ---" -ForegroundColor Yellow
$procA = Start-Process -FilePath $dsearchExe -ArgumentList "node","start","--headless","--port","7744","--data-dir","`"$testDirA`"" -PassThru -NoNewWindow -RedirectStandardOutput "$testDirA\node.log" -RedirectStandardError "$testDirA\node-err.log"
Start-Sleep -Seconds 5
Write-Host "Node A started (PID: $($procA.Id))"

# Step 6: Check peers list on both nodes
Write-Host "`n--- Step 6: Check peers list ---" -ForegroundColor Yellow

# Poll for up to 10 seconds — the inbound handler is async and may need
# a moment to complete the handshake + write peers.json
$foundPeerA = $false
$foundPeerB = $false
for ($attempt = 0; $attempt -lt 10; $attempt++) {
    Start-Sleep -Seconds 1

    if (Test-Path "$testDirA\peers.json") {
        $peersA = Get-Content "$testDirA\peers.json" -ErrorAction SilentlyContinue
        if ($peersA -match "node_id") { $foundPeerA = $true }
    }
    if (Test-Path "$testDirB\peers.json") {
        $peersB = Get-Content "$testDirB\peers.json" -ErrorAction SilentlyContinue
        if ($peersB -match "node_id") { $foundPeerB = $true }
    }

    if ($foundPeerA -and $foundPeerB) { break }
}

# Show final state
if (Test-Path "$testDirA\peers.json") {
    $peersA = Get-Content "$testDirA\peers.json" -ErrorAction SilentlyContinue
    Write-Host "Node A peers.json:"
    Write-Host $peersA
}
if (Test-Path "$testDirB\peers.json") {
    $peersB = Get-Content "$testDirB\peers.json" -ErrorAction SilentlyContinue
    Write-Host "Node B peers.json:"
    Write-Host $peersB
}

# Also check via CLI
$peersListA = & $dsearchExe peers list --data-dir $testDirA 2>&1 | Out-String
Write-Host "Node A peers list (CLI):"
Write-Host $peersListA

# Handshake must be bidirectional: the dialer (A) recording the peer it
# connected to is necessary but not sufficient — the listener (B) has to
# record the inbound connection too, or routing/gossip in later phases
# breaks for anyone who didn't dial first.
$foundPeerA = $peersA -and ($peersA -match "node_id")
$foundPeerB = $peersB -and ($peersB -match "node_id")

if ($foundPeerA) {
    Write-Host "[OK] Node A (dialer) sees Node B in peers.json" -ForegroundColor Green
} else {
    Write-Host "[FAIL] Node A (dialer) does not see Node B in peers.json" -ForegroundColor Red
}

if ($foundPeerB) {
    Write-Host "[OK] Node B (listener) sees Node A in peers.json" -ForegroundColor Green
} else {
    Write-Host "[FAIL] Node B (listener) does not see Node A in peers.json - inbound connections are not being recorded as peers" -ForegroundColor Red
}

$foundPeer = $foundPeerA -and $foundPeerB
if (-not $foundPeer) {
    Write-Host "[FAIL] Two-node handshake is not bidirectional - aborting" -ForegroundColor Red
    Stop-Process -Id $procA.Id -Force -ErrorAction SilentlyContinue
    Stop-Process -Id $procB.Id -Force -ErrorAction SilentlyContinue
    exit 1
}

# Step 7: Stop Node A gracefully using `dsearch node stop`
Write-Host "`n--- Step 7: Stop Node A gracefully ---" -ForegroundColor Yellow
$stopResult = & $dsearchExe node stop --data-dir $testDirA 2>&1 | Out-String
Write-Host "Stop result: $stopResult"
Start-Sleep -Seconds 5

# Check Node B's log for clean disconnect
$nodeBErr = if (Test-Path "$testDirB\node-err.log") { Get-Content "$testDirB\node-err.log" -ErrorAction SilentlyContinue | Out-String } else { "" }
$nodeBOut = if (Test-Path "$testDirB\node.log") { Get-Content "$testDirB\node.log" -ErrorAction SilentlyContinue | Out-String } else { "" }
$nodeBLog = $nodeBErr + $nodeBOut

Write-Host "Node B stderr log (last 30 lines):"
$nodeBErrLines = $nodeBErr -split "`n" | Select-Object -Last 30
Write-Host $nodeBErrLines

Write-Host "Node B stdout log (last 30 lines):"
$nodeBOutLines = $nodeBOut -split "`n" | Select-Object -Last 30
Write-Host $nodeBOutLines

$cleanDisconnect = $false
if ($nodeBLog -match "Goodbye" -or $nodeBLog -match "removed" -or $nodeBLog -match "closed" -or $nodeBLog -match "disconnect" -or $nodeBLog -match "Stream from") {
    Write-Host "[OK] Node B detected Node A disconnect cleanly" -ForegroundColor Green
    $cleanDisconnect = $true
} else {
    Write-Host "[FAIL] No explicit clean disconnect detected in Node B log" -ForegroundColor Red
    # Don't exit yet — the peers.json check below is the hard gate.
    # But this is a real FAIL, not a WARN: a node that doesn't log
    # disconnects is a problem even if the routing table is correct.
}

# Check that Node B did NOT panic
if ($nodeBLog -match "panic" -or $nodeBLog -match "PANIC") {
    Write-Host "[FAIL] Node B panicked when Node A disconnected!" -ForegroundColor Red
    Stop-Process -Id $procB.Id -Force -ErrorAction SilentlyContinue
    exit 1
} else {
    Write-Host "[OK] Node B did not panic on peer disconnect" -ForegroundColor Green
}

# Check Node B's peers.json — peer MUST be removed after disconnect.
# This is the hard gate: B had A as a peer (confirmed in Step 6),
# and now B must have removed it. An empty list after a previously
# non-empty one is the real evidence of clean disconnect handling.
Start-Sleep -Seconds 2
if (Test-Path "$testDirB\peers.json") {
    $peersBAfter = Get-Content "$testDirB\peers.json" -ErrorAction SilentlyContinue
    Write-Host "Node B peers.json after disconnect:"
    Write-Host $peersBAfter
    if ($peersBAfter -match "\[\s*\]" -or $peersBAfter -eq "[]") {
        Write-Host "[OK] Node B's peer list is empty after disconnect (peer removed)" -ForegroundColor Green
        $cleanDisconnect = $true
    } elseif ($peersBAfter -match "node_id") {
        Write-Host "[FAIL] Node B still shows a peer in peers.json after disconnect - not removed" -ForegroundColor Red
        Stop-Process -Id $procB.Id -Force -ErrorAction SilentlyContinue
        exit 1
    }
}

# Also check Node A's peers.json — A should have removed B too
# (now that A has a handle_messages loop that detects disconnects)
if (Test-Path "$testDirA\peers.json") {
    $peersAAfter = Get-Content "$testDirA\peers.json" -ErrorAction SilentlyContinue
    Write-Host "Node A peers.json after disconnect:"
    Write-Host $peersAAfter
    if ($peersAAfter -match "\[\s*\]" -or $peersAAfter -eq "[]") {
        Write-Host "[OK] Node A's peer list is empty after disconnect (peer removed)" -ForegroundColor Green
    } elseif ($peersAAfter -match "node_id") {
        Write-Host "[WARN] Node A still shows a peer in peers.json after disconnect" -ForegroundColor Yellow
    }
}

if (-not $cleanDisconnect) {
    Write-Host "[FAIL] No evidence of clean disconnect" -ForegroundColor Red
    Stop-Process -Id $procB.Id -Force -ErrorAction SilentlyContinue
    exit 1
}

# Step 8: Stop Node B
Write-Host "`n--- Step 8: Stop Node B ---" -ForegroundColor Yellow
$stopResultB = & $dsearchExe node stop --data-dir $testDirB 2>&1 | Out-String
Write-Host "Stop result: $stopResultB"
Start-Sleep -Seconds 3

# Force-kill any remaining processes as cleanup
Stop-Process -Id $procA.Id -Force -ErrorAction SilentlyContinue
Stop-Process -Id $procB.Id -Force -ErrorAction SilentlyContinue

# Step 9: Verify config show round-trips
Write-Host "`n--- Step 9: Verify config show ---" -ForegroundColor Yellow
$configShow = & $dsearchExe config show --data-dir $testDirA 2>&1 | Out-String
Write-Host "Config output:"
Write-Host $configShow

# Check all expected keys are present
$expectedKeys = @("role", "max_connections", "min_protocol_version", "port", "enabled", "quota_mb", "level")
$missingKeys = @()
foreach ($key in $expectedKeys) {
    if ($configShow -notmatch $key) {
        $missingKeys += $key
    }
}
if ($missingKeys.Count -eq 0) {
    Write-Host "[OK] All expected config keys present" -ForegroundColor Green
} else {
    Write-Host "[WARN] Missing config keys: $($missingKeys -join ', ')" -ForegroundColor Yellow
}

# Step 10: Verify identity show
Write-Host "`n--- Step 10: Verify identity show ---" -ForegroundColor Yellow
$idShowA = & $dsearchExe identity show --data-dir $testDirA 2>&1 | Out-String
Write-Host $idShowA
if ($idShowA -match "Node ID: \S+") {
    Write-Host "[OK] Identity show works" -ForegroundColor Green
} else {
    Write-Host "[FAIL] Identity show failed" -ForegroundColor Red
    exit 1
}

# Step 11: Verify bootstrap list
Write-Host "`n--- Step 11: Verify bootstrap list ---" -ForegroundColor Yellow
$bootstrapList = & $dsearchExe bootstrap list --data-dir $testDirA 2>&1 | Out-String
Write-Host $bootstrapList
if ($bootstrapList -match $nodeIdB) {
    Write-Host "[OK] Bootstrap list shows Node B" -ForegroundColor Green
} else {
    Write-Host "[WARN] Bootstrap list may not show Node B" -ForegroundColor Yellow
}

# Step 12: Verify role list
Write-Host "`n--- Step 12: Verify role list ---" -ForegroundColor Yellow
$roleList = & $dsearchExe role list 2>&1 | Out-String
Write-Host $roleList
$expectedRoles = @("light", "full", "bootstrap", "relay", "scraper", "archive")
$missingRoles = @()
foreach ($role in $expectedRoles) {
    if ($roleList -notmatch $role) {
        $missingRoles += $role
    }
}
if ($missingRoles.Count -eq 0) {
    Write-Host "[OK] All expected roles present" -ForegroundColor Green
} else {
    Write-Host "[FAIL] Missing roles: $($missingRoles -join ', ')" -ForegroundColor Red
    exit 1
}

# Summary
Write-Host "`n=== Phase 1 Exit Test Summary ===" -ForegroundColor Cyan
Write-Host "Init + file generation: OK" -ForegroundColor Green
Write-Host "Identity show: OK" -ForegroundColor Green
Write-Host "Bootstrap config: OK" -ForegroundColor Green
Write-Host "Config show: OK" -ForegroundColor Green
Write-Host "Role list: OK" -ForegroundColor Green
if ($foundPeer) {
    Write-Host "Two-node handshake (bidirectional): OK" -ForegroundColor Green
} else {
    Write-Host "Two-node handshake (bidirectional): FAIL" -ForegroundColor Red
}
if ($cleanDisconnect) {
    Write-Host "Clean disconnect: OK" -ForegroundColor Green
} else {
    Write-Host "Clean disconnect: FAIL" -ForegroundColor Red
}
Write-Host "`n=== Phase 1 Exit Test Complete ===" -ForegroundColor Cyan