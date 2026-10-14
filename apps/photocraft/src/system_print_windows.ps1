$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object System.Text.UTF8Encoding($false)
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Data.Pdf.PdfDocument, Windows.Data.Pdf, ContentType = WindowsRuntime]
$null = [Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime]

$extensions = [System.WindowsRuntimeSystemExtensions]
$operationTask = $extensions.GetMethods() | Where-Object {
    $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and
    $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'
} | Select-Object -First 1
$actionTask = $extensions.GetMethods() | Where-Object {
    $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and
    $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncAction'
} | Select-Object -First 1
function Wait-Operation($operation, $resultType) {
    $task = $operationTask.MakeGenericMethod($resultType).Invoke($null, @($operation))
    $task.Wait()
    $task.Result
}
function Wait-Action($action) {
    $task = $actionTask.Invoke($null, @($action))
    $task.Wait()
}

$source = Wait-Operation ([Windows.Storage.StorageFile]::GetFileFromPathAsync($env:PHOTOCRAFT_PRINT_FILE)) ([Windows.Storage.StorageFile])
$pdf = Wait-Operation ([Windows.Data.Pdf.PdfDocument]::LoadFromFileAsync($source)) ([Windows.Data.Pdf.PdfDocument])
if ($pdf.PageCount -ne 1) { throw 'PhotoCraft expected a one-page print document' }

$page = $pdf.GetPage(0)
try {
    $width = [int][Math]::Max(1, [Math]::Round($page.Size.Width * 100.0 / 96.0))
    $height = [int][Math]::Max(1, [Math]::Round($page.Size.Height * 100.0 / 96.0))
    $renderWidth = [uint32][Math]::Max(1, [Math]::Min(32768, [Math]::Round($page.Size.Width * 3.0)))
    $renderHeight = [uint32][Math]::Max(1, [Math]::Min(32768, [Math]::Round($page.Size.Height * 3.0)))
    $options = [Windows.Data.Pdf.PdfPageRenderOptions]::new()
    $options.DestinationWidth = $renderWidth
    $options.DestinationHeight = $renderHeight
    $stream = [System.IO.MemoryStream]::new()
    try {
        $random = [System.IO.WindowsRuntimeStreamExtensions]::AsRandomAccessStream($stream)
        Wait-Action ($page.RenderToStreamAsync($random, $options))
        $stream.Position = 0
        $image = [System.Drawing.Image]::FromStream($stream)
        $bitmap = [System.Drawing.Bitmap]::new($image)
        $image.Dispose()
    } finally {
        $stream.Dispose()
    }
} finally {
    $page.Dispose()
}

$document = [System.Drawing.Printing.PrintDocument]::new()
$document.DocumentName = $env:PHOTOCRAFT_PRINT_TITLE
$document.OriginAtMargins = $false
$document.DefaultPageSettings.PaperSize = [System.Drawing.Printing.PaperSize]::new('PhotoCraft page', $width, $height)
$document.DefaultPageSettings.Landscape = ($width -gt $height)
$document.DefaultPageSettings.Margins = [System.Drawing.Printing.Margins]::new(0, 0, 0, 0)
$document.add_PrintPage({
    param($sender, $event)
    $box = [System.Drawing.RectangleF]::new(-$event.PageSettings.HardMarginX, -$event.PageSettings.HardMarginY, $width, $height)
    $event.Graphics.DrawImage($bitmap, $box)
    $event.HasMorePages = $false
})

$dialog = [System.Windows.Forms.PrintDialog]::new()
$dialog.UseEXDialog = $true
$dialog.AllowPrintToFile = $true
$dialog.Document = $document
try {
    if ($dialog.ShowDialog() -ne [System.Windows.Forms.DialogResult]::OK) {
        Write-Output 'cancelled'
        return
    }
    $document.PrintController = [System.Drawing.Printing.StandardPrintController]::new()
    $document.Print()
    Write-Output 'printed'
} finally {
    $dialog.Dispose()
    $document.Dispose()
    $bitmap.Dispose()
}
