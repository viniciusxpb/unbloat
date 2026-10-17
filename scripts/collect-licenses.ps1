$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'i18n.ps1')
$root = Split-Path $PSScriptRoot
Push-Location $root
try {
    $metadataJson = & cargo metadata --locked --offline --format-version 1
    if ($LASTEXITCODE -ne 0) { throw (Get-UnbloatText 'package.metadata_failed') }
    $metadata = $metadataJson | ConvertFrom-Json
    $destination = Join-Path $root 'licenses\source-notices'
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    $inventory = @('# Locked dependency inventory', '', 'Generated from Cargo.lock with cargo metadata. Original license texts follow in source-notices/.', '', '| Package | Version | License expression | Upstream |', '| --- | --- | --- | --- |')
    foreach ($package in ($metadata.packages | Where-Object { $_.source } | Sort-Object name, version)) {
        if (-not $package.license) { throw (Get-UnbloatText 'licenses.missing' @($package.name)) }
        if ($package.license -notmatch '^(MIT|Apache-2.0|Unlicense|Unicode-3.0|BSL-1.0|Zlib|ISC|BSD-3-Clause|0BSD|/|\(|\)| OR | AND )+$') { throw (Get-UnbloatText 'licenses.review' @($package.name, $package.license)) }
        $url = if ($package.repository) { $package.repository } else { "https://crates.io/crates/$($package.name)/$($package.version)" }
        $inventory += "| $($package.name) | $($package.version) | $($package.license) | $url |"
        $source = Split-Path $package.manifest_path
        $notices = @(Get-ChildItem -LiteralPath $source -File | Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE|COPYRIGHT|UNLICENSE)' })
        if (-not $notices.Count) { throw (Get-UnbloatText 'licenses.no_files' @($package.name)) }
        $target = Join-Path $destination "$($package.name)-$($package.version)"
        New-Item -ItemType Directory -Path $target -Force | Out-Null
        foreach ($file in $notices) { Copy-Item -LiteralPath $file.FullName -Destination $target -Force }
    }
    $inventory | Set-Content -LiteralPath (Join-Path $root 'licenses\DEPENDENCIES.md') -Encoding UTF8
    Write-Host (Get-UnbloatText 'licenses.complete')
} finally { Pop-Location }
