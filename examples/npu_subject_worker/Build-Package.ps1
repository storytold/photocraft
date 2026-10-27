#requires -Version 7.2
[CmdletBinding(DefaultParameterSetName = 'Unsigned')]
param(
    [Parameter(ParameterSetName = 'Unsigned')][switch]$Unsigned,
    [Parameter(Mandatory, ParameterSetName = 'Development')][switch]$DevelopmentCertificate,
    [Parameter(Mandatory, ParameterSetName = 'Pfx')][string]$CertificatePath,
    [Parameter(ParameterSetName = 'Pfx')][string]$CertificatePasswordEnvironment = 'PHOTOOCRAFT_SIGNING_PASSWORD',
    [string]$Publisher = 'CN=PhotoCraft Companion Development',
    [string]$PackageVersion = '0.1.0.0',
    [string]$OutputDirectory = (Join-Path $PSScriptRoot '.artifacts'),
    [string]$TimestampUrl
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
if (!$IsWindows) { throw 'Build/package this Windows ARM64 companion on a Windows cloud runner.' }
if ($PackageVersion -notmatch '^\d+\.\d+\.\d+\.\d+$') { throw 'PackageVersion must have four numeric components.' }
$version = [version]$PackageVersion
if ($version.Major -gt 65535 -or $version.Minor -gt 65535 -or $version.Build -gt 65535 -or $version.Revision -gt 65535) {
    throw 'MSIX version components must fit u16.'
}
$root = [IO.Path]::GetFullPath($OutputDirectory)
[IO.Directory]::CreateDirectory($root) | Out-Null
# Unique staging paths; never clean a caller-supplied directory recursively.
$stage = Join-Path $root ('build-' + [guid]::NewGuid().ToString('N'))
$payload = Join-Path $stage 'payload'
[IO.Directory]::CreateDirectory($payload) | Out-Null
$package = Join-Path $root "photocraft-windows-ai-arm64-$PackageVersion.msix"
if (Test-Path -LiteralPath $package) { throw "Output already exists: $package" }
$project = Join-Path $PSScriptRoot 'PhotoCraft.WindowsAI.csproj'
Push-Location $PSScriptRoot
try {
    & dotnet publish $project -c Release -r win-arm64 --self-contained true -p:Platform=ARM64 --output $payload
    if ($LASTEXITCODE -ne 0) { throw 'ARM64 dotnet publish failed.' }
} finally { Pop-Location }

$toolsPackage = (Get-Content -LiteralPath (Join-Path $payload 'windows-sdk-tools.txt') -Raw).Trim()
# All tools are from the pinned Microsoft.Windows.SDK.BuildTools NuGet package.
$tools = Join-Path $toolsPackage 'bin\10.0.28000.0\x64'
$makeappx = Join-Path $tools 'makeappx.exe'
$signtool = Join-Path $tools 'signtool.exe'
if (!(Test-Path -LiteralPath $makeappx)) { throw "Pinned SDK MakeAppx not found: $makeappx" }
if (!(Test-Path -LiteralPath $signtool)) { throw "Pinned SDK SignTool not found: $signtool" }

[xml]$manifest = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'Package.appxmanifest') -Raw
$ns = [Xml.XmlNamespaceManager]::new($manifest.NameTable)
$foundation = 'http://schemas.microsoft.com/appx/manifest/foundation/windows10'
$ns.AddNamespace('m', $foundation)
$identity = $manifest.SelectSingleNode('/m:Package/m:Identity', $ns)
$identity.SetAttribute('Publisher', $Publisher)
$identity.SetAttribute('Version', $PackageVersion)
# App-local WinAppSDK requires its WinRT registrations as well as its DLLs. Merge
# the exact fragments emitted by the restored SDK (not handwritten class names).
$extensions = $manifest.CreateElement('Extensions', $foundation)
$capabilities = $manifest.SelectSingleNode('/m:Package/m:Capabilities', $ns)
$manifest.DocumentElement.InsertBefore($extensions, $capabilities) | Out-Null
$fragmentPaths = @(Get-Content -LiteralPath (Join-Path $payload 'windowsappsdk-fragments.txt') | Where-Object { $_.Trim() })
if ($fragmentPaths.Count -eq 0) { throw 'SDK emitted no app-local activation fragments.' }
$hasExtractor = $false
foreach ($path in $fragmentPaths) {
    [xml]$fragment = Get-Content -LiteralPath $path -Raw
    $fragmentNs = [Xml.XmlNamespaceManager]::new($fragment.NameTable)
    $fragmentNs.AddNamespace('m', $foundation)
    foreach ($extension in $fragment.SelectNodes('/m:Fragment/m:Extensions/m:Extension', $fragmentNs)) {
        $category = $extension.GetAttribute('Category')
        if ($category -notin @('windows.activatableClass.inProcessServer', 'windows.activatableClass.proxyStub')) { continue }
        $dll = $extension.SelectSingleNode('.//m:Path', $fragmentNs)
        # SDK's own self-contained manifest generator excludes these singleton/widget proxies.
        if ($null -ne $dll -and $dll.InnerText -in @('PushNotificationsLongRunningTask.ProxyStub.dll', 'Microsoft.Windows.Widgets.dll')) { continue }
        if ($null -ne $dll -and !(Test-Path -LiteralPath (Join-Path $payload $dll.InnerText))) {
            throw "Missing app-local SDK component: $($dll.InnerText)"
        }
        if ($extension.OuterXml.Contains('Microsoft.Windows.AI.Imaging.ImageForegroundExtractor')) { $hasExtractor = $true }
        $extensions.AppendChild($manifest.ImportNode($extension, $true)) | Out-Null
    }
}
$aiFiles = @(Get-ChildItem -Path $payload -Filter "*AI*.dll")
if ($hasExtractor -or $aiFiles.Count -gt 0 -or (Test-Path (Join-Path $payload 'photocraft-windows-ai.dll'))) {
    Write-Host "Confirmed Windows AI companion components in payload: $($aiFiles.Name -join ', ')"
} else {
    throw 'No Windows AI companion components found in published payload.'
}
$manifest.Save((Join-Path $payload 'AppxManifest.xml'))

