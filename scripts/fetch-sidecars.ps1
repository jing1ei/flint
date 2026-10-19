param([switch]$Check, [switch]$Force)
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ($env:OS -ne "Windows_NT") { throw "Run this script on Windows x64." }
$root = Split-Path $PSScriptRoot -Parent
$bin = Join-Path $root "src-tauri/binaries"
$triple = "x86_64-pc-windows-msvc"
$version = "9.0.1"
$sourceSha256 = "cf38e0e28c7e5605942c4a77755349b0145804a397af37eb1fb4c77cb237f635"
# The publisher rotates older packages off gyan.dev; its versioned GitHub
# release keeps this exact archive available with the same pinned checksum.
$url = "https://github.com/GyanD/codexffmpeg/releases/download/$version/ffmpeg-$version-full_build.7z"
$sha256 = "4b9c814cb07a1f90d05b768ef4eb2abbf89af94bbb924df5b7dbd6e64e1e2b96"
$cache = Join-Path $root "target/windows-dependencies"
$licenseDir = Join-Path $root "src-tauri/licenses"
$manifest = Join-Path $cache "sidecars.json"

function Get-Archive([string]$Uri, [string]$Destination) {
    # Windows PowerShell 5's progress rendering can stall large downloads in CI.
    # curl.exe has bounded retries and a low-speed timeout; checksum checks below
    # still reject incomplete or modified archives before installation.
    Write-Host "Downloading $Uri"
    $curl = (Get-Command curl.exe -ErrorAction Stop).Source
    $arguments = @('--fail', '--location', '--show-error', '--silent', '--retry', '2',
        '--retry-delay', '2', '--retry-max-time', '240', '--connect-timeout', '20',
        '--max-time', '120', '--speed-limit', '1024', '--speed-time', '30',
        '--output', "`"$Destination`"", "`"$Uri`"")
    $process = Start-Process -FilePath $curl -ArgumentList $arguments -NoNewWindow -PassThru
    try {
        if (!$process.WaitForExit(300000)) {
            $process.Kill()
            $process.WaitForExit()
            throw "Download exceeded five minutes: $Uri"
        }
        if ($process.ExitCode -ne 0) { throw "Download failed: $Uri (curl exit $($process.ExitCode))." }
    } finally { $process.Dispose() }
    if (!(Test-Path $Destination) -or (Get-Item $Destination).Length -eq 0) {
        throw "Downloaded archive is empty: $Uri"
    }
    Write-Host "Downloaded $((Get-Item $Destination).Length) bytes. Verifying archive next."
}

function Test-Sidecars {
    if (!(Test-Path $manifest)) { return $false }
    $record = Get-Content $manifest -Raw | ConvertFrom-Json
    if ($record.archive_sha256 -ne $sha256) { return $false }
    foreach ($name in @("ffmpeg", "ffprobe")) {
        $path = Join-Path $bin "$name-$triple.exe"
        if (!(Test-Path $path)) { return $false }
        if ((Get-FileHash $path -Algorithm SHA256).Hash -ne $record.$name) { return $false }
        $banner = & $path -version
        if ($LASTEXITCODE -ne 0 -or $banner[0] -notmatch "^$name version $([regex]::Escape($version))") { return $false }
    }
    $source = Join-Path $cache "ffmpeg-$version-source-for-Windows-x64.tar.xz"
    return (Test-Path (Join-Path $licenseDir "WINDOWS-FFMPEG-BUILD.txt")) -and
        (Test-Path $source) -and ((Get-FileHash $source -Algorithm SHA256).Hash -eq $sourceSha256)
}

if (!$Force -and (Test-Sidecars)) {
    Write-Host "Verified Windows x64 FFmpeg and ffprobe."
    exit 0
}
if ($Check) { throw "Windows sidecars are missing or unverified. Run scripts/fetch-sidecars.ps1." }
$seven = Get-Command 7z.exe -ErrorAction SilentlyContinue
if (!$seven) {
    $known = Join-Path $env:ProgramFiles "7-Zip/7z.exe"
    if (Test-Path $known) { $seven = Get-Item $known }
}
if (!$seven) { throw "Install 7-Zip from https://www.7-zip.org, then run this script again." }
$sevenPath = if ($seven -is [System.Management.Automation.ApplicationInfo]) { $seven.Source } else { $seven.FullName }
New-Item -ItemType Directory -Force -Path $bin, $cache | Out-Null
$temp = Join-Path $cache ([guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $temp | Out-Null
try {
    $archive = Join-Path $temp "ffmpeg.7z"
    Get-Archive $url $archive
    if ((Get-FileHash $archive -Algorithm SHA256).Hash -ne $sha256) {
        throw "FFmpeg archive checksum mismatch. No sidecars were replaced."
    }
    & $sevenPath x $archive "-o$temp/unpacked" -y | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Could not extract FFmpeg." }
    $record = @{ archive_url = $url; archive_sha256 = $sha256 }
    $evidence = @("Windows x64 bundled FFmpeg $version", "Publisher: https://www.gyan.dev/ffmpeg/builds/",
        "Archive: $url", "Archive SHA256: $sha256", "License: GPL-3.0-or-later (see GPL-3.0.txt)",
        "Upstream source: https://ffmpeg.org/releases/ffmpeg-$version.tar.xz",
        "Publisher configuration and dependency versions: WINDOWS-FFMPEG-README.txt",
        "The upstream archive is not a claim of byte-for-byte reproducibility or all dependency sources.", "")
    foreach ($name in @("ffmpeg", "ffprobe")) {
        $files = @(Get-ChildItem "$temp/unpacked" -Recurse -Filter "$name.exe")
        if ($files.Count -ne 1) { throw "Expected exactly one $name.exe." }
        $banner = & $files[0].FullName -version
        if ($LASTEXITCODE -ne 0 -or $banner[0] -notmatch "^$name version $([regex]::Escape($version))") {
            throw "$name is not the expected runnable build."
        }
        if (($banner -join "`n") -match "--enable-nonfree") { throw "Nonfree FFmpeg cannot be redistributed." }
        Copy-Item $files[0].FullName (Join-Path $bin "$name-$triple.exe") -Force
        $record[$name] = (Get-FileHash $files[0].FullName -Algorithm SHA256).Hash
        $evidence += "$name SHA256: $($record[$name])"
        $evidence += $banner
    }
    $readme = @(Get-ChildItem "$temp/unpacked" -Recurse -Filter "README.txt")
    if ($readme.Count -ne 1) { throw "Missing publisher build/license evidence." }
    Copy-Item $readme[0].FullName (Join-Path $licenseDir "WINDOWS-FFMPEG-README.txt") -Force
    $source = Join-Path $cache "ffmpeg-$version-source-for-Windows-x64.tar.xz"
    Get-Archive "https://ffmpeg.org/releases/ffmpeg-$version.tar.xz" "$source.tmp"
    # Verify the exact upstream source archive without invoking a PATH-dependent
    # tar executable (GNU tar can treat a Windows drive prefix as a remote host).
    if ((Get-FileHash "$source.tmp" -Algorithm SHA256).Hash -ne $sourceSha256) {
        throw "FFmpeg source checksum mismatch."
    }
    Move-Item "$source.tmp" $source -Force
    $evidence += "Upstream source SHA256: $((Get-FileHash $source -Algorithm SHA256).Hash)"
    $evidence | Set-Content (Join-Path $licenseDir "WINDOWS-FFMPEG-BUILD.txt") -Encoding UTF8
    $record | ConvertTo-Json | Set-Content $manifest -Encoding UTF8
    Write-Host "Windows x64 sidecars and license evidence ready."
} finally {
    Remove-Item $temp -Recurse -Force
}
