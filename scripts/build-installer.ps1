[CmdletBinding()]
param(
    [string]$TargetDirectory = 'target',
    [string]$OutputDirectory = 'target/installer',
    [string]$CompilerPath
)

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
$TargetDirectory = [IO.Path]::GetFullPath($TargetDirectory, $root)
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory, $root)
$toolDirectory = Join-Path $env:LOCALAPPDATA 'CutClipper/build-tools'

if (!$CompilerPath) {
    $CompilerPath = (Get-Command ISCC.exe -ErrorAction SilentlyContinue).Source
    if (!$CompilerPath) {
        $CompilerPath = @(
            (Join-Path $toolDirectory 'InnoSetup/ISCC.exe')
            (Join-Path $env:ProgramFiles 'Inno Setup 7/ISCC.exe')
            (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6/ISCC.exe')
            (Join-Path $env:LOCALAPPDATA 'Programs/Inno Setup 6/ISCC.exe')
        ) | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
    }
}
if (!$CompilerPath -or !(Test-Path -LiteralPath $CompilerPath -PathType Leaf)) {
    throw 'Instala Inno Setup 6.3 o posterior, o indica la ruta a ISCC.exe con -CompilerPath.'
}

$cargo = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (!$cargo) { $cargo = Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe' }
$metadata = & $cargo metadata --locked --no-deps --format-version 1 --manifest-path (Join-Path $root 'Cargo.toml')
if ($LASTEXITCODE -ne 0) { throw 'No se pudo leer la versión de Cut.' }
$version = ($metadata | ConvertFrom-Json).packages[0].version

& (Join-Path $PSScriptRoot 'build-windows.ps1') -Release -TargetDirectory $TargetDirectory | Out-Host

New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$buildDirectory = Join-Path $TargetDirectory 'release'
& $CompilerPath "/DAppVersion=$version" "/DBuildDirectory=$buildDirectory" `
    "/O$OutputDirectory" (Join-Path $PSScriptRoot 'installer/Cut.iss')
if ($LASTEXITCODE -ne 0) { throw "Inno Setup no pudo crear el instalador ($LASTEXITCODE)." }
Get-Item -LiteralPath (Join-Path $OutputDirectory "Cut-$version-Setup-x64.exe")
