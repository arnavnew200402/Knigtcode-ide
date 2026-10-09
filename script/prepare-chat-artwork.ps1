# Extract only artwork from the supplied 1536x1024 Chat design reference.
# UI text and controls are rendered natively; none are baked into these assets.
param([Parameter(Mandatory = $true)][string]$ReferenceImage)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
public static class KnightCodeChatArtwork {
    public static void Extract(string source, string directory) {
        using (var reference = new Bitmap(source)) {
            if (reference.Width != 1536 || reference.Height != 1024)
                throw new ArgumentException("Expected the 1536x1024 Chat design reference");
            using (var scenery = reference.Clone(new Rectangle(260, 70, 1276, 747), PixelFormat.Format32bppArgb))
            using (var original = (Bitmap)scenery.Clone()) {
                // Reconstruct the central sky underneath the reference's hero
                // and greeting using its unoccupied right-hand starfield.
                for (int y = 60; y < 625; y++) {
                    for (int x = 380; x < 945; x++) {
                        double edge = Math.Min(Math.Min(x - 380, 944 - x), Math.Min(y - 60, 624 - y));
                        double alpha = Math.Min(1, Math.Max(0, edge / 60));
                        var a = original.GetPixel(x, y);
                        var b = original.GetPixel(950 + (x - 380) * 230 / 565, 30 + (y - 60) * 300 / 565);
                        scenery.SetPixel(x, y, Color.FromArgb(255,
                            (int)(a.R * (1 - alpha) + b.R * alpha),
                            (int)(a.G * (1 - alpha) + b.G * alpha),
                            (int)(a.B * (1 - alpha) + b.B * alpha)));
                    }
                }
                scenery.Save(System.IO.Path.Combine(directory, "chat-landscape.png"), ImageFormat.Png);
            }
            using (var hero = reference.Clone(new Rectangle(716, 166, 420, 309), PixelFormat.Format32bppArgb)) {
                // Feather the crop into the live sky rather than showing a
                // rectangular screenshot panel around the knight artwork.
                for (int y = 0; y < hero.Height; y++) {
                    for (int x = 0; x < hero.Width; x++) {
                        double edge = Math.Min(Math.Min(x, hero.Width - 1 - x), Math.Min(y, hero.Height - 1 - y));
                        double alpha = Math.Min(1, Math.Max(0, edge / 28));
                        var c = hero.GetPixel(x, y);
                        hero.SetPixel(x, y, Color.FromArgb((int)(255 * alpha), c.R, c.G, c.B));
                    }
                }
                hero.Save(System.IO.Path.Combine(directory, "chat-knight.png"), ImageFormat.Png);
            }
        }
    }
}
'@
$output = Join-Path $PSScriptRoot '../crates/agent_ui/assets'
New-Item -ItemType Directory -Force $output | Out-Null
[KnightCodeChatArtwork]::Extract((Resolve-Path $ReferenceImage).Path, (Resolve-Path $output).Path)
Write-Output 'Extracted Chat landscape and knight artwork (without screenshot UI).'
