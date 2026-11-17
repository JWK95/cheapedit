# Build and run a development copy of cheapedit — no installing.
#
#   .\tools\dev.ps1                  build + run
#   .\tools\dev.ps1 "D:\clip.mp4"    build + run with a video opened
#
# The dev copy lives at target\debug\cheapedit.exe. It:
#   - shows a DEV badge and "[DEV]" in the window title
#   - keeps its own settings and logs in %APPDATA%\cheapedit-dev
#   - never checks for updates
# so it never disturbs the installed cheapedit.
param([Parameter(ValueFromRemainingArguments = $true)] [string[]] $AppArgs)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
# The bundled FFmpeg and libmpv live in target\deps (made once by fetch-deps.ps1).
if (-not (Test-Path 'target\deps\ffmpeg.exe')) {
    & "$PSScriptRoot\fetch-deps.ps1"
    if ($LASTEXITCODE -ne 0) { exit 1 }
}
cargo run -- @AppArgs
