# Encodes a 1-second test clip in every container x video codec x audio codec
# combination and reports which audio codecs work for each container + video codec.
# Used to build the tables in src/formats.rs.
param([string]$Ffmpeg = "ffmpeg", [string]$Ffprobe = "ffprobe")

$tmp = Join-Path $env:TEMP "cheapedit-combos"
New-Item -ItemType Directory -Force $tmp | Out-Null

$containers = [ordered]@{ mp4 = "mp4"; mkv = "matroska"; webm = "webm"; mov = "mov" }
$video = [ordered]@{
    "h264/libx264"     = @("-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p")
    "h265/libx265"     = @("-c:v", "libx265", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-x265-params", "log-level=error")
    "av1/libaom-av1"   = @("-c:v", "libaom-av1", "-cpu-used", "8", "-row-mt", "1", "-pix_fmt", "yuv420p")
    "vp9/libvpx-vp9"   = @("-c:v", "libvpx-vp9", "-deadline", "realtime", "-cpu-used", "8", "-pix_fmt", "yuv420p")
    "prores/prores_ks" = @("-c:v", "prores_ks", "-profile:v", "2", "-pix_fmt", "yuv422p10le")
    "h264/h264_nvenc"  = @("-c:v", "h264_nvenc", "-pix_fmt", "yuv420p")
    "h265/hevc_nvenc"  = @("-c:v", "hevc_nvenc", "-pix_fmt", "yuv420p")
    "av1/av1_nvenc"    = @("-c:v", "av1_nvenc", "-pix_fmt", "yuv420p")
}
$audio = [ordered]@{
    aac    = @("-c:a", "aac", "-b:a", "128k")
    mp3    = @("-c:a", "libmp3lame", "-b:a", "128k")
    opus   = @("-c:a", "libopus", "-b:a", "96k")
    flac   = @("-c:a", "flac")
    vorbis = @("-c:a", "libvorbis", "-b:a", "128k")
    pcm    = @("-c:a", "pcm_s16le")
}

foreach ($c in $containers.Keys) {
    foreach ($v in $video.Keys) {
        $okList = @()
        foreach ($a in $audio.Keys) {
            $out = Join-Path $tmp "t.$c"
            Remove-Item $out -ErrorAction SilentlyContinue
            $ffArgs = @("-hide_banner", "-loglevel", "error", "-y",
                "-f", "lavfi", "-i", "testsrc2=s=320x240:r=30:d=1",
                "-f", "lavfi", "-i", "sine=f=440:d=1:sample_rate=48000") + $video[$v] + $audio[$a] + @("-f", $containers[$c], $out)
            & $Ffmpeg @ffArgs 2>&1 | Out-Null
            if ($LASTEXITCODE -eq 0 -and (Test-Path $out)) {
                $streams = (& $Ffprobe -v error -show_entries stream=codec_type -of csv=p=0 $out) -join "+"
                if ($streams -eq "video+audio") { $okList += $a }
            }
        }
        "{0,-5} {1,-18} ok audio: {2}" -f $c, $v, ($(if ($okList) { $okList -join ", " } else { "(none - video codec not allowed)" }))
    }
}
$out = Join-Path $tmp "t.mp3"
& $Ffmpeg -hide_banner -loglevel error -y -f lavfi -i "sine=f=440:d=1" -c:a libmp3lame -b:a 192k $out 2>&1 | Out-Null
"mp3   (audio only)       ok: $($LASTEXITCODE -eq 0)"
