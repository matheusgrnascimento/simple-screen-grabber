<#
.SYNOPSIS
    Builds the Simple Screen Grabber MSI installer and the portable package.

.DESCRIPTION
    1. Builds the release executable (the C runtime is linked statically by
       .cargo\config.toml, so it needs no Visual C++ Redistributable).
    2. Stages the executable, the icon and the license.
    3. Builds dist\SimpleScreenGrabber-<version>-x64.msi with the WiX Toolset.
    4. Builds dist\SimpleScreenGrabber-<version>-portable.zip: the executable,
       the license and the "portable" marker, which keeps the settings next
       to the executable.

    Requires WiX Toolset 5 with the UI and Util extensions:
        dotnet tool install --global wix --version 5.0.2
        wix extension add -g WixToolset.UI.wixext/5.0.2
        wix extension add -g WixToolset.Util.wixext/5.0.2
#>
[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$stage = Join-Path $root "target\installer-stage"
$portable = Join-Path $root "target\portable-stage"
$dist = Join-Path $root "dist"
$msi = Join-Path $dist "SimpleScreenGrabber-$version-x64.msi"
$zip = Join-Path $dist "SimpleScreenGrabber-$version-portable.zip"

Write-Host "==> Building release $version" -ForegroundColor Cyan
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed." }

Write-Host "==> Staging files" -ForegroundColor Cyan
foreach ($dir in $stage, $portable) {
    if (Test-Path $dir) { Remove-Item $dir -Recurse -Force }
    New-Item -ItemType Directory -Path $dir -Force | Out-Null
}
Copy-Item "target\release\simple-screen-grabber.exe" $stage
Copy-Item "assets\icon.ico" $stage
Copy-Item "LICENSE" "$stage\LICENSE.txt"

# License page of the installer (RTF).
$escape = { param($s) $s.Replace('\', '\\').Replace('{', '\{').Replace('}', '\}') }
$body = (Get-Content LICENSE | ForEach-Object { (& $escape $_) + "\par" }) -join "`n"
"{\rtf1\ansi\deff0{\fonttbl{\f0 Segoe UI;}}\fs18`n$body`n}" | Set-Content "$stage\license.rtf" -Encoding ascii

Write-Host "==> Building MSI" -ForegroundColor Cyan
New-Item -ItemType Directory -Path $dist -Force | Out-Null
wix build "installer\Package.wxs" -arch x64 -ext WixToolset.UI.wixext -ext WixToolset.Util.wixext `
    -d "Version=$version" -d "StageDir=$stage" -o $msi
if ($LASTEXITCODE -ne 0) { throw "wix build failed." }

Write-Host "==> Building portable package" -ForegroundColor Cyan
Copy-Item "target\release\simple-screen-grabber.exe" $portable
Copy-Item "LICENSE" "$portable\LICENSE.txt"
New-Item -ItemType File -Path "$portable\portable" | Out-Null
if (Test-Path $zip) { Remove-Item $zip }
Compress-Archive -Path "$portable\*" -DestinationPath $zip

foreach ($file in $msi, $zip) {
    Write-Host "==> $file ($([math]::Round((Get-Item $file).Length / 1KB)) KB)" -ForegroundColor Green
}
