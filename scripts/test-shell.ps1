# This test temporarily registers Unbloat in HKCU and always unregisters it.
[CmdletBinding()]
param([switch]$PortableOnly)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$clsid = '{C74E4F58-593D-4FB5-AE90-5683C2D96373}'
$knownClsids = @($clsid, '{C74E4F58-593D-4FB5-AE90-5683C2D96372}', '{C74E4F58-593D-4FB5-AE90-5683C2D96371}')
if (-not $PortableOnly -and ($knownClsids | Where-Object { Test-Path -LiteralPath "HKCU:\Software\Classes\CLSID\$_" })) { throw 'Uma instalação já existe. Use -PortableOnly ou um usuário de teste sem Unbloat instalado.' }
$base = Join-Path $root ('target\shell-validation-' + [Guid]::NewGuid().ToString('N'))
$installed = Join-Path $base 'installed'
$portable = Join-Path $base 'portable'
$projects = Join-Path $base 'projects'
$probe = Join-Path $root 'target\release\examples\shell_probe.exe'
if (-not (Test-Path -LiteralPath $probe)) { throw 'Execute cargo build --workspace --release --examples --bins --lib --locked' }
$results = @()
function Invoke-Probe([string]$Dll, [string[]]$Folders, [string[]]$Flags) {
    & $probe $Dll @Folders @Flags
    if ($LASTEXITCODE -ne 0) { throw 'shell_probe falhou' }
}
New-Item -Path $portable, $projects -ItemType Directory -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $root 'target\release\unbloat.exe') -Destination $portable
Copy-Item -LiteralPath (Join-Path $root 'target\release\unbloat_shell.dll') -Destination $portable
Copy-Item -LiteralPath (Join-Path $root 'config.toml') -Destination $portable
$folders = @('front end', ('projeto ' + [char]0x00E7 + [char]0x00E3 + 'o'), 'service') | ForEach-Object { Join-Path $projects $_ }
foreach ($folder in $folders) {
    New-Item -Path (Join-Path $folder 'src'), (Join-Path $folder 'node_modules') -ItemType Directory -Force | Out-Null
    [IO.File]::WriteAllText((Join-Path $folder '.gitignore'), "node_modules/`n*.log`n")
    [IO.File]::WriteAllText((Join-Path $folder 'src\main.txt'), ('source code' * 3000))
    [IO.File]::WriteAllText((Join-Path $folder 'node_modules\excluded'), 'must not be archived')
}
$image = Join-Path $projects ('foto ' + [char]0x00E7 + '.png')
[IO.File]::WriteAllBytes($image, [Convert]::FromBase64String('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII='))
$textFile = Join-Path $projects 'read me.txt'
[IO.File]::WriteAllText($textFile, 'explicit selected file')
try {
    $dll = Join-Path $portable 'unbloat_shell.dll'
    Invoke-Probe $dll @($folders[0]) @('--invoke'); $results += '- PASS: single folder, separate process, native UI'
    Invoke-Probe $dll @($folders[0], $folders[1]) @('--invoke'); $results += '- PASS: two siblings, Unicode and spaces'
    Invoke-Probe $dll $folders @('--invoke'); $results += '- PASS: three siblings'
    Invoke-Probe $dll @($image) @('--invoke'); $results += '- PASS: individual PNG image with Unicode/spaces'
    Invoke-Probe $dll @($image, $textFile) @('--invoke'); $results += '- PASS: multiple individual files'
    Invoke-Probe $dll @($image, $folders[0]) @('--invoke'); $results += '- PASS: mixed file and folder selection'
    $large = Join-Path $folders[2] 'large.bin'
    $file = [IO.File]::Create($large); $file.SetLength(256MB); $file.Dispose()
    Invoke-Probe $dll @($folders[2]) @('--invoke', '--cancel'); $results += '- PASS: safe cancellation, no archive published'
    Remove-Item -LiteralPath $large
    if ($PortableOnly) {
        $results | Set-Content -LiteralPath (Join-Path $root 'target\SHELL-RESULTS.md') -Encoding UTF8
        $results | Write-Output
        return
    }
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'install.ps1') -InstallDir $installed
    if ($LASTEXITCODE -ne 0) { throw 'Instalação falhou' }
    $installedManifest = Get-Content -LiteralPath (Join-Path $installed '.unbloat-install.json') -Raw | ConvertFrom-Json
    Invoke-Probe (Join-Path $installed $installedManifest.shell_dll) @($folders[0], $folders[1]) @('--invoke', '--registered')
    $results += '- PASS: per-user installation, registered COM activation and invocation'
    & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'install.ps1') -InstallDir $installed
    if ($LASTEXITCODE -ne 0) { throw 'Reinstalação falhou' }
    $results += '- PASS: reinstallation'
    # Exercise installation while the actual application is compressing.
    $large = Join-Path $folders[2] 'large.bin'
    $file = [IO.File]::Create($large); $file.SetLength(512MB); $file.Dispose()
    $startInfo = [Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = Join-Path $installed 'unbloat.exe'
    $startInfo.Arguments = '--shell "' + $folders[2] + '"'
    $startInfo.UseShellExecute = $false
    $startInfo.CreateNoWindow = $true # Hide the console while allowing the app's explicit Win32 progress window.
    $running = [Diagnostics.Process]::Start($startInfo)
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(10)
        do { $running.Refresh(); if ($running.MainWindowHandle -ne [IntPtr]::Zero) { break }; Start-Sleep -Milliseconds 20 } while (-not $running.HasExited -and [DateTime]::UtcNow -lt $deadline)
        if ($running.HasExited -or $running.MainWindowHandle -eq [IntPtr]::Zero) { throw 'Aplicativo de teste não iniciou a janela' }
        $previousPreference = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'install.ps1') -InstallDir $installed 2>&1 | Out-File -LiteralPath (Join-Path $base 'expected-in-use-error.txt')
        $exitCode = $LASTEXITCODE
        $ErrorActionPreference = $previousPreference
        if ($exitCode -eq 0) { throw 'Instalador não recusou binário em uso' }
    } finally {
        $ErrorActionPreference = 'Stop'
        if (-not $running.HasExited) {
            Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class UnbloatTestClose { [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr w, uint m, IntPtr a, IntPtr b); }'
            [UnbloatTestClose]::PostMessage($running.MainWindowHandle, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
            if (-not $running.WaitForExit(30000)) { throw 'Aplicativo não terminou após cancelamento seguro' }
        }
        $running.Dispose()
        Remove-Item -LiteralPath $large
    }
    $results += '- PASS: installation while the application is running fails clearly; safe cancellation finishes normally'
} finally {
    if (Test-Path -LiteralPath (Join-Path $installed '.unbloat-install.json')) {
        & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'uninstall.ps1') -InstallDir $installed
        if ($LASTEXITCODE -ne 0) { throw "Limpeza de instalação falhou: $installed" }
    }
}
if (Test-Path -LiteralPath "HKCU:\Software\Classes\CLSID\$clsid") { throw 'Registro COM não removido' }
if (Test-Path -LiteralPath 'HKCU:\Software\Classes\Directory\shellex\ContextMenuHandlers\Unbloat') { throw 'Handler não removido' }
if (Test-Path -LiteralPath 'HKCU:\Software\Classes\*\shellex\ContextMenuHandlers\Unbloat') { throw 'Handler de arquivos não removido' }
if (Test-Path -LiteralPath $installed) { throw 'Componentes instalados não removidos' }
if (Get-ChildItem -LiteralPath $projects -Filter '*.unbloat-part' -Force) { throw 'Temporário não removido' }
$results += '- PASS: uninstallation and registry cleanup'
$results += '- Manual visual Explorer check: not automated by this test'
$results | Set-Content -LiteralPath (Join-Path $root 'target\SHELL-RESULTS.md') -Encoding UTF8
$results | Write-Output
