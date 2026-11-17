# Download and lay out the FFmpeg and libmpv builds cheapedit bundles.
#
#   .\tools\fetch-deps.ps1         download (once, into target\deps-cache), check
#                                  every SHA-256 against tools\deps.toml, and build
#                                  target\deps\ (what the installer ships)
#   .\tools\fetch-deps.ps1 -Pin    after changing a version in deps.toml: download
#                                  and write the new SHA-256 values into it
#
# Nothing here touches PATH or anything outside this repo's target folder.
param([switch] $Pin)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

function Step($msg) { Write-Host "`n== $msg" -ForegroundColor Cyan }
function Fail($msg) { Write-Host "`nSTOPPED: $msg" -ForegroundColor Red; exit 1 }

$tomlPath = 'tools\deps.toml'
$cache = 'target\deps-cache'
$out = 'target\deps'
New-Item -ItemType Directory -Force $cache | Out-Null

# deps.toml is simple: [section] headers and key = "value" lines.
function Read-Deps {
    $deps = [ordered]@{}
    $section = $null
    foreach ($line in Get-Content $tomlPath) {
        if ($line -match '^\s*\[(\w+)\]\s*$') { $section = $Matches[1]; $deps[$section] = @{} }
        elseif ($section -and $line -match '^\s*(\w+)\s*=\s*"(.*)"\s*$') { $deps[$section][$Matches[1]] = $Matches[2] }
    }
    $deps
}

function Set-Pin($section, $key, $hash) {
    $text = Get-Content $tomlPath -Raw
    # Replace the key's value inside this section only.
    $pattern = "(?s)(\[$section\].*?\n$key\s*=\s*"")[0-9a-f]*("")"
    $new = [regex]::Replace($text, $pattern, "`${1}$hash`${2}", 1)
    [IO.File]::WriteAllText("$PWD\$tomlPath", $new)
}

# Download a file into the cache (skipped if already there), then check its hash.
function Get-Pinned($section, $urlKey, $hashKey, $fileName) {
    $d = $deps[$section]
    $url = $d[$urlKey]
    if (-not $fileName) { $fileName = Split-Path $url -Leaf }
    $file = Join-Path $cache $fileName
    if (-not (Test-Path $file)) {
        Write-Host "Downloading $url"
        & curl.exe -L --fail --silent --show-error -o "$file.part" $url
        if ($LASTEXITCODE -ne 0) { Remove-Item "$file.part" -ErrorAction SilentlyContinue; Fail "Download failed: $url" }
        Move-Item "$file.part" $file -Force
    }
    $hash = (Get-FileHash $file -Algorithm SHA256).Hash.ToLower()
    if ($Pin) {
        Set-Pin $section $hashKey $hash
        Write-Host "Pinned $section.$hashKey = $hash"
    } elseif (-not $d[$hashKey]) {
        Fail "$section.$hashKey is empty in deps.toml. Run .\tools\fetch-deps.ps1 -Pin first."
    } elseif ($hash -ne $d[$hashKey]) {
        Remove-Item $file
        Fail "$fileName doesn't match its pinned SHA-256 (got $hash). The bad download was deleted."
    } else {
        Write-Host "OK  $fileName"
    }
    $file
}

# Windows' own tar (libarchive) unpacks .zip and .7z; 7-Zip is a fallback for older Windows.
function Expand($archive, $dest) {
    New-Item -ItemType Directory -Force $dest | Out-Null
    & "$env:SystemRoot\System32\tar.exe" -xf $archive -C $dest 2>$null
    if ($LASTEXITCODE -eq 0) { return }
    $7z = "$env:ProgramFiles\7-Zip\7z.exe"
    if (Test-Path $7z) {
        & $7z x -y "-o$dest" $archive | Out-Null
        if ($LASTEXITCODE -eq 0) { return }
    }
    Fail "Couldn't unpack $archive. Install 7-Zip (winget install 7zip.7zip) and try again."
}

