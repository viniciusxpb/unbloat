# Regression test for opening unbloat.exe with a double click (no arguments).
# Uses the real native window, installer and uninstaller, with an isolated installation folder.
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
# Same catalogs and language choice as the application.
$language = if ([string]::IsNullOrWhiteSpace($env:UNBLOAT_LANG)) { (Get-UICulture).Name } else { $env:UNBLOAT_LANG }
$language = if (($language -split '[-_.@]')[0] -ieq 'pt') { 'pt-BR' } else { 'en' }
$catalog = @{}
foreach ($line in [IO.File]::ReadAllLines((Join-Path $root "locales\$language.txt"), [Text.Encoding]::UTF8)) {
    if ($line -and -not $line.StartsWith('#')) { $parts = $line.Split(@([char]'='), 2); $catalog[$parts[0]] = $parts[1].Replace('\r', "`r").Replace('\n', "`n") }
}
function Get-UnbloatText([string]$Key, [object[]]$Values = @()) {
    $text = $catalog[$Key]
    for ($i = 0; $i -lt $Values.Count; $i++) { $text = $text.Replace("{$i}", [string]$Values[$i]) }
    $text
}
$clsid = '{C74E4F58-593D-4FB5-AE90-5683C2D96374}'
$knownClsids = @($clsid, '{C74E4F58-593D-4FB5-AE90-5683C2D96373}', '{C74E4F58-593D-4FB5-AE90-5683C2D96372}', '{C74E4F58-593D-4FB5-AE90-5683C2D96371}')
if ($knownClsids | Where-Object { Test-Path -LiteralPath "HKCU:\Software\Classes\CLSID\$_" }) { throw 'Use um usuário de teste sem Unbloat instalado.' }
$base = Join-Path $root ('target\launcher-validation-' + [Guid]::NewGuid().ToString('N'))
$portable = Join-Path $base 'portable'
$installed = Join-Path $base 'installed'
New-Item -Path $portable -ItemType Directory -Force | Out-Null
Copy-Item -Path (Join-Path $root 'dist\Unbloat\*') -Destination $portable -Recurse
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class UnbloatLauncherTest {
    public delegate bool EnumProc(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc proc, IntPtr data);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, IntPtr w, StringBuilder text, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr window, StringBuilder text, int count);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr w, IntPtr l);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr window, int id);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window, IntPtr hdc, uint flags);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left,Top,Right,Bottom; }
    public static IntPtr Find(uint process, string className) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((window, data) => {
            uint owner; GetWindowThreadProcessId(window, out owner);
            var name = new StringBuilder(64); GetClassName(window, name, name.Capacity);
            if (owner == process && IsWindowVisible(window) && name.ToString() == className) { found = window; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@
function Start-App([string]$Executable) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $Executable
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    # Redirect only the installation folder; the shipping installer code runs unchanged.
    $info.EnvironmentVariables['UNBLOAT_INSTALL_DIR'] = $installed
    [Diagnostics.Process]::Start($info)
}
function Wait-Window($Process, [string]$Class = 'UnbloatWelcome') {
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    do {
        if ($Process.HasExited) { throw "Executável encerrou antes de mostrar a janela (exit $($Process.ExitCode))." }
        $window = [UnbloatLauncherTest]::Find([uint32]$Process.Id, $Class)
        if ($window -ne [IntPtr]::Zero) { return $window }
        Start-Sleep -Milliseconds 30
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Janela '$Class' não apareceu."
}
# The description control holds the title, a blank line and the body without markup.
function Expected([string]$Title, [string]$Body) { (Get-UnbloatText $Title) + "`n`n" + (Get-UnbloatText $Body).Replace("**", "") }
function Label-Text([IntPtr]$Window) {
    $label = [UnbloatLauncherTest]::GetDlgItem($Window, 30)
    $text = [Text.StringBuilder]::new(4096)
    $result = [IntPtr]::Zero
    [UnbloatLauncherTest]::SendMessageTimeout($label, 0xD, [IntPtr]$text.Capacity, $text, 2, 2000, [ref]$result) | Out-Null
    $text.ToString().Replace("`r`n", "`n")
}
function Wait-Label([IntPtr]$Window, $Process, [string]$Expected, [string]$Failure) {
    $deadline = [DateTime]::UtcNow.AddSeconds(45)
    do {
        if ($Process.HasExited) { throw "Janela encerrou: $Failure" }
        if ((Label-Text $Window) -eq $Expected) { return }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "$Failure Texto recebido: $(Label-Text $Window)"
}
function Close-App($Process) {
    if ($Process -and -not $Process.HasExited) {
        $window = [UnbloatLauncherTest]::Find([uint32]$Process.Id, 'UnbloatWelcome')
        [UnbloatLauncherTest]::PostMessage($window, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
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
    if ($initialText -ne (Expected "launcher.welcome_title" "launcher.welcome")) { throw "Instruções de instalação ausentes. Texto recebido: $initialText" }
    if (-not [UnbloatLauncherTest]::IsWindowEnabled([UnbloatLauncherTest]::GetDlgItem($window, 10))) { throw 'Botão de instalação desabilitado.' }
    if ([UnbloatLauncherTest]::IsWindowEnabled([UnbloatLauncherTest]::GetDlgItem($window, 40))) { throw 'Desinstalar habilitado sem instalação.' }
    $rect = [UnbloatLauncherTest+RECT]::new()
    [UnbloatLauncherTest]::GetWindowRect($window, [ref]$rect) | Out-Null
    $bitmap = [Drawing.Bitmap]::new($rect.Right - $rect.Left, $rect.Bottom - $rect.Top)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $hdc = $graphics.GetHdc()
    try { [UnbloatLauncherTest]::PrintWindow($window, $hdc, 2) | Out-Null } finally { $graphics.ReleaseHdc($hdc) }
    $bitmap.Save((Join-Path $root 'target\launcher-preview.png'), [Drawing.Imaging.ImageFormat]::Png)
    $graphics.Dispose(); $bitmap.Dispose()
    [UnbloatLauncherTest]::PostMessage($window, 0x111, [IntPtr]10, [IntPtr]::Zero) | Out-Null
    Wait-Label $window $app (Expected "launcher.installed" "launcher.usage") 'Instalação pela janela não foi concluída.'
    if (-not (Test-Path -LiteralPath (Join-Path $installed 'unbloat.exe'))) { throw 'Instalador não copiou o executável.' }
    if (-not (Test-Path -LiteralPath "HKCU:\Software\Classes\CLSID\$clsid\InprocServer32")) { throw 'Instalador não registrou o COM.' }
    Close-App $app
    if ($app.ExitCode -ne 0) { throw 'Janela de instalação encerrou com erro.' }
    $installedApp = Start-App (Join-Path $installed 'unbloat.exe')
    $installedWindow = Wait-Window $installedApp
    if ((Label-Text $installedWindow) -ne (Expected "launcher.already_installed" "launcher.usage")) { throw 'Aplicativo instalado não mostrou instruções de uso.' }
    if ([UnbloatLauncherTest]::IsWindowEnabled([UnbloatLauncherTest]::GetDlgItem($installedWindow, 10))) { throw 'Aplicativo instalado não deve tentar sobrescrever seu próprio executável em uso.' }
    # Uninstall from the installed copy itself, confirming the question.
    [UnbloatLauncherTest]::PostMessage($installedWindow, 0x111, [IntPtr]40, [IntPtr]::Zero) | Out-Null
    $question = Wait-Window $installedApp '#32770'
    [UnbloatLauncherTest]::PostMessage($question, 0x111, [IntPtr]6, [IntPtr]::Zero) | Out-Null # IDYES
    Wait-Label $installedWindow $installedApp (Expected "launcher.uninstalled" "launcher.uninstalled_body") 'Desinstalação pela janela não foi concluída.'
    if (Test-Path -LiteralPath "HKCU:\Software\Classes\CLSID\$clsid") { throw 'Desinstalação manteve o registro COM.' }
    if (Test-Path -LiteralPath $installed) { throw "Desinstalação manteve a pasta: $((Get-ChildItem -LiteralPath $installed -Recurse -Force | ForEach-Object Name) -join ', ')" }
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
        & (Join-Path $portable 'unbloat.exe') --uninstall --install-dir $installed | Out-Host
        if ($LASTEXITCODE -ne 0) { throw 'Falha ao limpar instalação temporária.' }
    }
}
if (Test-Path -LiteralPath "HKCU:\Software\Classes\CLSID\$clsid") { throw 'Registro temporário não foi removido.' }
'PASS: no-argument launch stays visible, install button runs the real installer, installed app explains usage and uninstalls itself, CLI stdout preserved, test registration removed.'
