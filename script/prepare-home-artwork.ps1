# Install a clean Home background verbatim. Native GPUI draws every heading,
# button and project card; do not extract, stretch or composite screenshot UI.
param([Parameter(Mandatory = $true)][Alias('ReferenceImage')][string]$BackgroundImage)
$ErrorActionPreference = 'Stop'
$source = (Resolve-Path $BackgroundImage).Path
Add-Type -AssemblyName System.Drawing
$image = [System.Drawing.Image]::FromFile($source)
try {
    if ($image.RawFormat.Guid -ne [System.Drawing.Imaging.ImageFormat]::Png.Guid) {
        throw 'Supply a PNG background.'
    }
    if ($image.Width -ne 2 * $image.Height) {
        throw 'Supply a 2:1 landscape background (for example, 2560x1280).'
    }
    $dimensions = "$($image.Width)x$($image.Height)"
} finally { $image.Dispose() }
$assets = Join-Path $PSScriptRoot '../crates/knightcode_onboarding/assets'
New-Item -ItemType Directory -Force $assets | Out-Null
$destination = Join-Path $assets 'home-landscape.png'
if ([IO.Path]::GetFullPath($source) -ne [IO.Path]::GetFullPath($destination)) {
    Copy-Item -LiteralPath $source -Destination $destination -Force
}
if ((Get-FileHash $source).Hash -ne (Get-FileHash $destination).Hash) {
    throw 'Background copy verification failed.'
}
Write-Output "Installed the original $dimensions background without resampling or compositing."
