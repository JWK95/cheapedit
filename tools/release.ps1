# Publish a new cheapedit release to GitHub.
#
#   .\tools\release.ps1 -Version 0.2.0 -Notes "What changed, in plain English"
#
# Steps (stops at the first problem; nothing is published until all checks pass):
#   1. Checks: on branch main, no uncommitted changes, version is new.
#   2. Fetches the pinned FFmpeg and libmpv (tools\deps.toml), checking every
#      SHA-256, sets the version in Cargo.toml, runs the tests, builds the exe.
#   3. Builds the installer (with FFmpeg and libmpv in deps\) and its SHA-256
#      checksum file.
#   4. Commits "Release vX.Y.Z", tags it, pushes, and creates the GitHub release
#      with the installer + checksum attached, plus the exact FFmpeg and mpv
#      source archives (the GPL's source requirement). Installed copies of
#      cheapedit then offer the update the next time they start.
param(
    [Parameter(Mandatory = $true)] [string] $Version,
    [Parameter(Mandatory = $true)] [string] $Notes
)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

function Step($msg) { Write-Host "`n== $msg" -ForegroundColor Cyan }
function Fail($msg) { Write-Host "`nSTOPPED: $msg" -ForegroundColor Red; exit 1 }
# Tools like cargo and git print progress to stderr; Windows PowerShell 5.1 would
# treat that as an error, so judge success by the exit code instead.
function Run($exe, [string[]]$a) {
    $ErrorActionPreference = 'Continue'
    & $exe @a 2>&1 | ForEach-Object { "$_" }
    if ($LASTEXITCODE -ne 0) { Fail "$exe $($a -join ' ') failed (exit $LASTEXITCODE)" }
}

$Version = $Version.TrimStart('v', 'V')
if ($Version -notmatch '^\d+\.\d+\.\d+$') { Fail "Version must look like 1.2.3 (got '$Version')." }
$tag = "v$Version"

Step "Checking the repo"
if ((git branch --show-current) -ne 'main') { Fail "Switch to the main branch first." }
if (git status --porcelain) { Fail "There are uncommitted changes. Commit or stash them first." }
if (git tag --list $tag) { Fail "Tag $tag already exists." }
$cur = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if ([version]$Version -le [version]$cur -and (git tag --list "v$cur")) { Fail "$Version isn't newer than the last release ($cur)." }

$iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { Fail "Inno Setup 6 not found. Install it with: winget install JRSoftware.InnoSetup" }
Run gh @('auth', 'status')

Step "Fetching FFmpeg and libmpv"
& "$PSScriptRoot\fetch-deps.ps1"
if ($LASTEXITCODE -ne 0) { Fail "fetch-deps.ps1 failed." }
$sources = @(Select-String -Path tools\deps.toml -Pattern '^source_name = "(.+)"' | ForEach-Object { "target\deps-cache\$($_.Matches[0].Groups[1].Value)" })
foreach ($s in $sources) { if (-not (Test-Path $s)) { Fail "Source archive missing: $s" } }
$included = (Get-Content target\deps\VERSIONS.txt | Where-Object { $_ }) -join "`n- "
$Notes = "$Notes`n`nIncludes:`n- $included`n`nThe exact source code for these is attached below, as the GPL requires."

Step "Setting version $Version"
$toml = Get-Content Cargo.toml -Raw
$toml = [regex]::Replace($toml, '(?m)^version = ".*?"', "version = `"$Version`"", 1)
[IO.File]::WriteAllText("$PWD\Cargo.toml", $toml)

Step "Running tests"
Run cargo @('test', '--quiet')

Step "Building the release exe"
Run cargo @('build', '--release')

Step "Building the installer"
Run $iscc @("/DAppVersion=$Version", '/Q', 'installer\cheapedit.iss')
$setup = "installer\Output\cheapedit-Setup-$Version.exe"
if (-not (Test-Path $setup)) { Fail "Installer not found at $setup" }
$hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLower()
$shaFile = "$setup.sha256"
[IO.File]::WriteAllText("$PWD\$shaFile", "$hash  cheapedit-Setup-$Version.exe`n")
Write-Host "SHA-256 $hash"

Step "Committing, tagging and pushing"
Run git @('add', 'Cargo.toml', 'Cargo.lock')
git diff --cached --quiet
if ($LASTEXITCODE -ne 0) { Run git @('commit', '-m', "Release $tag") }
Run git @('tag', '-a', $tag, '-m', "cheapedit $Version")
Run git @('push', 'origin', 'main')
Run git @('push', 'origin', $tag)

Step "Creating the GitHub release"
$notesFile = New-TemporaryFile
[IO.File]::WriteAllText($notesFile.FullName, $Notes)
Run gh (@('release', 'create', $tag, $setup, $shaFile) + $sources + @('--title', "cheapedit $Version", '--notes-file', $notesFile.FullName))
Remove-Item $notesFile

Write-Host "`nDone: cheapedit $Version is released." -ForegroundColor Green
