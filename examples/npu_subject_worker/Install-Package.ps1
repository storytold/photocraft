<#
.SYNOPSIS
    Installs the PhotoCraft Windows AI companion MSIX package on a Windows 11 Copilot+ PC.

.DESCRIPTION
    The Windows AI companion runs as an external console process registered as an execution alias.
    Because Windows AI requires the package identity and systemAIModels capability, it must be installed
    as a packaged MSIX application. PhotoCraft itself remains a standalone green package.

    This script:
    1. Verifies the OS is Windows 11 ARM64 (Build 26100+ recommended).
    2. Installs the development certificate to the CurrentUser\TrustedPeople store (requires confirmation).
    3. Installs the MSIX package via Add-AppxPackage.
    4. Verifies the execution alias (%LOCALAPPDATA%\Microsoft\WindowsApps\photocraft-windows-ai.exe).
    5. Explains how to register and enable the adapter in PhotoCraft.
#>

[CmdletBinding()]
param(
    [string]$MsixPath,
    [string]$CertificatePath,
    [switch]$SkipCertificateTrust
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Write-Host "=== PhotoCraft Windows AI Companion Installation ===" -ForegroundColor Cyan

# 1. Architecture check
$arch = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture
Write-Host "Detected Architecture: $arch"
if ($arch -ne [System.Runtime.InteropServices.Architecture]::Arm64) {
    Write-Warning "Windows AI foreground extraction is designed for ARM64 Copilot+ PCs with Qualcomm Snapdragon X Elite/Plus NPU."
    Write-Warning "On non-ARM64 systems, the companion can probe but will report 'unavailable'."
}

# 2. Locate MSIX and CER
$scriptDir = $PSScriptRoot
if (-not $MsixPath) {
    $found = Get-ChildItem -Path $scriptDir -Filter "photocraft-windows-ai-arm64-*.msix" -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($found) { $MsixPath = $found.FullName }
}

if (-not $CertificatePath) {
    $cerFound = Get-ChildItem -Path $scriptDir -Filter "photocraft-windows-ai.cer" -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($cerFound) { $CertificatePath = $cerFound.FullName }
}

if (-not $MsixPath -or -not (Test-Path -LiteralPath $MsixPath)) {
    throw "MSIX package not found. Please specify -MsixPath <path-to-msix>."
}

Write-Host "Package: $MsixPath"

# 3. Certificate Trust (if not skipped and certificate provided)
if (-not $SkipCertificateTrust -and $CertificatePath -and (Test-Path -LiteralPath $CertificatePath)) {
    Write-Host "Found certificate: $CertificatePath"
    $cert = [System.Security.Cryptography.X509Certificates.X509Certificate2]::new($CertificatePath)
    Write-Host "Subject:    $($cert.Subject)"
    Write-Host "Thumbprint: $($cert.Thumbprint)"

    $store = [System.Security.Cryptography.X509Certificates.X509Store]::new("TrustedPeople", [System.Security.Cryptography.X509Certificates.StoreLocation]::CurrentUser)
    $store.Open([System.Security.Cryptography.X509Certificates.OpenFlags]::ReadWrite)
    try {
        $alreadyTrusted = $store.Certificates.Find([System.Security.Cryptography.X509Certificates.X509FindType]::FindByThumbprint, $cert.Thumbprint, $false)
        if ($alreadyTrusted.Count -eq 0) {
            Write-Host "`nTo install an unpacked development-signed package, the certificate must be trusted in CurrentUser\TrustedPeople."
            $confirm = Read-Host "Do you want to trust this certificate for CurrentUser? (y/N)"
            if ($confirm -eq 'y' -or $confirm -eq 'Y') {
                $store.Add($cert)
                Write-Host "Certificate installed to CurrentUser\TrustedPeople." -ForegroundColor Green
            } else {
                Write-Warning "Skipped certificate trust. Package installation may fail unless developer mode or enterprise trust is configured."
            }
        } else {
            Write-Host "Certificate is already in CurrentUser\TrustedPeople." -ForegroundColor Green
        }
    } finally {
        $store.Close()
        $cert.Dispose()
    }
}

# 4. Install MSIX package
Write-Host "`nInstalling MSIX package via Add-AppxPackage..."
try {
    Add-AppxPackage -Path $MsixPath
    Write-Host "MSIX package installed successfully." -ForegroundColor Green
} catch {
    Write-Error "Failed to install MSIX package: $_"
    Write-Host "Tip: Enable Developer Mode in Windows Settings (System > For developers) if sideloading is restricted." -ForegroundColor Yellow
    exit 1
}

# 5. Verify AppExecutionAlias
$aliasPath = Join-Path $env:LOCALAPPDATA "Microsoft\WindowsApps\photocraft-windows-ai.exe"
if (Test-Path -LiteralPath $aliasPath) {
    Write-Host "AppExecutionAlias verified: $aliasPath" -ForegroundColor Green
} else {
    Write-Warning "Execution alias not immediately found at $aliasPath. It may take a moment or need terminal restart."
}

# 6. Next Steps
$adapterJson = Join-Path $scriptDir "adapter.json"
Write-Host @"

=== Next Steps in PhotoCraft ===
1. Launch PhotoCraft (from 'D:\ProgramGreen\Craft' or your portable directory).
2. Go to menu: Filter > Plug-ins > Compatibility Adapters…
3. Click 'Browse' or paste the path to adapter manifest:
   $adapterJson
4. Click 'Inspect' to review declared capabilities.
5. Check 'I trust this native adapter package' and click 'Install'.
6. Check 'Enabled' on the installed package row.
7. Click 'Check availability' to probe the Windows AI NPU model status.
   (If it displays 'needs_model', check the consent box and click 'Prepare model' to download system components).
8. Open an image and run: Select > Subject!

"@ -ForegroundColor Cyan
