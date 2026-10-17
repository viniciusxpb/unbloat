[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
Push-Location $root
try {
    $metadataJson = & cargo metadata --no-deps --locked --offline --format-version 1
    if ($LASTEXITCODE -ne 0) { throw 'cargo metadata falhou' }
    $metadata = $metadataJson | ConvertFrom-Json
    $package = @($metadata.packages | Where-Object { $_.name -eq 'unbloat' -and $_.id -in $metadata.workspace_members })
    if ($package.Count -ne 1) { throw 'Pacote unbloat ausente ou ambiguo no workspace.' }
    $version = $package[0].version
} finally { Pop-Location }
# Generated notices stay out of Git, but must always ship with the binaries.
& (Join-Path $PSScriptRoot 'collect-licenses.ps1')
$destination = Join-Path $root 'dist\Unbloat'
New-Item -Path $destination -ItemType Directory -Force | Out-Null
foreach ($pair in @(@('target\release\unbloat.exe','unbloat.exe'), @('target\release\unbloat_shell.dll','unbloat-shell.dll'), @('config.toml','config.toml'), @('LICENSE','LICENSE'), @('README.md','README.md'), @('VALIDATION.md','VALIDATION.md'), @('scripts\install.ps1','install.ps1'), @('scripts\uninstall.ps1','uninstall.ps1'))) {
    Copy-Item -LiteralPath (Join-Path $root $pair[0]) -Destination (Join-Path $destination $pair[1]) -Force
}
Copy-Item -LiteralPath (Join-Path $root 'licenses') -Destination $destination -Recurse -Force
$archive = Join-Path $root "dist\Unbloat-$version-windows-x64.zip"
Compress-Archive -Path (Join-Path $destination '*') -DestinationPath $archive -Force
Write-Host "Distribuição criada em $destination"
Write-Host "ZIP: $archive"
