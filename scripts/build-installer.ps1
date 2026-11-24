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

$downloadDirectory = Join-Path $toolDirectory 'downloads'
$bootstrapper = Join-Path $downloadDirectory 'MicrosoftEdgeWebview2Setup.exe'
if (!(Test-Path -LiteralPath $bootstrapper -PathType Leaf)) {
    New-Item -ItemType Directory -Force -Path $downloadDirectory | Out-Null
    $temporary = Join-Path $downloadDirectory "WebView2-$([guid]::NewGuid()).exe"
    try {
        Invoke-WebRequest -Uri 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' -OutFile $temporary -TimeoutSec 60
        $signature = Get-AuthenticodeSignature -LiteralPath $temporary
        if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notlike '*Microsoft Corporation*') {
            throw 'La firma de la descarga de WebView2 no es válida.'
        }
        Move-Item -LiteralPath $temporary -Destination $bootstrapper -Force
    } finally {
        Remove-Item -LiteralPath $temporary -ErrorAction SilentlyContinue
    }
}
$signature = Get-AuthenticodeSignature -LiteralPath $bootstrapper
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notlike '*Microsoft Corporation*') {
    throw "La firma del instalador de WebView2 no es válida. Revisa $bootstrapper."
}

New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$buildDirectory = Join-Path $TargetDirectory 'release'
& $CompilerPath "/DAppVersion=$version" "/DBuildDirectory=$buildDirectory" "/DBootstrapper=$bootstrapper" `
    "/O$OutputDirectory" (Join-Path $PSScriptRoot 'installer/Cut.iss')
if ($LASTEXITCODE -ne 0) { throw "Inno Setup no pudo crear el instalador ($LASTEXITCODE)." }
Get-Item -LiteralPath (Join-Path $OutputDirectory "Cut-$version-Setup-x64.exe")
