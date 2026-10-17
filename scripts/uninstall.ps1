[CmdletBinding()]
param([string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\Unbloat'))
$ErrorActionPreference = 'Stop'
$InstallDir = [IO.Path]::GetFullPath($InstallDir)
if ($InstallDir -eq [IO.Path]::GetPathRoot($InstallDir)) { throw 'Diretório inválido.' }
$marker = Join-Path $InstallDir '.unbloat-install.json'
if (-not (Test-Path -LiteralPath $marker)) {
    if (-not (Test-Path -LiteralPath $InstallDir)) { Write-Host 'Unbloat já está desinstalado.'; return }
    throw 'Marcador de instalação ausente; nenhum arquivo foi removido.'
}
if ((Get-Item -LiteralPath $InstallDir).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Diretório de instalação é link/junction.' }
$manifest = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
$currentClsid = '{C74E4F58-593D-4FB5-AE90-5683C2D96373}'
$legacyClsids = @('{C74E4F58-593D-4FB5-AE90-5683C2D96372}', '{C74E4F58-593D-4FB5-AE90-5683C2D96371}')
if ($manifest.product -ne 'Unbloat' -or $manifest.clsid -notin (@($currentClsid) + $legacyClsids)) { throw 'Marcador de instalação inválido.' }
$clsid = $manifest.clsid
$prefix = $InstallDir.TrimEnd('\') + '\'
$targets = foreach ($relative in $manifest.files) {
    $target = [IO.Path]::GetFullPath((Join-Path $InstallDir $relative))
    if (-not $target.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Caminho fora da instalação.' }
    $ancestor = Split-Path $target
    while ($ancestor.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        if ((Test-Path -LiteralPath $ancestor) -and ((Get-Item -LiteralPath $ancestor).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Link/junction dentro da instalação; nenhum arquivo foi removido.' }
        $ancestor = Split-Path $ancestor
    }
    $target
}
$classKey = "HKCU:\Software\Classes\CLSID\$clsid"
$handlerKey = 'HKCU:\Software\Classes\Directory\shellex\ContextMenuHandlers\Unbloat'
$fileHandlerKey = 'HKCU:\Software\Classes\*\shellex\ContextMenuHandlers\Unbloat'
$registration = Get-Item -LiteralPath "$classKey\InprocServer32" -ErrorAction SilentlyContinue
$registered = if ($registration) { $registration.GetValue('') } else { $null }
if ($registered -and [IO.Path]::GetFullPath([IO.Path]::GetDirectoryName($registered)) -ne $InstallDir) { throw 'Outra instalação está registrada; não será removida.' }
foreach ($key in @($handlerKey, $fileHandlerKey, $classKey)) { if (Test-Path -LiteralPath $key) { Remove-Item -LiteralPath $key -Recurse -Force } }
Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class UnbloatUninstallNotify { [DllImport("shell32.dll")] public static extern void SHChangeNotify(uint e, uint f, IntPtr a, IntPtr b); }'
[UnbloatUninstallNotify]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)
$locked = @()
foreach ($target in $targets) {
    if (Test-Path -LiteralPath $target) {
        try { Remove-Item -LiteralPath $target -Force } catch { $locked += $target }
    }
}
if ($locked.Count) { throw "Menu removido, mas há arquivos em uso. Feche o Unbloat e saia/entre na sessão do Windows; execute este script novamente. Arquivos: $($locked -join ', ')" }
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
Write-Host 'Unbloat desinstalado.'
