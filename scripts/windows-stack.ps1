# Runs the P6 stack natively on Windows, without Docker: Redis, Mailpit and the idp.
#
#   powershell -ExecutionPolicy Bypass -File scripts\windows-stack.ps1          # start
#   powershell -ExecutionPolicy Bypass -File scripts\windows-stack.ps1 -Stop    # stop
#
# Expects Redis and Mailpit for Windows unzipped under $Tools (see README.md, "Windows without Docker").
# Same ports and settings as compose.yaml: idp :8080, Mailpit UI :8025, SMTP :1025, Redis :6379.
param(
    [string]$Tools = "$env:LOCALAPPDATA\p6-tools",
    [string]$Issuer = "http://127.0.0.1:8080",
    [ValidateSet('303', 'interstitial')][string]$CustomSchemeRedirect = '303',
    [switch]$Stop
)
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$logs = Join-Path $Tools 'logs'

if ($Stop) {
    Get-Process idp, mailpit, redis-server -ErrorAction SilentlyContinue | Stop-Process -Force
    Write-Host 'stopped'
    return
}

New-Item -ItemType Directory -Force $logs | Out-Null
$redisDir = (Get-ChildItem "$Tools\redis" -Directory | Select-Object -First 1).FullName

if (-not (Get-Process redis-server -ErrorAction SilentlyContinue)) {
    Start-Process -WindowStyle Hidden "$redisDir\redis-server.exe" -WorkingDirectory $redisDir `
        -ArgumentList '--port', '6379', '--bind', '127.0.0.1', '--save', '""', '--appendonly', 'no'
}
if (-not (Get-Process mailpit -ErrorAction SilentlyContinue)) {
    Start-Process -WindowStyle Hidden "$Tools\mailpit\mailpit.exe" -WorkingDirectory "$Tools\mailpit" `
        -ArgumentList '--listen', '127.0.0.1:8025', '--smtp', '127.0.0.1:1025', '--smtp-auth-accept-any', '--smtp-auth-allow-insecure'
}

Push-Location $repo
try {
    cargo build --release -p idp -p another-login
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
} finally { Pop-Location }

Get-Process idp -ErrorAction SilentlyContinue | Stop-Process -Force
$env:ISSUER = $Issuer
$env:REDIS_URL = 'redis://127.0.0.1:6379'
$env:SMTP_HOST = '127.0.0.1'
$env:SMTP_PORT = '1025'
# Same fixed pepper as compose.yaml, so codes survive an idp restart. Spike only.
$env:OTP_PEPPER = '5f1d0c9ab0e14a6f8c1e2d3b4a5f60718293a4b5c6d7e8f90a1b2c3d4e5f6071'
$env:CUSTOM_SCHEME_REDIRECT = $CustomSchemeRedirect
$env:REQUIRE_TXN_COOKIE = 'true'
$env:CLIENTS_FILE = "$repo\config\clients.json"
$env:WELL_KNOWN_DIR = "$repo\config\well-known"
$env:BIND = '127.0.0.1:8080'
$env:RUST_LOG = 'info'
Start-Process -WindowStyle Hidden "$repo\target\release\idp.exe" -WorkingDirectory $repo `
    -RedirectStandardOutput "$logs\idp.out.log" -RedirectStandardError "$logs\idp.err.log"

for ($i = 0; $i -lt 30; $i++) {
    try { if ((Invoke-WebRequest -UseBasicParsing "http://127.0.0.1:8080/healthz").StatusCode -eq 200) { break } } catch {}
    Start-Sleep -Milliseconds 500
}
Write-Host "idp      $Issuer   (timeline: $Issuer/spike, logs: $logs)"
Write-Host "mailpit  http://127.0.0.1:8025"
