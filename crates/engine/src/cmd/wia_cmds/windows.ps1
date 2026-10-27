# WIA discovery/settings plus Windows' current scanner API for acquisition. Legacy Automation
# Item.Transfer can return E_ACCESSDENIED on working drivers (including the Epson V500).
# The Rust caller runs this static
# script in an STA process; request JSON and the private output directory arrive as data.
# WIA documentation: https://learn.microsoft.com/en-us/previous-versions/windows/desktop/wiaaut/
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

function Get-WiaProperty($Properties, [int] $Id) {
    # Properties.Item(number) uses a collection position, not a WIA PropertyID.
    foreach ($property in $Properties) {
        if ([int] $property.PropertyID -eq $Id) { return $property }
    }
    return $null
}

function Get-WiaValue($Properties, [int] $Id) {
    $property = Get-WiaProperty $Properties $Id
    if ($null -ne $property) { return $property.Value }
    return $null
}

function Initialize-ScannerApi {
    Add-Type -AssemblyName System.Runtime.WindowsRuntime
    $null = [Windows.Devices.Scanners.ImageScanner,Windows.Devices.Scanners,ContentType=WindowsRuntime]
    $null = [Windows.Devices.Scanners.ImageScannerScanResult,Windows.Devices.Scanners,ContentType=WindowsRuntime]
    $null = [Windows.Storage.StorageFolder,Windows.Storage,ContentType=WindowsRuntime]
}

function Await-Scanner($Operation, [Type]$ResultType, [Type]$ProgressType = $null) {
    $count = 1
    $name = 'IAsyncOperation`1'
    if ($null -ne $ProgressType) { $count = 2; $name = 'IAsyncOperationWithProgress`2' }
    $method = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
        $_.Name -eq 'AsTask' -and $_.IsGenericMethod -and
        $_.GetGenericArguments().Length -eq $count -and $_.GetParameters().Length -eq 1 -and
        $_.GetParameters()[0].ParameterType.Name -eq $name
    } | Select-Object -First 1
    if ($null -ne $ProgressType) { $generic = $method.MakeGenericMethod($ResultType, $ProgressType) }
    else { $generic = $method.MakeGenericMethod($ResultType) }
    $task = $generic.Invoke($null, @($Operation))
    $task.GetAwaiter().GetResult()
}

function Select-ScannerFormat($Configuration) {
    foreach ($name in @('Tiff', 'Png', 'DeviceIndependentBitmap', 'Jpeg')) {
        $format = [Enum]::Parse([Windows.Devices.Scanners.ImageScannerFormat], $name)
        if ($Configuration.IsFormatSupported($format)) { return $format }
    }
    throw 'The scanner supports no TIFF, PNG, BMP, or JPEG transfer format.'
}

function Write-Result($Result) {
    $path = [System.IO.Path]::Combine($env:PHOTOCRAFT_WIA_OUTPUT_DIR, 'result.json')
    $json = ConvertTo-Json -InputObject $Result -Depth 8 -Compress
    [System.IO.File]::WriteAllText($path, $json, (New-Object System.Text.UTF8Encoding($false)))
}

function Cancel-Acquisition($Result) {
    $Result.cancelled = $true
    Write-Result $Result
}

