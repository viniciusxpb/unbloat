# Regression test for opening unbloat.exe with a double click (no arguments).
# Uses the real native window and real installer, with an isolated installation target.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
. (Join-Path $PSScriptRoot 'i18n.ps1')
$clsid = '{C74E4F58-593D-4FB5-AE90-5683C2D96374}'
$knownClsids = @($clsid, '{C74E4F58-593D-4FB5-AE90-5683C2D96373}', '{C74E4F58-593D-4FB5-AE90-5683C2D96372}', '{C74E4F58-593D-4FB5-AE90-5683C2D96371}')
if ($knownClsids | Where-Object { Test-Path -LiteralPath "HKCU:\Software\Classes\CLSID\$_" }) { throw 'Use um usuário de teste sem Unbloat instalado.' }
$base = Join-Path $root ('target\launcher-validation-' + [Guid]::NewGuid().ToString('N'))
$portable = Join-Path $base 'portable'
$installed = Join-Path $base 'installed'
New-Item -Path $portable -ItemType Directory -Force | Out-Null
Copy-Item -Path (Join-Path $root 'dist\Unbloat\*') -Destination $portable -Recurse
# Redirect only the test installation target; invoke the shipping installer itself.
function Quote-PS([string]$value) { "'" + $value.Replace("'", "''") + "'" }
$wrapper = '& ' + (Quote-PS (Join-Path $root 'scripts\install.ps1')) + ' -SourceDir ' + (Quote-PS $portable) + ' -InstallDir ' + (Quote-PS $installed)
[IO.File]::WriteAllText((Join-Path $portable 'install.ps1'), $wrapper, [Text.UTF8Encoding]::new($true))
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class UnbloatLauncherTest {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string cls, string text);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr window, StringBuilder text, int count);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, IntPtr w, StringBuilder text, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr window, StringBuilder text, int count);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr window, int id);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window, IntPtr hdc, uint flags);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
}
'@
function Start-App([string]$Executable) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Executable
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    [Diagnostics.Process]::Start($info)
}
function Wait-Window($Process) {
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        $Process.Refresh()
        if ($Process.HasExited) { throw "Executável encerrou antes de mostrar a janela (exit $($Process.ExitCode))." }
        if ($Process.MainWindowHandle -ne [IntPtr]::Zero) {
            $name = [Text.StringBuilder]::new(128)
            [UnbloatLauncherTest]::GetClassName($Process.MainWindowHandle, $name, $name.Capacity) | Out-Null
            if ($name.ToString() -eq 'UnbloatWelcome') { return $Process.MainWindowHandle }
        }
        Start-Sleep -Milliseconds 30
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Janela inicial não apareceu.'
}
function Label-Text([IntPtr]$Window) {
    $label = [UnbloatLauncherTest]::GetDlgItem($Window, 30)
    $text = [Text.StringBuilder]::new(4096)
    $result = [IntPtr]::Zero
    [UnbloatLauncherTest]::SendMessageTimeout($label, 0xD, [IntPtr]$text.Capacity, $text, 2, 2000, [ref]$result) | Out-Null
    $text.ToString()
}
function Close-App($Process) {
    if ($Process -and -not $Process.HasExited) {
        $Process.Refresh()
        [UnbloatLauncherTest]::PostMessage($Process.MainWindowHandle, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
        if (-not $Process.WaitForExit(30000)) { throw 'Janela não encerrou com segurança.' }
    }
}
$app = $null
$installedApp = $null
try {
    $app = Start-App (Join-Path $portable 'unbloat.exe')
    $window = Wait-Window $app
    Start-Sleep -Milliseconds 2200
    if ($app.HasExited -or -not [UnbloatLauncherTest]::IsWindowVisible($window)) { throw 'Janela fechou ou ficou invisível.' }
    $initialText = Label-Text $window
    if ($initialText -ne (Get-UnbloatText 'launcher.welcome')) { throw "Instruções de instalação ausentes. Texto recebido: $initialText" }
    $button = [UnbloatLauncherTest]::GetDlgItem($window, 10)
    if (-not [UnbloatLauncherTest]::IsWindowEnabled($button)) { throw 'Botão de instalação desabilitado.' }
    $rect = [UnbloatLauncherTest+RECT]::new()
    [UnbloatLauncherTest]::GetWindowRect($window, [ref]$rect) | Out-Null
    $bitmap = [Drawing.Bitmap]::new($rect.Right - $rect.Left, $rect.Bottom - $rect.Top)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $hdc = $graphics.GetHdc()
    try { [UnbloatLauncherTest]::PrintWindow($window, $hdc, 2) | Out-Null } finally { $graphics.ReleaseHdc($hdc) }
    $bitmap.Save((Join-Path $root 'target\launcher-preview.png'), [Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
    [UnbloatLauncherTest]::PostMessage($window, 0x111, [IntPtr]10, [IntPtr]::Zero) | Out-Null
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    do {
        if ($app.HasExited) { throw 'Janela encerrou durante instalação.' }
        if ((Label-Text $window) -eq (Get-UnbloatText 'launcher.installed' @((Get-UnbloatText 'launcher.usage')))) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    if ((Label-Text $window) -ne (Get-UnbloatText 'launcher.installed' @((Get-UnbloatText 'launcher.usage')))) { throw 'Instalação pela janela não foi concluída.' }
    if (-not (Test-Path -LiteralPath (Join-Path $installed 'unbloat.exe'))) { throw 'Instalador não copiou o executável.' }
    Close-App $app
    if ($app.ExitCode -ne 0) { throw 'Janela de instalação encerrou com erro.' }
    $installedApp = Start-App (Join-Path $installed 'unbloat.exe')
    $installedWindow = Wait-Window $installedApp
    if ((Label-Text $installedWindow) -ne (Get-UnbloatText 'launcher.already_installed' @((Get-UnbloatText 'launcher.usage')))) { throw 'Aplicativo instalado não mostrou instruções de uso.' }
    if ([UnbloatLauncherTest]::IsWindowEnabled([UnbloatLauncherTest]::GetDlgItem($installedWindow, 10))) { throw 'Aplicativo instalado não deve tentar sobrescrever seu próprio executável em uso.' }
    Close-App $installedApp
    # GUI subsystem must preserve redirected CLI stdout as well.
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = Join-Path $portable 'unbloat.exe'; $info.Arguments = '--help'
    $info.UseShellExecute = $false; $info.CreateNoWindow = $true; $info.RedirectStandardOutput = $true
    $info.StandardOutputEncoding = [Text.UTF8Encoding]::new($false, $true)
    $cli = [Diagnostics.Process]::Start($info)
    $help = $cli.StandardOutput.ReadToEnd(); $cli.WaitForExit()
    if ($cli.ExitCode -ne 0 -or -not $help.StartsWith((Get-UnbloatText 'app.name'))) { throw 'CLI stdout regression.' }
    $cli.Dispose()
} finally {
    Close-App $app; Close-App $installedApp
    if (Test-Path -LiteralPath (Join-Path $installed '.unbloat-install.json')) {
        & powershell -NoProfile -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot 'uninstall.ps1') -InstallDir $installed
        if ($LASTEXITCODE -ne 0) { throw 'Falha ao limpar instalação temporária.' }
    }
}
if (Test-Path -LiteralPath "HKCU:\Software\Classes\CLSID\$clsid") { throw 'Registro temporário não foi removido.' }
'PASS: no-argument launch stays visible, install button runs real installer, installed app explains usage, CLI stdout preserved, test registration removed.'
