# Local-only orchestration for tools/ui-real-stack.spec.js.
# Brings up the backend stack, starts the frontend dev server,
# runs Playwright, then leaves both running so failures can be inspected.
# Pass -Clean to tear down compose + dx serve after a successful run.

[CmdletBinding()]
param(
    [switch]$Clean
)

$ErrorActionPreference = "Stop"

function Wait-ForUrl {
    param([string]$Url, [int]$TimeoutSeconds = 60)
    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    while ((Get-Date) -lt $deadline) {
        try {
            $resp = Invoke-WebRequest -UseBasicParsing -Uri $Url -TimeoutSec 5
            if ($resp.StatusCode -lt 500) { return $true }
        } catch { Start-Sleep -Seconds 2 }
    }
    throw "timed out waiting for $Url"
}

function Get-EnvFromFile {
    param([string]$Path)
    if (-not (Test-Path $Path)) { return @{} }
    $result = @{}
    Get-Content $Path | Where-Object { $_ -match '^[A-Z_]+=' } | ForEach-Object {
        $kv = $_ -split '=', 2
        $result[$kv[0]] = $kv[1]
    }
    return $result
}

$envFile = Get-EnvFromFile -Path ".env"
foreach ($k in @("LOCAL_LOGIN_EMAIL","LOCAL_LOGIN_TEACHER_PASSWORD","LOCAL_LOGIN_STUDENT_EMAIL","LOCAL_LOGIN_STUDENT_PASSWORD")) {
    if ($envFile.ContainsKey($k)) { Set-Item "env:$k" $envFile[$k] }
}

Write-Host "Starting docker compose..."
docker compose up -d

Write-Host "Waiting for backend healthz..."
Wait-ForUrl -Url "http://localhost:8080/healthz" -TimeoutSeconds 60

Write-Host "Starting dx serve in background..."
$dxLog = Join-Path $PSScriptRoot "..\target\dx-serve.log"
$dxProcess = Start-Process -FilePath "dx" `
    -ArgumentList "serve","--platform","web","--port","3000" `
    -WorkingDirectory (Resolve-Path "crates/shell-web") `
    -PassThru `
    -RedirectStandardOutput $dxLog `
    -RedirectStandardError $dxLog

try {
    Write-Host "Waiting for dx serve at http://localhost:3000..."
    Wait-ForUrl -Url "http://localhost:3000" -TimeoutSeconds 180

    Write-Host "Running playwright spec..."
    & npx playwright test tools/ui-real-stack.spec.js
    $exit = $LASTEXITCODE
    if ($exit -ne 0) {
        Write-Host "playwright failed (exit $exit) - leaving stack running for inspection"
        exit $exit
    }
    Write-Host "playwright passed"
}
finally {
    if ($Clean) {
        Write-Host "Cleanup: stopping dx serve and docker compose..."
        if ($dxProcess -and -not $dxProcess.HasExited) {
            Stop-Process -Id $dxProcess.Id -Force
        }
        docker compose down
    }
}
