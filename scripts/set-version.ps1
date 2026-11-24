[CmdletBinding()]
param(
    [Parameter(Mandatory)]
    [ValidatePattern('^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$')]
    [string]$Version
)

$ErrorActionPreference = 'Stop'
$parts = $Version.Split('.') | ForEach-Object { [int]$_ }
if ($parts | Where-Object { $_ -gt 65535 }) {
    throw 'Cada componente de la versión debe caber en la versión de archivo de Windows.'
}
$root = Split-Path $PSScriptRoot
$updated = foreach ($name in 'Cargo.toml', 'Cargo.lock') {
    $path = Join-Path $root $name
    $content = [IO.File]::ReadAllText($path)
    $pattern = '(?m)(^name = "cut"\r?\nversion = ")[^"]+(")'
    if ([regex]::Matches($content, $pattern).Count -ne 1) {
        throw "No se encontró una sola versión de Cut en $name."
    }
    $output = [regex]::Replace($content, $pattern, { param($m) $m.Groups[1].Value + $Version + $m.Groups[2].Value })
    [pscustomobject]@{ Path = $path; Content = $output }
}
foreach ($file in $updated) {
    [IO.File]::WriteAllText($file.Path, $file.Content)
}
