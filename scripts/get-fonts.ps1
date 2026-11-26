# Downloads the two OFL fonts used by the Cut UI into assets/fonts.
# Run once from the repository root:  powershell -ExecutionPolicy Bypass -File scripts/get-fonts.ps1
$ErrorActionPreference = "Stop"
$dest = Join-Path $PSScriptRoot "..\assets\fonts"
New-Item -ItemType Directory -Force -Path $dest | Out-Null

$files = @{
  # Sin corchetes en el nombre local: Windows PowerShell los trata como comodines.
  "BricolageGrotesque-Variable.ttf" = "https://github.com/google/fonts/raw/main/ofl/bricolagegrotesque/BricolageGrotesque%5Bopsz%2Cwdth%2Cwght%5D.ttf"
  "MartianMono-Variable.ttf"        = "https://github.com/google/fonts/raw/main/ofl/martianmono/MartianMono%5Bwdth%2Cwght%5D.ttf"
}
foreach ($name in $files.Keys) {
  $target = Join-Path $dest $name
  Write-Host "Descargando $name"
  Invoke-WebRequest -Uri $files[$name] -OutFile $target
}

# The SIL OFL asks that the license travel with the fonts; the installer copies these.
$licenses = Join-Path $PSScriptRoot "..\assets\licenses"
@{
  "BricolageGrotesque-LICENSE.txt" = "https://github.com/google/fonts/raw/main/ofl/bricolagegrotesque/OFL.txt"
  "MartianMono-LICENSE.txt"        = "https://github.com/google/fonts/raw/main/ofl/martianmono/OFL.txt"
}.GetEnumerator() | ForEach-Object {
  Write-Host "Descargando $($_.Key)"
  Invoke-WebRequest -Uri $_.Value -OutFile (Join-Path $licenses $_.Key)
}
Write-Host "Listo. Vuelve a compilar: cargo build --release"
