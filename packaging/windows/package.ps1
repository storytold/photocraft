<#
.SYNOPSIS
  Build, sign and package PhotoCraft for Windows.

.DESCRIPTION
  Produces, in $env:DIST (default: dist/release):
    photocraft-<version>-windows-<arch>.msi            per-machine installer (WiX v5)
    photocraft-<version>-windows-<arch>-portable.zip   photocraft.exe + portable.txt
    photocraft-cli-<version>-windows-<arch>.zip        separate signed command-line executable

  The binaries link the C runtime statically (+crt-static), so neither the MSI nor the portable
  zip needs the Visual C++ redistributable. Signing is delegated to sign.ps1 (skipped with a
  warning when no signing secrets are set).

  Needs: Rust (MSVC toolchain + the target), the Windows SDK (rc.exe, signtool.exe),
  and WiX v5: dotnet tool install --global wix --version 5.0.2

.EXAMPLE
  pwsh packaging/windows/package.ps1 -Arch x64
  pwsh packaging/windows/package.ps1 -Arch x86 -SkipBuild
  pwsh packaging/windows/package.ps1 -Arch arm64     # cross-compiled; needs the MSVC ARM64 build tools
#>
param(
  [ValidateSet('x64', 'x86', 'arm64')] [string] $Arch = 'x64',
  [switch] $SkipBuild
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

function Invoke-Native([string] $What, [scriptblock] $Block) {
  Write-Output "==> $What"
  & $Block
  if ($LASTEXITCODE -ne 0) { throw "$What failed with exit code $LASTEXITCODE" }
}

# The version lives in one place: [workspace.package] version in the root Cargo.toml.
$Version = $env:PHOTOCRAFT_VERSION
if (-not $Version) {
  $inPkg = $false
  foreach ($line in Get-Content (Join-Path $Root 'Cargo.toml')) {
    if ($line -match '^\s*\[') { $inPkg = ($line.Trim() -eq '[workspace.package]'); continue }
    if ($inPkg -and $line -match '^\s*version\s*=\s*"([^"]+)"') { $Version = $Matches[1]; break }
  }
}
if (-not $Version) { throw 'could not read [workspace.package] version from Cargo.toml' }
# MSI ProductVersion is numeric (major.minor.build); pre-release tags are dropped there.
$MsiVersion = ($Version -split '-')[0]

$Target = switch ($Arch) { 'x64' { 'x86_64-pc-windows-msvc' } 'x86' { 'i686-pc-windows-msvc' } 'arm64' { 'aarch64-pc-windows-msvc' } }
$Dist = if ($env:DIST) { $env:DIST } else { Join-Path $Root 'dist\release' }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
New-Item -ItemType Directory -Force -Path $Dist | Out-Null

if (-not $env:PHOTOCRAFT_BUILD_SHA) { $env:PHOTOCRAFT_BUILD_SHA = (git -C $Root rev-parse HEAD 2>$null) }
if (-not $env:PHOTOCRAFT_BUILD_DATE) { $env:PHOTOCRAFT_BUILD_DATE = (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd') }

Write-Output "PhotoCraft $Version for Windows $Arch ($Target)"

if (-not $SkipBuild) {
  # Static CRT: no VC++ redistributable needed. Scoped to the target so host build scripts and
  # proc-macros are unaffected.
  $flagVar = 'CARGO_TARGET_' + ($Target.ToUpper() -replace '-', '_') + '_RUSTFLAGS'
  [Environment]::SetEnvironmentVariable($flagVar, '-C target-feature=+crt-static')
  # Fail the build (rather than warn) if the icon/VERSIONINFO can't be embedded.
  $env:PHOTOCRAFT_REQUIRE_WINRES = '1'
  Invoke-Native "cargo build ($Target)" { cargo build --profile native-release --locked -p photocraft -p photocraft-cli --features heif --target $Target }
}

$Bin = Join-Path $TargetDir "$Target\native-release"

# Check both binaries' PE headers before packaging. Machine (COFF header) must match -Arch, so an
# x64 build can never ship labelled arm64. Subsystem (optional header): 2 = Windows GUI, 3 = console.
# The app must be GUI (no console window opens with it); the CLI must stay console so its output
# reaches the terminal.
function Get-PeHeader([string] $Path) {
  $bytes = [System.IO.File]::ReadAllBytes($Path)
  $pe = [BitConverter]::ToInt32($bytes, 0x3C)
  return @{ Machine = [BitConverter]::ToUInt16($bytes, $pe + 4); Subsystem = [BitConverter]::ToUInt16($bytes, $pe + 0x5C) }
}
$Machine = switch ($Arch) { 'x64' { 0x8664 } 'x86' { 0x14C } 'arm64' { 0xAA64 } }
foreach ($check in @(@('photocraft.exe', 2), @('photocraft-cli.exe', 3))) {
  $h = Get-PeHeader (Join-Path $Bin $check[0])
  if ($h.Machine -ne $Machine) { throw "$($check[0]) is for machine 0x$('{0:X}' -f $h.Machine), expected 0x$('{0:X}' -f $Machine) ($Arch)" }
  if ($h.Subsystem -ne $check[1]) { throw "$($check[0]) has PE subsystem $($h.Subsystem), expected $($check[1])" }
  Write-Output "ok $($check[0]): $Arch, PE subsystem $($h.Subsystem)"
}
# Keep matching compiler PDBs privately under target, never in a public package.
$Diagnostics = Join-Path $TargetDir "windows-diagnostics\$Version\$Arch"
Remove-Item -Recurse -Force $Diagnostics -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Diagnostics | Out-Null
foreach ($binary in 'photocraft', 'photocraft-cli') {
  # Cargo/toolchain versions may expose the CLI PDB with a hyphen or crate-name underscore.
  $Candidates = @("$binary.pdb", (($binary -replace '-', '_') + '.pdb')) | Select-Object -Unique
  $Pdb = $Candidates | ForEach-Object { Join-Path $Bin $_ } | Where-Object { Test-Path $_ -PathType Leaf } | Select-Object -First 1
  if (-not $Pdb) { throw "missing native-release PDB for $binary in $Bin" }
  Copy-Item $Pdb $Diagnostics
}

$Stage = Join-Path $TargetDir "windows-package\$Arch"
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Copy-Item (Join-Path $Bin 'photocraft.exe'), (Join-Path $Bin 'photocraft-cli.exe') $Stage

& (Join-Path $PSScriptRoot 'sign.ps1') (Join-Path $Stage 'photocraft.exe') (Join-Path $Stage 'photocraft-cli.exe')

# Stage one authoritative set of OFL notices for the MSI and both archives.
$FontLicenses = Join-Path $Stage 'font-licenses'
New-Item -ItemType Directory -Force -Path $FontLicenses | Out-Null
if ($env:CRAFT_FONTS_DIR) {
  $FontRoot = Join-Path $env:CRAFT_FONTS_DIR 'fonts'
  if (-not (Test-Path $FontRoot -PathType Container)) { throw "missing craft-fonts directory: $FontRoot" }
  foreach ($fontDir in Get-ChildItem $FontRoot -Directory | Sort-Object Name) {
    $lic = Join-Path $fontDir.FullName 'OFL.txt'
    if (Test-Path $lic -PathType Leaf) { Copy-Item $lic (Join-Path $FontLicenses "OFL-$($fontDir.Name).txt") }
  }
}
$LicenseFiles = @(Get-ChildItem $FontLicenses -File | Sort-Object Name)
if ($env:CRAFT_FONTS_DIR -and $LicenseFiles.Count -eq 0) { throw 'craft-fonts was configured but no OFL notices were found' }
$FontLicenseArgs = @()
if ($LicenseFiles.Count -gt 0) {
  # Explicit WiX File elements avoid wildcard harvesting extensions. The conditional include
  # contributes components directly to PhotocraftFiles; no-font development builds omit it.
  $IncludePath = Join-Path $Stage 'font-licenses.wxi'
  $Xml = [System.Text.StringBuilder]::new()
  [void]$Xml.AppendLine('<Include xmlns="http://wixtoolset.org/schemas/v4/wxs">')
  for ($i = 0; $i -lt $LicenseFiles.Count; $i++) {
    $Source = [System.Security.SecurityElement]::Escape($LicenseFiles[$i].FullName)
    [void]$Xml.AppendLine("<Component Id=`"CraftFontLicense$i`" Guid=`"*`"><File Id=`"CraftFontNotice$i`" Source=`"$Source`" KeyPath=`"yes`" /></Component>")
  }
  [void]$Xml.AppendLine('</Include>')
  [IO.File]::WriteAllText($IncludePath, $Xml.ToString(), [System.Text.UTF8Encoding]::new($false))
  $FontLicenseArgs = @('-d', "CraftFontLicenses=$IncludePath")
}

# ---- MSI ---------------------------------------------------------------------------------------
$Msi = Join-Path $Dist "photocraft-$Version-windows-$Arch.msi"
& (Join-Path $PSScriptRoot 'check-icons.ps1')
Invoke-Native 'wix build' {
  wix build (Join-Path $PSScriptRoot 'photocraft.wxs') -arch $Arch `
    -d "Version=$MsiVersion" -d "BinDir=$Stage" -d "IconPath=$(Join-Path $Root 'assets\app-icon\photocraft.ico')" `
    @FontLicenseArgs -o $Msi
}
Invoke-Native 'MSI shortcut icon validation (ICE50)' {
  wix msi validate $Msi -ice ICE50 -intermediateFolder (Join-Path $Stage 'msi-validation')
}
# wix writes its debug symbols (.wixpdb) next to the MSI; keep them out of the release assets.
Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($Msi, '.wixpdb'))
& (Join-Path $PSScriptRoot 'sign.ps1') $Msi

# ---- portable zip ------------------------------------------------------------------------------
$Portable = Join-Path $TargetDir "windows-package\photocraft-$Version-windows-$Arch-portable"
Remove-Item -Recurse -Force $Portable -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Portable | Out-Null
Copy-Item (Join-Path $Stage 'photocraft.exe') $Portable
foreach ($f in 'README.md', 'LICENSE', 'LICENSE-MIT', 'LICENSE-APACHE') {
  $p = Join-Path $Root $f
  if (Test-Path $p) { Copy-Item $p $Portable }
}
foreach ($lic in $LicenseFiles) { Copy-Item $lic.FullName $Portable }
# portable.txt beside photocraft.exe switches on portable mode: settings, presets and recovery
# files go to PhotoCraftData\ next to the exe instead of %APPDATA% (#228; see app_dirs.rs).
Copy-Item (Join-Path $PSScriptRoot 'portable.txt') $Portable
$Zip = Join-Path $Dist "photocraft-$Version-windows-$Arch-portable.zip"
Remove-Item -Force $Zip -ErrorAction SilentlyContinue
Compress-Archive -Path $Portable -DestinationPath $Zip -CompressionLevel Optimal

# ---- separate CLI zip --------------------------------------------------------------------------
$CliPortable = Join-Path $TargetDir "windows-package\photocraft-cli-$Version-windows-$Arch"
Remove-Item -Recurse -Force $CliPortable -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $CliPortable | Out-Null
Copy-Item (Join-Path $Stage 'photocraft-cli.exe') $CliPortable
foreach ($f in 'README.md', 'LICENSE', 'LICENSE-MIT', 'LICENSE-APACHE') {
  $p = Join-Path $Root $f
  if (Test-Path $p) { Copy-Item $p $CliPortable }
}
foreach ($lic in $LicenseFiles) { Copy-Item $lic.FullName $CliPortable }
$CliZip = Join-Path $Dist "photocraft-cli-$Version-windows-$Arch.zip"
Remove-Item -Force $CliZip -ErrorAction SilentlyContinue
Compress-Archive -Path $CliPortable -DestinationPath $CliZip -CompressionLevel Optimal

# Smoke-test the CLI when this machine can run it. An ARM64 build made on an x64 runner can't run
# here; .github/workflows/windows-arm64.yml installs and runs it on ARM64 instead.
$HostArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
if ($Arch -ne 'arm64' -or $HostArch -eq 'arm64') {
  Invoke-Native 'photocraft-cli --version' { & (Join-Path $Stage 'photocraft-cli.exe') --version }
} else {
  Write-Output "skipping photocraft-cli --version: an $Arch build doesn't run on this $HostArch machine"
}
Get-Item $Msi, $Zip, $CliZip | Format-Table Name, Length
