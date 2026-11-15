<#
 Builds Quadra on Windows 10/11 and produces the NSIS installer.
 Run from the repo root in PowerShell:   powershell -ExecutionPolicy Bypass -File tools\build-windows.ps1
 Needs: winget (ships with Windows 10/11), internet access. WebView2 is preinstalled on Win10/11.
 Optional: -SingBoxRepo <git url> to clone the sing-box fork next to this repo when it is missing.
#>
param([string]$SingBoxRepo = "")
$ErrorActionPreference = "Stop"
$repo = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location $repo

function Have($cmd) { [bool](Get-Command $cmd -ErrorAction SilentlyContinue) }
function Refresh-Path {
  $env:Path = [Environment]::GetEnvironmentVariable("Path","Machine") + ";" + [Environment]::GetEnvironmentVariable("Path","User") + ";$env:USERPROFILE\.cargo\bin"
}
function Winget-Install($id) {
  winget install --id $id -e --silent --accept-package-agreements --accept-source-agreements
  Refresh-Path
}

# 1. Prerequisites (installed only when missing)
if (-not (Have winget)) { throw "winget not found: install 'App Installer' from the Microsoft Store" }
if (-not (Have node))   { Winget-Install "OpenJS.NodeJS.LTS" }          # Node 22 LTS or newer
if (-not (Have go))     { Winget-Install "GoLang.Go" }
if (-not (Have git))    { Winget-Install "Git.Git" }
if (-not (Have rustup)) { Winget-Install "Rustlang.Rustup" }

# MSVC C++ build tools: look for the VC tools component with vswhere.
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$hasVc = (Test-Path $vswhere) -and (& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -latest)
if (-not $hasVc) {
  Write-Host "Installing Visual Studio Build Tools (C++ workload). This is large and may ask for elevation."
  winget install --id Microsoft.VisualStudio.2022.BuildTools -e --accept-package-agreements --accept-source-agreements `
    --override "--quiet --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
  Write-Host "If that failed, install 'Build Tools for Visual Studio 2022' manually and tick 'Desktop development with C++', then re-run."
}
Refresh-Path
rustup default stable-x86_64-pc-windows-msvc
rustup target add x86_64-pc-windows-msvc

# 2. sing-box core, built from the fork that lives next to this repo
$sb = Join-Path $repo "..\sing-box-xhttp"
if (-not (Test-Path $sb) -and $SingBoxRepo) { git clone $SingBoxRepo $sb }
# fetch-singbox.mjs builds the core and downloads the rule-sets; without ..\sing-box-xhttp it
# clones the repository named in tools\singbox.source
node tools/fetch-singbox.mjs
if ($LASTEXITCODE -ne 0) { throw "sing-box build failed" }

# 3. Xray, frontend deps, app + installer
node tools/fetch-xray.mjs
npm ci
npm run tauri build
if ($LASTEXITCODE -ne 0) { throw "tauri build failed" }

$setup = Get-ChildItem "$repo\src-tauri\target\release\bundle\nsis\*-setup.exe" | Select-Object -First 1
Write-Host "`nInstaller: $($setup.FullName)"
Write-Host "App exe:   $repo\src-tauri\target\release\quadra.exe"
