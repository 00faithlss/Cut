[CmdletBinding()]
param(
    [switch]$Release,
    [string]$TargetDirectory
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
if (!$TargetDirectory) { $TargetDirectory = Join-Path $root 'target' }
$TargetDirectory = [IO.Path]::GetFullPath($TargetDirectory, $root)
$profile = if ($Release) { 'release' } else { 'debug' }
$cargo = Get-Command cargo -ErrorAction SilentlyContinue
if (!$cargo) {
    $cargoPath = Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe'
    if (!(Test-Path -LiteralPath $cargoPath)) { throw 'Instala Rust mediante rustup.' }
    $cargo = Get-Command $cargoPath
}
$toolRoot = Join-Path $env:LOCALAPPDATA 'CutClipper/build-tools/mingw64/bin'
$previousPath = $env:Path
if (Test-Path -LiteralPath $toolRoot) { $env:Path = "$toolRoot;$env:Path" }

Push-Location $root
try {
    $buildOptions = @('build', '--locked', '--target-dir', $TargetDirectory)
    if ($Release) { $buildOptions += '--release' }
    & $cargo.Source @buildOptions
    if ($LASTEXITCODE -ne 0) { throw "Falló la compilación de Cut ($LASTEXITCODE)." }

    $output = Join-Path $TargetDirectory $profile
    $loader = Get-ChildItem (Join-Path $output 'build') -Recurse -File -Filter 'WebView2Loader.dll' |
        Where-Object { $_.Directory.Name -eq 'x64' } |
        Sort-Object LastWriteTime -Descending |
        Select-Object -First 1
    if (!$loader) { throw 'No se encontró WebView2Loader.dll en los artefactos de compilación.' }
    Copy-Item -LiteralPath $loader.FullName -Destination $output
    Get-Item -LiteralPath (Join-Path $output 'cut.exe')
} finally {
    Pop-Location
    $env:Path = $previousPath
}