# Original, procedurally generated geometric package logos: no bundled assets/fonts.
Add-Type -AssemblyName System.Drawing
$assets = Join-Path $payload 'Assets'
[IO.Directory]::CreateDirectory($assets) | Out-Null
foreach ($entry in @(@('StoreLogo.png',50), @('Square44x44Logo.png',44), @('Square150x150Logo.png',150))) {
    $side = [int]$entry[1]
    $bitmap = [Drawing.Bitmap]::new($side, $side)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $brush = [Drawing.SolidBrush]::new([Drawing.Color]::FromArgb(255, 104, 211, 190))
    try {
        $graphics.Clear([Drawing.Color]::FromArgb(255, 25, 38, 53))
        $graphics.FillEllipse($brush, [int]($side*.25), [int]($side*.14), [int]($side*.5), [int]($side*.5))
        $graphics.FillRectangle($brush, [int]($side*.14), [int]($side*.65), [int]($side*.72), [int]($side*.2))
        $bitmap.Save((Join-Path $assets $entry[0]), [Drawing.Imaging.ImageFormat]::Png)
    } finally { $brush.Dispose(); $graphics.Dispose(); $bitmap.Dispose() }
}
# Include applicable upstream SDK redistribution notices, not its development tools.
$licenses = Join-Path $payload 'ThirdPartyNotices'
[IO.Directory]::CreateDirectory($licenses) | Out-Null
foreach ($path in $fragmentPaths) {
    $componentRoot = Split-Path (Split-Path $path)
    $license = Join-Path $componentRoot 'license.txt'
    if (Test-Path -LiteralPath $license) {
        $label = Split-Path (Split-Path $componentRoot) -Leaf
        Copy-Item -LiteralPath $license -Destination (Join-Path $licenses ($label + '.txt'))
    }
}
# These text files contain build-machine NuGet paths and are not runtime payload.
foreach ($name in @('windowsappsdk-fragments.txt', 'windows-sdk-tools.txt')) {
    $file = [IO.Path]::GetFullPath((Join-Path $payload $name))
    if (![string]::Equals([IO.Path]::GetDirectoryName($file), $payload, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unexpected cleanup path.' }
    Remove-Item -LiteralPath $file
}
& $makeappx pack /d $payload /p $package /o
if ($LASTEXITCODE -ne 0) { throw 'MakeAppx validation/package creation failed.' }

$signingPfx = $null
$temporaryPfx = $null
$certificate = $null
$password = ''
try {
    if ($PSCmdlet.ParameterSetName -eq 'Development') {
        # In-memory self-signed key: NO machine/user certificate store is modified.
        $rsa = [Security.Cryptography.RSA]::Create(3072)
        try {
            $request = [Security.Cryptography.X509Certificates.CertificateRequest]::new($Publisher, $rsa,
                [Security.Cryptography.HashAlgorithmName]::SHA256, [Security.Cryptography.RSASignaturePadding]::Pkcs1)
            $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($false, $false, 0, $true))
            $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509KeyUsageExtension]::new([Security.Cryptography.X509Certificates.X509KeyUsageFlags]::DigitalSignature, $true))
            $eku = [Security.Cryptography.OidCollection]::new()
            $eku.Add([Security.Cryptography.Oid]::new('1.3.6.1.5.5.7.3.3')) | Out-Null
            $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509EnhancedKeyUsageExtension]::new($eku, $true))
            $certificate = $request.CreateSelfSigned([DateTimeOffset]::UtcNow.AddMinutes(-5), [DateTimeOffset]::UtcNow.AddMonths(3))
            $password = [guid]::NewGuid().ToString('N')
            $temporaryPfx = Join-Path $stage 'temporary-signing-key.pfx'
            [IO.File]::WriteAllBytes($temporaryPfx, $certificate.Export([Security.Cryptography.X509Certificates.X509ContentType]::Pfx, $password))
            $signingPfx = $temporaryPfx
        } finally { $rsa.Dispose() }
    } elseif ($PSCmdlet.ParameterSetName -eq 'Pfx') {
        $signingPfx = [IO.Path]::GetFullPath($CertificatePath)
        $password = [Environment]::GetEnvironmentVariable($CertificatePasswordEnvironment)
        if ($null -eq $password) { throw "Missing certificate password environment variable: $CertificatePasswordEnvironment" }
        $certificate = [Security.Cryptography.X509Certificates.X509Certificate2]::new($signingPfx, $password,
            [Security.Cryptography.X509Certificates.X509KeyStorageFlags]::EphemeralKeySet)
        if ($certificate.Subject -ne $Publisher) { throw 'Signing certificate subject must match the manifest Publisher exactly.' }
    }
    if ($null -ne $signingPfx) {
        $signArgs = @('sign', '/fd', 'SHA256', '/f', $signingPfx, '/p', $password)
        if ($TimestampUrl) { $signArgs += @('/tr', $TimestampUrl, '/td', 'SHA256') }
        $signArgs += $package
        & $signtool @signArgs
        if ($LASTEXITCODE -ne 0) { throw 'SignTool failed.' }
        [IO.File]::WriteAllBytes((Join-Path $root 'photocraft-windows-ai.cer'),
            $certificate.Export([Security.Cryptography.X509Certificates.X509ContentType]::Cert))
        Write-Host "Signer thumbprint: $($certificate.Thumbprint)"
        Write-Host 'Signing does not trust or install this certificate. Review Install-Package.ps1 on the target PC.'
    } else {
        Write-Warning 'UNSIGNED MSIX: not installable by the normal sideload installer; sign it before distribution.'
    }
} finally {
    if ($null -ne $certificate) { $certificate.Dispose() }
    if ($null -ne $temporaryPfx -and (Test-Path -LiteralPath $temporaryPfx)) {
        $checked = [IO.Path]::GetFullPath($temporaryPfx)
        if (![string]::Equals([IO.Path]::GetDirectoryName($checked), $stage, [StringComparison]::OrdinalIgnoreCase)) { throw 'Unexpected key cleanup path.' }
        Remove-Item -LiteralPath $checked
    }
}
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'adapter.json') -Destination (Join-Path $root 'adapter.json')
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'Install-Package.ps1') -Destination (Join-Path $root 'Install-Package.ps1')
Get-FileHash -LiteralPath $package -Algorithm SHA256 | Format-List
Write-Host "ARM64 package: $package"
Write-Host 'No package, AI model, signing trust or system policy was installed/changed by this script.'
