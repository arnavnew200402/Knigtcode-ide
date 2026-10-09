# Extract only unlabelled landscape/knight regions. All headings, controls,
# project names and dates are rendered by native GPUI elements.
param([Parameter(Mandatory = $true)][string]$ReferenceImage)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$image = [System.Drawing.Bitmap]::FromFile((Resolve-Path $ReferenceImage).Path)
try {
    if ($image.Width -ne 1584 -or $image.Height -ne 993) { throw 'Expected the 1584x993 Home reference.' }
    $assets = Join-Path $PSScriptRoot '../crates/knightcode_onboarding/assets'
    New-Item -ItemType Directory -Force $assets | Out-Null
    $scene = [System.Drawing.Bitmap]::new(1580, 928, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $graphics = [System.Drawing.Graphics]::FromImage($scene)
        try {
            $graphics.Clear([System.Drawing.Color]::FromArgb(3, 9, 19))
            $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            # Clean landscape left of the reference heading; stretch this
            # texture horizontally instead of copying any typography.
            $graphics.DrawImage($image, [System.Drawing.Rectangle]::new(0, 0, 722, 438),
                [System.Drawing.Rectangle]::new(6, 65, 138, 438), [System.Drawing.GraphicsUnit]::Pixel)
            # Knight and landscape to the right of all heading text. Stop
            # before the reference's action cards begin at y=505.
            $graphics.DrawImage($image, [System.Drawing.Rectangle]::new(722, 0, 858, 438),
                [System.Drawing.Rectangle]::new(724, 65, 858, 438), [System.Drawing.GraphicsUnit]::Pixel)
            # The unlabelled bottom landscape reconstructs the area behind the
            # native action/recent cards. No screenshot controls are retained.
            $graphics.DrawImage($image, [System.Drawing.Rectangle]::new(0, 438, 1580, 490),
                [System.Drawing.Rectangle]::new(6, 874, 1572, 108), [System.Drawing.GraphicsUnit]::Pixel)
            $shade = [System.Drawing.Drawing2D.LinearGradientBrush]::new(
                [System.Drawing.Rectangle]::new(0, 0, 760, 438),
                [System.Drawing.Color]::FromArgb(145, 0, 4, 10),
                [System.Drawing.Color]::FromArgb(0, 0, 4, 10), 0.0)
            try { $graphics.FillRectangle($shade, 0, 0, 760, 438) } finally { $shade.Dispose() }
        } finally { $graphics.Dispose() }
        # Feather reconstructed areas; no straight seams behind native UI.
        for ($y = 0; $y -lt 438; $y++) {
            $edge = $scene.GetPixel(721, $y)
            for ($x = 722; $x -lt 782; $x++) {
                $t = ($x - 722) / 59.0
                $t = $t * $t * (3.0 - 2.0 * $t)
                $pixel = $scene.GetPixel($x, $y)
                $scene.SetPixel($x, $y, [System.Drawing.Color]::FromArgb(255,
                    [int]($edge.R * (1-$t) + $pixel.R * $t),
                    [int]($edge.G * (1-$t) + $pixel.G * $t),
                    [int]($edge.B * (1-$t) + $pixel.B * $t)))
            }
        }
        for ($x = 0; $x -lt 1580; $x++) {
            $edge = $scene.GetPixel($x, 437)
            for ($y = 438; $y -lt 490; $y++) {
                $t = ($y - 438) / 51.0
                $t = $t * $t * (3.0 - 2.0 * $t)
                $pixel = $scene.GetPixel($x, $y)
                $scene.SetPixel($x, $y, [System.Drawing.Color]::FromArgb(255,
                    [int]($edge.R * (1-$t) + $pixel.R * $t),
                    [int]($edge.G * (1-$t) + $pixel.G * $t),
                    [int]($edge.B * (1-$t) + $pixel.B * $t)))
            }
        }
        $scene.Save((Join-Path $assets 'home-landscape.png'), [System.Drawing.Imaging.ImageFormat]::Png)
    } finally { $scene.Dispose() }
    $texture = $image.Clone([System.Drawing.Rectangle]::new(6, 874, 1572, 108), [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try { $texture.Save((Join-Path $assets 'home-card-texture.png'), [System.Drawing.Imaging.ImageFormat]::Png) }
    finally { $texture.Dispose() }
    Write-Output 'Prepared Home artwork without screenshot headings, project names, dates or controls.'
} finally { $image.Dispose() }
