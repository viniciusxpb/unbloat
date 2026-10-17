[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
# Generated notices stay out of Git, but must always ship with the binaries.
& (Join-Path $PSScriptRoot 'collect-licenses.ps1')
$destination = Join-Path $root 'dist\Unbloat'
New-Item -Path $destination -ItemType Directory -Force | Out-Null
foreach ($pair in @(@('target\release\unbloat.exe','unbloat.exe'), @('target\release\unbloat_shell.dll','unbloat-shell.dll'), @('config.toml','config.toml'), @('LICENSE','LICENSE'), @('README.md','README.md'), @('VALIDATION.md','VALIDATION.md'), @('scripts\install.ps1','install.ps1'), @('scripts\uninstall.ps1','uninstall.ps1'))) {
    Copy-Item -LiteralPath (Join-Path $root $pair[0]) -Destination (Join-Path $destination $pair[1]) -Force
}
Copy-Item -LiteralPath (Join-Path $root 'licenses') -Destination $destination -Recurse -Force
Compress-Archive -Path (Join-Path $destination '*') -DestinationPath (Join-Path $root 'dist\Unbloat-0.2.0-windows-x64.zip') -Force
Write-Host "Distribuição criada em $destination"
