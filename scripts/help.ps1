[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'i18n.ps1')
Write-Output (Get-UnbloatText 'make.usage')
Write-Output ''
Write-Output (Get-UnbloatText 'make.heading')
foreach ($command in @('help', 'build', 'test', 'run', 'install', 'uninstall', 'package', 'benchmark')) {
    Write-Output ('  {0,-11} {1}' -f $command, (Get-UnbloatText "make.$command"))
}