$deps = Read-Deps

Step "Checking downloads"
$ffZip = Get-Pinned 'ffmpeg' 'url' 'sha256'
$ffSrc = Get-Pinned 'ffmpeg' 'source_url' 'source_sha256' $deps['ffmpeg']['source_name']
$mpv7z = Get-Pinned 'libmpv' 'url' 'sha256'
$mpvSrc = Get-Pinned 'libmpv' 'source_url' 'source_sha256' $deps['libmpv']['source_name']
if ($Pin) { $deps = Read-Deps; Write-Host "`nNew pins written to $tomlPath." }

Step "Unpacking"
$work = Join-Path $cache 'unpacked'
if (Test-Path $work) { Remove-Item $work -Recurse -Force }
Expand $ffZip "$work\ffmpeg"
Expand $mpv7z "$work\mpv"
# mpv's licence files come from its source archive.
New-Item -ItemType Directory -Force "$work\mpvsrc" | Out-Null
& "$env:SystemRoot\System32\tar.exe" -xzf $mpvSrc -C "$work\mpvsrc" --strip-components 1 '*/LICENSE.GPL' '*/LICENSE.LGPL' '*/Copyright' 2>$null

Step "Laying out $out"
if (Test-Path $out) { Remove-Item $out -Recurse -Force }
New-Item -ItemType Directory -Force "$out\licenses" | Out-Null
$ffBin = Get-ChildItem "$work\ffmpeg" -Recurse -Directory -Filter bin | Select-Object -First 1
if (-not $ffBin) { Fail "No bin folder in the FFmpeg archive." }
# ffmpeg.exe, ffprobe.exe and the shared DLLs they need (ffplay isn't used).
Get-ChildItem $ffBin.FullName -File | Where-Object { $_.Name -ne 'ffplay.exe' } | Copy-Item -Destination $out
$ffRoot = $ffBin.Parent.FullName
foreach ($f in 'LICENSE', 'README.txt') { if (Test-Path "$ffRoot\$f") { Copy-Item "$ffRoot\$f" "$out\licenses\FFmpeg-$f" } }
$dll = Get-ChildItem "$work\mpv" -Recurse -Filter 'libmpv-2.dll' | Select-Object -First 1
if (-not $dll) { Fail "No libmpv-2.dll in the mpv archive." }
Copy-Item $dll.FullName $out
foreach ($f in 'LICENSE.GPL', 'LICENSE.LGPL', 'Copyright') { if (Test-Path "$work\mpvsrc\$f") { Copy-Item "$work\mpvsrc\$f" "$out\licenses\mpv-$f" } }
foreach ($need in 'ffmpeg.exe', 'ffprobe.exe', 'libmpv-2.dll') { if (-not (Test-Path "$out\$need")) { Fail "$need is missing from $out" } }

$ff = $deps['ffmpeg']; $mpv = $deps['libmpv']
$versions = @"
FFmpeg $($ff['version']) ($($ff['build']))
libmpv $($mpv['version']) ($($mpv['build']))
"@
[IO.File]::WriteAllText("$PWD\$out\VERSIONS.txt", $versions)
$readme = @"
cheapedit bundles these programs. Both are free software under the GNU GPL.

$versions
FFmpeg: https://ffmpeg.org (licence: FFmpeg-LICENSE)
mpv:    https://mpv.io (licence: mpv-Copyright, mpv-LICENSE.GPL)

The exact source code for these builds is attached to every cheapedit
release on GitHub (https://github.com/JWK95/cheapedit/releases), as
$($ff['source_name']) and $($mpv['source_name']).
"@
[IO.File]::WriteAllText("$PWD\$out\licenses\README.txt", $readme)
Remove-Item $work -Recurse -Force

$size = (Get-ChildItem $out -Recurse -File | Measure-Object Length -Sum).Sum / 1MB
Write-Host ("`nDone: {0} ({1:N0} MB)" -f $out, $size) -ForegroundColor Green
