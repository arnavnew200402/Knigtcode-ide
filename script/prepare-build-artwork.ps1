# Extract artwork only from the supplied Build reference; all controls remain native.
param([Parameter(Mandatory = $true)][string]$ReferenceImage)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$image = [System.Drawing.Bitmap]::FromFile((Resolve-Path $ReferenceImage).Path)
try {
    if ($image.Width -ne 1584 -or $image.Height -ne 993) { throw 'Expected the 1584x993 Build reference.' }
    $assets = Join-Path $PSScriptRoot '../crates/knightcode_onboarding/assets'
    $agentAssets = Join-Path $PSScriptRoot '../crates/agent_ui/assets'
    $projectAssets = Join-Path $PSScriptRoot '../crates/project_panel/assets'
    New-Item -ItemType Directory -Force $assets, $agentAssets, $projectAssets | Out-Null
    foreach ($crop in @(
        @{ Path = (Join-Path $assets 'build-knight.png'); Rect = [System.Drawing.Rectangle]::new(862, 60, 254, 568) },
        @{ Path = (Join-Path $agentAssets 'build-agent-knight.png'); Rect = [System.Drawing.Rectangle]::new(1306, 312, 78, 99) },
        @{ Path = (Join-Path $projectAssets 'build-sidebar.png'); Rect = [System.Drawing.Rectangle]::new(64, 500, 240, 354) }
    )) {
        $bitmap = $image.Clone($crop.Rect, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
        try {
            if ($crop.Path -like '*build-knight.png') {
                # Blend the artwork into the native canvas; do not include the
                # reference's heading at the crop's left edge.
                for ($x = 0; $x -lt 8; $x++) {
                    $t = $x / 7.0
                    $opacity = $t * $t * (3.0 - 2.0 * $t)
                    for ($y = 0; $y -lt $bitmap.Height; $y++) {
                        $pixel = $bitmap.GetPixel($x, $y)
                        $bitmap.SetPixel($x, $y, [System.Drawing.Color]::FromArgb([int][Math]::Round($pixel.A * $opacity), $pixel.R, $pixel.G, $pixel.B))
                    }
                }
            }
            $bitmap.Save($crop.Path, [System.Drawing.Imaging.ImageFormat]::Png)
        } finally { $bitmap.Dispose() }
    }
    Write-Output 'Extracted Build knight artwork without UI text or controls.'
} finally { $image.Dispose() }
