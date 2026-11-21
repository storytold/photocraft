param([Parameter(Mandatory=$true)][string] $FixtureRoot)
. (Join-Path $PSScriptRoot 'nightly.ps1') -FunctionsOnly
function Assert([bool] $Value, [string] $Message) { if (-not $Value) { throw $Message } }
$spec = Get-NightlyArchiveSpec '0.23.1-nightly.20261120.17.1.gabcdef012345' 'x64'
Assert ($spec.Archive -eq 'photocraft-0.23.1-nightly.20261120.17.1.gabcdef012345-windows-x86_64-nightly.zip') 'wrong nightly artifact name'
Assert ($spec.Executables.Count -eq 1 -and $spec.Executables[0] -eq 'photocraft.exe') 'nightly must not ship the unqualified CLI'
Assert ($spec.AppId -eq 'ai.storyteller.photocraft.nightly-v1') 'nightly identity is not distinct'
foreach ($arch in @('x86', 'arm64')) {
  $refused = $false
  try { Get-NightlyArchiveSpec '0.23.1' $arch | Out-Null } catch { $refused = $true }
  Assert $refused 'unsupported target was accepted'
}
foreach ($bad in @('', '0.23.1', '../other', '0.23.1/elsewhere')) {
  $refused = $false
  try { Get-NightlyArchiveSpec $bad 'x64' | Out-Null } catch { $refused = $true }
  Assert $refused 'unsafe artifact version was accepted'
}
Assert-NightlyFlavor 'PhotoCraft Nightly' 'PhotoCraft 0.23.1-nightly.20261120.17.1.gabcdef012345 (nightly-v1)' '0.23.1-nightly.20261120.17.1.gabcdef012345'
foreach ($pair in @(@('PhotoCraft', 'PhotoCraft 0.23.1 (nightly-v1)'), @('PhotoCraft Nightly', 'PhotoCraft 0.23.1'))) {
  $refused = $false
  try { Assert-NightlyFlavor $pair[0] $pair[1] '0.23.1-nightly.20261120.17.1.gabcdef012345' } catch { $refused = $true }
  Assert $refused 'stable or mismatched build flavor was accepted'
}
Write-Output 'nightly packaging spec, target/version refusal and flavor-binding tests passed'

# Exercise the SAME production notice copier with the real repository layout, then inspect a ZIP.
if (-not $FixtureRoot -or -not [IO.Path]::IsPathRooted($FixtureRoot)) { throw 'supply an owned absolute -FixtureRoot for the notices/ZIP regression' }
$testRoot = Join-Path $FixtureRoot ("nightly-notices-" + [Guid]::NewGuid().ToString('N'))
try {
  $repo = Join-Path $testRoot 'app'
  $fontRepo = Join-Path $testRoot 'craft-fonts'
  $family = Join-Path $fontRepo 'fonts\ExampleFamily'
  $stage = Join-Path $testRoot 'stage'
  New-Item -ItemType Directory -Force -Path $repo, $family, $stage | Out-Null
  $rootNames = @('README.md', 'LICENSE', 'LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE', 'ATTRIBUTION.md')
  foreach ($name in $rootNames) { Set-Content -LiteralPath (Join-Path $repo $name) -Value "root-notice-$name" }
  Set-Content -LiteralPath (Join-Path $family 'OFL.txt') -Value 'font-notice-example'
  Copy-NightlyNotices $repo $fontRepo $stage
  foreach ($name in @($rootNames) + @('OFL-ExampleFamily.txt')) {
    Assert (Test-Path -LiteralPath (Join-Path $stage $name)) "missing staged notice $name"
  }
  $zip = Join-Path $testRoot 'notices.zip'
  Compress-Archive -Path (Join-Path $stage '*') -DestinationPath $zip
  $archive = [IO.Compression.ZipFile]::OpenRead($zip)
  try {
    $names = @($archive.Entries | ForEach-Object { $_.FullName })
    foreach ($name in @($rootNames) + @('OFL-ExampleFamily.txt')) { Assert ($name -in $names) "missing archived notice $name" }
    foreach ($forbidden in @('portable.txt', 'photocraft-cli.exe')) { Assert ($forbidden -notin $names) "unexpected payload $forbidden" }
    $entry = $archive.GetEntry('OFL-ExampleFamily.txt')
    $reader = [IO.StreamReader]::new($entry.Open())
    try { Assert ($reader.ReadToEnd().Contains('font-notice-example')) 'wrong font license bytes in ZIP' } finally { $reader.Dispose() }
  } finally { $archive.Dispose() }
} finally {
  if (Test-Path -LiteralPath $testRoot) { Remove-Item -LiteralPath $testRoot -Recurse -Force }
}
Write-Output 'nightly mock-repository root and font notices are present in stage and ZIP'
