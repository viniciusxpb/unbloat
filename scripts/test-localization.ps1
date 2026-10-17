# UTF-8 output, language fallback, native controls and installer diagnostics.
[CmdletBinding()]
param(
    [string]$Executable,
    [switch]$SkipWindows
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
if (-not $Executable) { $Executable = Join-Path $root 'target\debug\unbloat.exe' }
$Executable = [IO.Path]::GetFullPath($Executable)
if (-not (Test-Path -LiteralPath $Executable)) { throw 'Build unbloat before running localization checks.' }
. (Join-Path $PSScriptRoot 'i18n.ps1')
$powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
function Start-LocalizedProcess([string]$File, [string]$Arguments, [string]$Language, [bool]$Redirect) {
    $info = [Diagnostics.ProcessStartInfo]::new()
    $info.FileName = $File
    $info.Arguments = $Arguments
    $info.WorkingDirectory = $root
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    if ($Language) { $info.EnvironmentVariables['UNBLOAT_LANG'] = $Language }
    else { $info.EnvironmentVariables.Remove('UNBLOAT_LANG') }
    if ($Redirect) {
        $info.RedirectStandardOutput = $true
        $info.RedirectStandardError = $true
        $info.StandardOutputEncoding = [Text.UTF8Encoding]::new($false, $true)
        $info.StandardErrorEncoding = [Text.UTF8Encoding]::new($false, $true)
    }
    return [Diagnostics.Process]::Start($info)
}
function Capture([string]$File, [string]$Arguments, [string]$Language, [int]$ExitCode = 0) {
    $process = Start-LocalizedProcess $File $Arguments $Language $true
    try {
        $output = $process.StandardOutput.ReadToEnd()
        $errors = $process.StandardError.ReadToEnd()
        if (-not $process.WaitForExit(30000)) { throw 'Localized command timed out.' }
        if ($process.ExitCode -ne $ExitCode) { throw "Unexpected exit code: $($process.ExitCode). $errors" }
        return ($output + $errors).Replace("`r`n", "`n").TrimEnd()
    } finally { $process.Dispose() }
}
if (-not $SkipWindows) {
    Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class UnbloatLocaleTest {
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowText(IntPtr hwnd, StringBuilder text, int count);
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr hwnd, int id);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr w, IntPtr l);
}
'@
}
foreach ($language in @('pt-BR', 'en', 'pt-PT', 'fr-FR', '')) {
    $resolved = if (-not $language) { (Get-UICulture).Name } else { $language }
    $catalog = Read-UnbloatCatalog $(if ($resolved -like 'pt*') { 'pt-BR' } else { 'en' })
    $help = Capture 'make' '--no-print-directory help' $language
    if (-not $help.StartsWith($catalog['make.usage']) -or -not $help.Contains($catalog['make.heading'])) { throw "Help language/encoding mismatch: $language" }
    foreach ($command in @('help', 'build', 'test', 'run', 'install', 'uninstall', 'package', 'benchmark')) {
        if (-not $help.Contains($catalog["make.$command"])) { throw "Missing help entry: $language/$command" }
    }
    if ((Capture 'make' '--no-print-directory' $language) -ne $help) { throw 'make and make help differ.' }
    $cli = Capture $Executable '--help' $language
    $metadata = (& cargo metadata --no-deps --locked --offline --format-version 1) | ConvertFrom-Json
    $version = ($metadata.packages | Where-Object name -eq 'unbloat').version
    $expectedHelp = $catalog['cli.help'].Replace('{0}', $version).Replace("`r`n", "`n")
    if ($cli -ne $expectedHelp) { throw "CLI language/encoding mismatch: $language" }
    $errorText = Capture $Executable '--invalid-option' $language 1
    if (-not $errorText.Contains($catalog['cli.unknown_argument'])) { throw "CLI diagnostic mismatch: $language" }
    $driveRoot = [IO.Path]::GetPathRoot($root) + '.'
    foreach ($script in @('install', 'uninstall')) {
        $arguments = '-NoProfile -ExecutionPolicy Bypass -File "' + (Join-Path $PSScriptRoot "$script.ps1") + '" -InstallDir "' + $driveRoot + '"'
        $diagnostic = Capture $powershell $arguments $language 1
        if (-not $diagnostic.Contains($catalog["$script.invalid_directory"])) { throw "Installer diagnostic mismatch: $language/$script`n$diagnostic" }
    }
    if (-not $SkipWindows) {
        $app = Start-LocalizedProcess $Executable '' $language $false
        try {
            $deadline = [DateTime]::UtcNow.AddSeconds(15)
            do {
                $app.Refresh()
                if ($app.HasExited) { throw 'Application closed before showing its window.' }
                if ($app.MainWindowHandle -ne [IntPtr]::Zero) { break }
                Start-Sleep -Milliseconds 30
            } while ([DateTime]::UtcNow -lt $deadline)
            if ($app.MainWindowHandle -eq [IntPtr]::Zero) { throw 'Application window did not appear.' }
            foreach ($control in @(@(0, 'launcher.title'), @(10, 'launcher.install'), @(20, 'common.close'))) {
                $handle = if ($control[0] -eq 0) { $app.MainWindowHandle } else { [UnbloatLocaleTest]::GetDlgItem($app.MainWindowHandle, $control[0]) }
                $text = [Text.StringBuilder]::new(1024)
                [UnbloatLocaleTest]::GetWindowText($handle, $text, $text.Capacity) | Out-Null
                if ($text.ToString() -ne $catalog[$control[1]]) { throw "Native control mismatch: $language/$($control[1])" }
            }
        } finally {
            if (-not $app.HasExited) {
                [UnbloatLocaleTest]::PostMessage($app.MainWindowHandle, 0x10, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
                if (-not $app.WaitForExit(10000)) { throw 'Application did not close.' }
            }
            $app.Dispose()
        }
    }
    Write-Output "PASS: language='$language', make/CLI/install/uninstall language and UTF-8 checks"
}
# Substitution must never interpret braces contained in a user's path.
if ((Get-UnbloatText 'config.read_failed' @('C:\{1}\file', 'DETAIL')).EndsWith('DETAIL') -eq $false) { throw 'Missing placeholder value.' }
if (-not (Get-UnbloatText 'config.read_failed' @('C:\{1}\file', 'DETAIL')).Contains('C:\{1}\file')) { throw 'Path interpreted as a placeholder.' }