$result = [ordered]@{ available = $true; devices = @(); acquired = $false; cancelled = $false }
$stage = 'initializing WIA'
try {
    $request = ConvertFrom-Json -InputObject $env:PHOTOCRAFT_WIA_REQUEST
    $stage = 'creating the WIA device manager'
    $manager = New-Object -ComObject WIA.DeviceManager
    $scannerInfos = @()
    $devices = @()
    $stage = 'enumerating WIA scanners'
    foreach ($info in $manager.DeviceInfos) {
        if ([int] $info.Type -ne 1) { continue } # ScannerDeviceType
        $id = [string] $info.DeviceID
        $name = [string] (Get-WiaValue $info.Properties 7) # WIA_DIP_DEV_NAME
        if ([string]::IsNullOrWhiteSpace($name)) { $name = $id }
        $devices += [ordered]@{
            id = $id
            name = $name
            manufacturer = [string] (Get-WiaValue $info.Properties 3)
            description = [string] (Get-WiaValue $info.Properties 4)
            type = 'scanner'
        }
        $scannerInfos += $info
    }
    $result.devices = @($devices)
    if ($request.action -eq 'list') {
        Write-Result $result
        exit 0
    }
    if ($devices.Count -eq 0) {
        throw 'No WIA scanner was found. Install the scanner Windows WIA driver (a TWAIN-only driver is insufficient), turn on the scanner, connect USB, and check the Windows Image Acquisition (WIA) service.'
    }
    $intent = switch ($request.mode) {
        'color' { 1 } # ColorIntent
        'gray'  { 2 } # GrayscaleIntent
        'text'  { 4 } # TextIntent
        default { throw 'Unknown WIA color mode.' }
    }
    $dialog = $null
    if ($request.showUI) { $dialog = New-Object -ComObject WIA.CommonDialog }
    $device = $null
    $selected = $null
    if (![string]::IsNullOrWhiteSpace([string] $request.deviceId)) {
        foreach ($info in $scannerInfos) {
            if ([string]::Equals([string] $info.DeviceID, [string] $request.deviceId, [System.StringComparison]::Ordinal)) {
                $selected = $info
                if ($request.showUI) {
                    $stage = 'connecting to the scanner'
                    $device = $info.Connect()
                }
                break
            }
        }
        if ($null -eq $selected) { throw 'The selected WIA scanner is unavailable. List devices again, check USB, and ensure no other application is using the scanner.' }
    } elseif ($request.showUI) {
        $stage = 'selecting a scanner'
        $device = $dialog.ShowSelectDevice(1, $true, $false)
        if ($null -eq $device) { Cancel-Acquisition $result; exit 0 }
    } else {
        throw 'A deviceId is required when showUI is false.'
    }
    $item = $null
    $items = $null
    $dpiX = $request.dpi
    $dpiY = $request.dpi
    $colorMode = $request.mode
    $region = $null
    if ($request.showUI) {
        # SingleSelect, UseCommonUI, CancelError. Quality bias does not override chosen settings.
        $stage = 'opening scanner settings'
        $items = $dialog.ShowSelectItems($device, $intent, 131072, $true, $true, $false)
        if ($null -eq $items -or $items.Count -eq 0) { Cancel-Acquisition $result; exit 0 }
        $item = $items.Item(1)
        $deviceId = [string]$device.DeviceID
        $deviceName = [string](Get-WiaValue $device.Properties 7)
        $pnpId = [string](Get-WiaValue $device.Properties 16)
        $uiDpiX = [double](Get-WiaValue $item.Properties 6147)
        $uiDpiY = [double](Get-WiaValue $item.Properties 6148)
        if ($null -eq $dpiX) { $dpiX = $uiDpiX; $dpiY = $uiDpiY }
        $colorMode = switch ([int](Get-WiaValue $item.Properties 4103)) {
            0 { 'text' }; 1 { 'text' }; 2 { 'gray' }; default { 'color' }
        }
        # WIA's selected rectangle is in pixels; the current API uses inches. Capture the
        # physical area before applying any DPI override so changing resolution cannot crop it.
        if ($uiDpiX -gt 0 -and $uiDpiY -gt 0) {
            $region = @(
                ([double](Get-WiaValue $item.Properties 6149) / $uiDpiX),
                ([double](Get-WiaValue $item.Properties 6150) / $uiDpiY),
                ([double](Get-WiaValue $item.Properties 6151) / $uiDpiX),
                ([double](Get-WiaValue $item.Properties 6152) / $uiDpiY)
            )
        }
    } else {
        $deviceId = [string]$selected.DeviceID
        $deviceName = [string](Get-WiaValue $selected.Properties 7)
        $pnpId = [string](Get-WiaValue $selected.Properties 16)
    }
    if ([string]::IsNullOrWhiteSpace($pnpId)) { throw 'The WIA driver did not expose its scanner PnP ID.' }
    # Release the settings connection before the scan API opens the same hardware.
    foreach ($object in @($item, $items, $device, $dialog)) {
        if ($null -ne $object -and [Runtime.InteropServices.Marshal]::IsComObject($object)) {
            [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($object)
        }
    }
    $item = $null; $items = $null; $device = $null; $dialog = $null
    $selected = $null; $info = $null; $scannerInfos = @(); $manager = $null
    [GC]::Collect()
    [GC]::WaitForPendingFinalizers()
    $stage = 'opening the Windows scanner API'
    Initialize-ScannerApi
    $scanner = Await-Scanner ([Windows.Devices.Scanners.ImageScanner]::FromIdAsync($pnpId)) ([Windows.Devices.Scanners.ImageScanner])
    $source = [Windows.Devices.Scanners.ImageScannerScanSource]::Flatbed
    if (!$scanner.IsScanSourceSupported($source)) { throw 'This import currently requires a flatbed WIA scanner.' }
    $configuration = $scanner.FlatbedConfiguration
    $stage = 'configuring the scan'
    $mode = switch ($colorMode) {
        'gray' { [Windows.Devices.Scanners.ImageScannerColorMode]::Grayscale }
        'text' { [Windows.Devices.Scanners.ImageScannerColorMode]::Monochrome }
        default { [Windows.Devices.Scanners.ImageScannerColorMode]::Color }
    }
    if (!$configuration.IsColorModeSupported($mode)) { throw "The scanner does not support $colorMode mode." }
    $stage = 'setting the scan color mode'
    if ($configuration.ColorMode -ne $mode) { $configuration.ColorMode = $mode }
    if ($null -ne $dpiX) {
        $stage = 'setting the scan resolution'
        $resolution = $configuration.DesiredResolution
        $resolution.DpiX = $dpiX
        $resolution.DpiY = $dpiY
        $configuration.DesiredResolution = $resolution
        $actual = $configuration.ActualResolution
        if ($actual.DpiX -ne $dpiX -or $actual.DpiY -ne $dpiY) { throw "The scanner did not accept $dpiX x $dpiY DPI. Choose a supported resolution." }
    }
    if ($null -ne $region -and $region[2] -gt 0 -and $region[3] -gt 0) {
        $stage = 'setting the selected scan area'
        $rectangle = $configuration.SelectedScanRegion
        # WIA extents are integer pixels and can round the platen edge outward; clamp to the
        # current API's physical bed dimensions (e.g. 11.7 vs 11.6929 inches on the V500).
        $maximum = $configuration.MaxScanArea
        $rectangle.X = [Math]::Max(0, [Math]::Min($region[0], $maximum.Width))
        $rectangle.Y = [Math]::Max(0, [Math]::Min($region[1], $maximum.Height))
        $rectangle.Width = [Math]::Min($region[2], $maximum.Width - $rectangle.X)
        $rectangle.Height = [Math]::Min($region[3], $maximum.Height - $rectangle.Y)
        if ($configuration.AutoCroppingMode -ne [Windows.Devices.Scanners.ImageScannerAutoCroppingMode]::Disabled) {
            $configuration.AutoCroppingMode = [Windows.Devices.Scanners.ImageScannerAutoCroppingMode]::Disabled
        }
        $configuration.SelectedScanRegion = $rectangle
    }
    $stage = 'selecting a supported scanner image format'
    $configuration.Format = Select-ScannerFormat $configuration
    $folder = Await-Scanner ([Windows.Storage.StorageFolder]::GetFolderFromPathAsync($env:PHOTOCRAFT_WIA_OUTPUT_DIR)) ([Windows.Storage.StorageFolder])
    $stage = 'transferring the scanned image'
    $scanned = Await-Scanner ($scanner.ScanFilesToFolderAsync($source, $folder)) ([Windows.Devices.Scanners.ImageScannerScanResult]) ([UInt32])
    $files = @($scanned.ScannedFiles)
    if ($files.Count -ne 1) { throw 'The flatbed scanner returned no image or more than one image.' }
    $file = $files[0]
    $stage = 'reading the acquired image format'
    $extension = ([string]$file.FileType).TrimStart('.').ToLowerInvariant()
    if ($extension -eq 'dib') { $extension = 'bmp' }
    if (@('bmp', 'png', 'tif', 'tiff', 'jpg', 'jpeg') -notcontains $extension) {
        throw "The WIA scanner returned unsupported image format '$extension'. Select TIFF, BMP, PNG, or JPEG in the scanner settings."
    }
    $imagePath = [System.IO.Path]::Combine($env:PHOTOCRAFT_WIA_OUTPUT_DIR, 'scan.bin')
    $stage = 'saving the acquired image'
    [System.IO.File]::Move($file.Path, $imagePath)
    $stage = 'reading the acquired image metadata'
    $result.acquired = $true
    $result.deviceId = $deviceId
    $result.deviceName = $deviceName
    $result.extension = $extension
    $result.format = [string]$configuration.Format
    $result.dpi = $configuration.ActualResolution.DpiX
    $result.dpiY = $configuration.ActualResolution.DpiY
    $stage = 'writing the acquisition result'
    Write-Result $result
    exit 0
} catch {
    # PowerShell wraps COM errors; walk to the inner exception to retain the driver HRESULT.
    $exception = $_.Exception
    while ($null -ne $exception.InnerException) { $exception = $exception.InnerException }
    $hrBytes = [BitConverter]::GetBytes([int] $exception.HResult)
    $hr = [BitConverter]::ToUInt32($hrBytes, 0)
    $hresult = '0x{0:X8}' -f $hr
    # Dialog cancellation normally returns null with CancelError=false. Also recognize generic
    # Win32 cancellation and the cancellation HRESULT used by some WIA Automation drivers.
    if ($hresult -eq '0x80210064' -or $hresult -eq '0x800704C7') {
        Cancel-Acquisition $result
        exit 0
    }
    $hint = switch ($hresult) {
        '0x80210006' { 'Close other scanner applications and try again.' } # WIA_ERROR_BUSY
        '0x80210005' { 'Check the USB connection and turn on the scanner.' } # WIA_ERROR_OFFLINE
        '0x8021000A' { 'Check the scanner USB connection and restart the Windows Image Acquisition (WIA) service.' }
        '0x8021000B' { 'The scanner does not support this WIA command. Try the scanner settings dialog or install its full WIA driver.' }
        '0x8021000E' { 'The scanner driver failed. Install or update its Windows WIA driver.' }
        '0x8021000F' { 'The scanner driver returned an invalid response. Install or update its Windows WIA driver.' }
        '0x80040154' { 'Windows WIA Automation is unavailable. Check the Windows Image Acquisition (WIA) service and repair the Windows scanner components.' }
        default { 'Check the Windows WIA driver, USB connection, Windows Image Acquisition (WIA) service, and whether another application is using the scanner.' }
    }
    $result.error = [ordered]@{ message = "${stage}: $($exception.Message) $hint"; hresult = $hresult }
    Write-Result $result
    exit 1
}
