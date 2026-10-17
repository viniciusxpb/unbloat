[CmdletBinding()]
param([string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\Unbloat'))
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'i18n.ps1')
$InstallDir = [IO.Path]::GetFullPath($InstallDir)
if ($InstallDir -eq [IO.Path]::GetPathRoot($InstallDir)) { throw (Get-UnbloatText 'uninstall.invalid_directory') }
# After the drive-root check: directory comparisons assume no trailing separator.
$InstallDir = $InstallDir.TrimEnd('\')
$marker = Join-Path $InstallDir '.unbloat-install.json'
if (-not (Test-Path -LiteralPath $marker)) {
    if (-not (Test-Path -LiteralPath $InstallDir)) { Write-Host (Get-UnbloatText 'uninstall.already_removed'); return }
    throw (Get-UnbloatText 'uninstall.missing_marker')
}
if ((Get-Item -LiteralPath $InstallDir).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw (Get-UnbloatText 'uninstall.link') }
$manifest = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
$currentClsid = '{C74E4F58-593D-4FB5-AE90-5683C2D96374}'
$legacyClsids = @('{C74E4F58-593D-4FB5-AE90-5683C2D96373}', '{C74E4F58-593D-4FB5-AE90-5683C2D96372}', '{C74E4F58-593D-4FB5-AE90-5683C2D96371}')
if ($manifest.product -ne 'Unbloat' -or $manifest.clsid -notin (@($currentClsid) + $legacyClsids)) { throw (Get-UnbloatText 'uninstall.invalid_marker') }
$clsid = $manifest.clsid
$prefix = $InstallDir.TrimEnd('\') + '\'
$targets = foreach ($relative in $manifest.files) {
    $target = [IO.Path]::GetFullPath((Join-Path $InstallDir $relative))
    if (-not $target.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { throw (Get-UnbloatText 'uninstall.outside') }
    $ancestor = Split-Path $target
    while ($ancestor.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        if ((Test-Path -LiteralPath $ancestor) -and ((Get-Item -LiteralPath $ancestor).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw (Get-UnbloatText 'uninstall.inner_link') }
        $ancestor = Split-Path $ancestor
    }
    $target
}
$classKey = "HKCU:\Software\Classes\CLSID\$clsid"
$handlerKey = 'HKCU:\Software\Classes\Directory\shellex\ContextMenuHandlers\Unbloat'
$fileHandlerKey = 'HKCU:\Software\Classes\*\shellex\ContextMenuHandlers\Unbloat'
$registration = Get-Item -LiteralPath "$classKey\InprocServer32" -ErrorAction SilentlyContinue
$registered = if ($registration) { $registration.GetValue('') } else { $null }
if ($registered -and [IO.Path]::GetFullPath([IO.Path]::GetDirectoryName($registered)) -ne $InstallDir) { throw (Get-UnbloatText 'uninstall.other_install') }
foreach ($key in @($handlerKey, $fileHandlerKey, $classKey)) { if (Test-Path -LiteralPath $key) { Remove-Item -LiteralPath $key -Recurse -Force } }
Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class UnbloatUninstallNotify { [DllImport("shell32.dll")] public static extern void SHChangeNotify(uint e, uint f, IntPtr a, IntPtr b); }'
[UnbloatUninstallNotify]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)
$locked = @()
# Keep the scripts and catalogs available if a locked binary requires a retry.
$supportPrefix = Join-Path $InstallDir 'locales\'
$support = @($targets | Where-Object { $_.EndsWith('.ps1', [StringComparison]::OrdinalIgnoreCase) -or $_.StartsWith($supportPrefix, [StringComparison]::OrdinalIgnoreCase) })
foreach ($target in ($targets | Where-Object { $_ -notin $support })) {
    if (Test-Path -LiteralPath $target) {
        try { Remove-Item -LiteralPath $target -Force } catch { $locked += $target }
    }
}
if ($locked.Count) { throw (Get-UnbloatText 'uninstall.locked' @(($locked -join ', '))) }
foreach ($target in $support) {
    if (Test-Path -LiteralPath $target) { Remove-Item -LiteralPath $target -Force }
}
Remove-Item -LiteralPath $marker -Force
# Remove only empty directories, preserving any files the user added.
$directories = @($targets | ForEach-Object {
    $directory = Split-Path $_
    while ($directory.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        $directory
        $directory = Split-Path $directory
    }
} | Sort-Object -Unique | Sort-Object Length -Descending)
foreach ($directory in $directories) {
    if ($directory.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase) -and (Test-Path -LiteralPath $directory) -and -not (Get-ChildItem -LiteralPath $directory -Force)) { Remove-Item -LiteralPath $directory }
}
if ((Test-Path -LiteralPath $InstallDir) -and -not (Get-ChildItem -LiteralPath $InstallDir -Force)) { Remove-Item -LiteralPath $InstallDir }
Write-Host (Get-UnbloatText 'uninstall.complete')
