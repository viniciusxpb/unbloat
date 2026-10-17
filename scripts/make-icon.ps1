# Original code-drawn Unbloat icon; no third-party artwork.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$destination = Join-Path (Split-Path $PSScriptRoot) 'assets\unbloat.ico'
$stream = [IO.File]::Create($destination)
$writer = [IO.BinaryWriter]::new($stream)
try {
    $images = @()
    foreach ($size in @(16, 32, 48, 64, 128, 256)) {
        $bitmap = [Drawing.Bitmap]::new($size, $size)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        $graphics.SmoothingMode = [Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $graphics.ScaleTransform($size / 64.0, $size / 64.0)
        $graphics.Clear([Drawing.ColorTranslator]::FromHtml('#12232d'))
        $pen = [Drawing.Pen]::new([Drawing.ColorTranslator]::FromHtml('#53edb5'), 9)
        $graphics.DrawLines($pen, [Drawing.PointF[]]@([Drawing.PointF]::new(21,19),[Drawing.PointF]::new(21,42),[Drawing.PointF]::new(26,47),[Drawing.PointF]::new(38,47),[Drawing.PointF]::new(43,42),[Drawing.PointF]::new(43,19)))
        $brush = [Drawing.SolidBrush]::new([Drawing.ColorTranslator]::FromHtml('#f4f7f8'))
        $graphics.FillPolygon($brush, [Drawing.PointF[]]@([Drawing.PointF]::new(32,4),[Drawing.PointF]::new(22,14),[Drawing.PointF]::new(28,14),[Drawing.PointF]::new(28,24),[Drawing.PointF]::new(36,24),[Drawing.PointF]::new(36,14),[Drawing.PointF]::new(42,14)))
        $png = [IO.MemoryStream]::new()
        $bitmap.Save($png, [Drawing.Imaging.ImageFormat]::Png)
        $images += ,@($size, $png.ToArray())
        $png.Dispose(); $brush.Dispose(); $pen.Dispose(); $graphics.Dispose(); $bitmap.Dispose()
    }
    $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$images.Count)
    $offset = 6 + 16 * $images.Count
    foreach ($item in $images) {
        $dimension = if ($item[0] -eq 256) { 0 } else { $item[0] }
        $writer.Write([byte]$dimension); $writer.Write([byte]$dimension)
        $writer.Write([byte]0); $writer.Write([byte]0); $writer.Write([uint16]1); $writer.Write([uint16]32)
        $writer.Write([uint32]$item[1].Length); $writer.Write([uint32]$offset)
        $offset += $item[1].Length
    }
    foreach ($item in $images) { $writer.Write([byte[]]$item[1]) }
} finally { $writer.Dispose(); $stream.Dispose() }
