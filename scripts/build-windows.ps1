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

# rust-toolchain.toml selects the MSVC toolchain that GPUI needs.
Push-Location $root
try {
    $buildOptions = @('build', '--locked', '--target-dir', $TargetDirectory)
    if ($Release) { $buildOptions += '--release' }
    & $cargo.Source @buildOptions
    if ($LASTEXITCODE -ne 0) { throw "Falló la compilación de Cut ($LASTEXITCODE)." }
    Get-Item -LiteralPath (Join-Path (Join-Path $TargetDirectory $profile) 'Cut.exe')
} finally {
    Pop-Location
}
