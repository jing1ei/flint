param([switch]$SkipInstall, [switch]$SkipSidecars)
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
if ($env:OS -ne "Windows_NT") { throw "Build-Windows.ps1 requires Windows 10/11 x64." }
Set-Location $PSScriptRoot
foreach ($tool in @("node", "npm.cmd", "rustc", "cargo")) {
    if (!(Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "Missing $tool. Install Node.js 22 LTS and Rust MSVC, plus Visual Studio C++ Build Tools. See docs/WINDOWS.md."
    }
}
if ((& rustc -vV) -notcontains "host: x86_64-pc-windows-msvc") {
    throw "Use the x86_64-pc-windows-msvc Rust toolchain for this build."
}
function Run-Checked([string]$Program, [string[]]$Arguments) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { throw "$Program failed with exit code $LASTEXITCODE." }
}
if (!$SkipInstall) { Run-Checked "npm.cmd" @("ci") }
if (!$SkipSidecars) {
    & "$PSScriptRoot/scripts/fetch-sidecars.ps1"
    if ($LASTEXITCODE -ne 0) { throw "Sidecar preparation failed with exit code $LASTEXITCODE." }
}
Run-Checked "npm.cmd" @("test")
Run-Checked "npm.cmd" @("run", "build")
Run-Checked "cargo" @("check", "--workspace", "--locked")
Run-Checked "cargo" @("test", "-p", "convert-core", "--test", "windows_smoke", "--locked")
Run-Checked "cargo" @("test", "-p", "convert-core", "--test", "crop_batch", "--locked")
Run-Checked "cargo" @("test", "-p", "convert-core", "--test", "midi_audio", "--locked")
Run-Checked "cargo" @("test", "-p", "convert-core", "--test", "music_links", "--locked")
Run-Checked "npm.cmd" @("run", "build:windows")
Write-Host "Installer: $PSScriptRoot\target\release\bundle\nsis"
