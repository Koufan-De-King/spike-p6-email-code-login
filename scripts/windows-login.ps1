# One recorded desktop login on Windows: opens a console running another-login, which opens the
# default browser. Output is also saved to <tools>\logs\cli-<platform>.log.
#
#   powershell -ExecutionPolicy Bypass -File scripts\windows-login.ps1 -Platform win11-edge
param(
    [Parameter(Mandatory)][string]$Platform,
    [string]$Tools = "$env:LOCALAPPDATA\p6-tools"
)
$repo = Split-Path -Parent $PSScriptRoot
$log = Join-Path $Tools "logs\cli-$Platform.log"
New-Item -ItemType Directory -Force (Split-Path $log) | Out-Null
# UTF-8 so the CLI's arrows and check marks render; stderr folded in as plain text.
$inner = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; Set-Location '$repo'; " +
         "cmd /c '.\target\release\another-login.exe --platform $Platform 2>&1' | Tee-Object -FilePath '$log'"
Start-Process powershell -ArgumentList '-NoExit', '-Command', $inner
Write-Host "log: $log"
