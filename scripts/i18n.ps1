# Shared catalog loader. Source and locale files are always read as UTF-8.
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$script:UnbloatLocaleRoot = Join-Path $PSScriptRoot 'locales'
if (-not (Test-Path -LiteralPath $script:UnbloatLocaleRoot -PathType Container)) {
    $script:UnbloatLocaleRoot = Join-Path (Split-Path $PSScriptRoot) 'locales'
}
$tag = if ([string]::IsNullOrWhiteSpace($env:UNBLOAT_LANG)) { (Get-UICulture).Name } else { $env:UNBLOAT_LANG.Trim() }
$script:UnbloatLanguage = if (($tag -split '[-_.@]')[0] -ieq 'pt') { 'pt-BR' } else { 'en' }
function Read-UnbloatCatalog([string]$Language) {
    $catalog = @{}
    foreach ($line in [IO.File]::ReadAllLines((Join-Path $script:UnbloatLocaleRoot "$Language.txt"), [Text.Encoding]::UTF8)) {
        if (-not $line -or $line.StartsWith('#')) { continue }
        $parts = $line.Split(@([char]'='), 2)
        if ($parts.Count -ne 2 -or $catalog.ContainsKey($parts[0])) { throw "Invalid localization entry: $line" }
        $catalog[$parts[0]] = $parts[1].Replace('\r', "`r").Replace('\n', "`n")
    }
    return $catalog
}
$script:UnbloatEnglish = Read-UnbloatCatalog 'en'
$script:UnbloatCatalog = if ($script:UnbloatLanguage -eq 'en') { $script:UnbloatEnglish } else { Read-UnbloatCatalog $script:UnbloatLanguage }
function Get-UnbloatText([string]$Key, [object[]]$Values = @()) {
    $template = $script:UnbloatCatalog[$Key]
    if ($null -eq $template) { $template = $script:UnbloatEnglish[$Key] }
    if ($null -eq $template) { throw "Unknown localization key: $Key" }
    $replace = {
        param($Match)
        $index = [int]$Match.Groups[1].Value
        if ($index -lt $Values.Count) { return [string]$Values[$index] }
        return $Match.Value
    }.GetNewClosure()
    return [regex]::Replace($template, '\{(\d+)\}', [Text.RegularExpressions.MatchEvaluator]$replace)
}
