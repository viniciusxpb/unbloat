[CmdletBinding()]
param(
    [string]$SourceDir,
    [string]$InstallDir = (Join-Path $env:LOCALAPPDATA 'Programs\Unbloat')
)
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'i18n.ps1')
if (-not [Environment]::Is64BitProcess) { throw (Get-UnbloatText 'install.x64') }
$clsid = '{C74E4F58-593D-4FB5-AE90-5683C2D96374}'
$legacyClsids = @('{C74E4F58-593D-4FB5-AE90-5683C2D96373}', '{C74E4F58-593D-4FB5-AE90-5683C2D96372}', '{C74E4F58-593D-4FB5-AE90-5683C2D96371}')
$repository = Split-Path $PSScriptRoot
if (-not $SourceDir) {
    $SourceDir = if (Test-Path -LiteralPath (Join-Path $PSScriptRoot 'unbloat.exe')) { $PSScriptRoot } else { Join-Path $repository 'target\release' }
}
$SourceDir = [IO.Path]::GetFullPath($SourceDir)
$InstallDir = [IO.Path]::GetFullPath($InstallDir)
if ($InstallDir -eq [IO.Path]::GetPathRoot($InstallDir)) { throw (Get-UnbloatText 'install.invalid_directory') }
# After the drive-root check: relative names and directory comparisons assume no trailing separator.
$InstallDir = $InstallDir.TrimEnd('\')
if (Test-Path -LiteralPath $InstallDir) {
    if ((Get-Item -LiteralPath $InstallDir).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw (Get-UnbloatText 'install.link') }
}
$exe = Join-Path $SourceDir 'unbloat.exe'
$dll = Join-Path $SourceDir 'unbloat-shell.dll'
if (-not (Test-Path -LiteralPath $dll)) { $dll = Join-Path $SourceDir 'unbloat_shell.dll' }
foreach ($file in @($exe, $dll)) { if (-not (Test-Path -LiteralPath $file)) { throw (Get-UnbloatText 'install.missing_file' @($file)) } }
$resources = if (Test-Path -LiteralPath (Join-Path $SourceDir 'LICENSE')) { $SourceDir } else { $repository }
if ($resources -eq $repository -and (Test-Path -LiteralPath (Join-Path $repository 'Cargo.toml'))) {
    # Development installs regenerate notices excluded from the source repository.
    Push-Location $repository
    try {
        & cargo xtask licenses
        if ($LASTEXITCODE -ne 0) { throw (Get-UnbloatText 'install.missing_notices') }
    } finally { Pop-Location }
}
foreach ($file in @('config.toml', 'LICENSE', 'licenses\THIRD_PARTY.md', 'licenses\DEPENDENCIES.md', 'locales\en.txt', 'locales\pt-BR.txt')) {
    if (-not (Test-Path -LiteralPath (Join-Path $resources $file))) { throw (Get-UnbloatText 'install.missing_resource' @($file)) }
}
if (-not (Test-Path -LiteralPath (Join-Path $resources 'licenses\source-notices') -PathType Container)) { throw (Get-UnbloatText 'install.missing_notices') }
# Copy before touching the registry. A content-named DLL allows migration from
# the previous handler even while Explorer still has that old DLL mapped.
New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
Copy-Item -LiteralPath $exe -Destination (Join-Path $InstallDir 'unbloat.exe') -Force
$dllHash = (Get-FileHash -LiteralPath $dll -Algorithm SHA256).Hash.ToLowerInvariant()
$installedDllName = 'unbloat-shell-' + $dllHash.Substring(0, 16) + '.dll'
$installedDll = Join-Path $InstallDir $installedDllName
if (Test-Path -LiteralPath $installedDll) {
    if ((Get-FileHash -LiteralPath $installedDll -Algorithm SHA256).Hash.ToLowerInvariant() -ne $dllHash) { throw (Get-UnbloatText 'install.hash') }
} else { Copy-Item -LiteralPath $dll -Destination $installedDll }
if (-not (Test-Path -LiteralPath (Join-Path $InstallDir 'config.toml'))) {
    Copy-Item -LiteralPath (Join-Path $resources 'config.toml') -Destination $InstallDir
}
Copy-Item -LiteralPath (Join-Path $resources 'LICENSE') -Destination $InstallDir -Force
Copy-Item -LiteralPath (Join-Path $resources 'licenses') -Destination $InstallDir -Recurse -Force
Copy-Item -LiteralPath (Join-Path $resources 'locales') -Destination $InstallDir -Recurse -Force
foreach ($script in @('install.ps1', 'uninstall.ps1', 'i18n.ps1')) {
    $source = Join-Path $PSScriptRoot $script
    $target = Join-Path $InstallDir $script
    if ([IO.Path]::GetFullPath($source) -ne [IO.Path]::GetFullPath($target)) { Copy-Item -LiteralPath $source -Destination $target -Force }
}
$ownedFiles = @('unbloat.exe', $installedDllName, 'config.toml', 'LICENSE', 'install.ps1', 'uninstall.ps1', 'i18n.ps1')
# Preserve the ownership list of older installations, including DLLs that may
# remain mapped until Explorer exits; uninstallation can retry them safely.
$previousMarker = Join-Path $InstallDir '.unbloat-install.json'
if (Test-Path -LiteralPath $previousMarker) {
    $previous = Get-Content -LiteralPath $previousMarker -Raw | ConvertFrom-Json
    if ($previous.product -eq 'Unbloat' -and $previous.clsid -in (@($clsid) + $legacyClsids)) { $ownedFiles += @($previous.files) }
}
$ownedFiles += foreach ($directory in @('licenses', 'locales')) {
    Get-ChildItem -LiteralPath (Join-Path $InstallDir $directory) -File -Recurse | ForEach-Object { $_.FullName.Substring($InstallDir.Length + 1) }
}
@{ product = 'Unbloat'; clsid = $clsid; shell_dll = $installedDllName; files = @($ownedFiles | Sort-Object -Unique) } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $InstallDir '.unbloat-install.json') -Encoding UTF8
$classKey = "HKCU:\Software\Classes\CLSID\$clsid"
$handlerKey = 'HKCU:\Software\Classes\Directory\shellex\ContextMenuHandlers\Unbloat'
$fileHandlerKey = 'HKCU:\Software\Classes\*\shellex\ContextMenuHandlers\Unbloat'
New-Item -Path "$classKey\InprocServer32" -Force | Out-Null
Set-Item -LiteralPath $classKey -Value 'Unbloat context menu'
Set-Item -LiteralPath "$classKey\InprocServer32" -Value $installedDll
New-ItemProperty -LiteralPath "$classKey\InprocServer32" -Name ThreadingModel -Value Apartment -PropertyType String -Force | Out-Null
foreach ($key in @($handlerKey, $fileHandlerKey)) {
    # Registry APIs treat the '*' file-class name literally, never as a wildcard.
    $registryKey = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey($key.Substring('HKCU:\'.Length))
    try { $registryKey.SetValue('', $clsid) } finally { $registryKey.Dispose() }
}
try {
    $instance = [Activator]::CreateInstance([Type]::GetTypeFromCLSID([Guid]$clsid, $true))
    $unknown = [Runtime.InteropServices.Marshal]::GetIUnknownForObject($instance)
    try {
        foreach ($id in @('000214E8-0000-0000-C000-000000000046', '000214E4-0000-0000-C000-000000000046')) {
            $iid = [Guid]$id; $interface = [IntPtr]::Zero
            $hr = [Runtime.InteropServices.Marshal]::QueryInterface($unknown, [ref]$iid, [ref]$interface)
            if ($hr -ne 0) { [Runtime.InteropServices.Marshal]::ThrowExceptionForHR($hr) }
            [Runtime.InteropServices.Marshal]::Release($interface) | Out-Null
        }
    } finally {
        [Runtime.InteropServices.Marshal]::Release($unknown) | Out-Null
        [Runtime.InteropServices.Marshal]::FinalReleaseComObject($instance) | Out-Null
    }
} catch {
    Remove-Item -LiteralPath $handlerKey -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $fileHandlerKey -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $classKey -Recurse -Force -ErrorAction SilentlyContinue
    throw (Get-UnbloatText 'install.com_failed' @($_.Exception.Message))
}
Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class UnbloatInstallNotify { [DllImport("shell32.dll")] public static extern void SHChangeNotify(uint e, uint f, IntPtr a, IntPtr b); }'
# Retire only previous class registrations belonging to this installation.
foreach ($legacyClsid in $legacyClsids) {
    $legacyKey = "HKCU:\Software\Classes\CLSID\$legacyClsid"
    $legacyServer = Get-Item -LiteralPath "$legacyKey\InprocServer32" -ErrorAction SilentlyContinue
    if ($legacyServer -and [IO.Path]::GetFullPath([IO.Path]::GetDirectoryName($legacyServer.GetValue(''))) -eq $InstallDir) {
        Remove-Item -LiteralPath $legacyKey -Recurse -Force
    }
}
[UnbloatInstallNotify]::SHChangeNotify(0x08000000, 0, [IntPtr]::Zero, [IntPtr]::Zero)
Write-Host (Get-UnbloatText 'install.complete' @($InstallDir))
Write-Host (Get-UnbloatText 'install.usage')
