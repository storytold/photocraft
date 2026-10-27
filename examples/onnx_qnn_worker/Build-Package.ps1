# CI-only validation draft: FULL self-contained publish, NO redistributable ZIP in this revision.
# Legal asset presence/integrity is not legal approval. See LEGAL-REVIEW.md.
[CmdletBinding()]
param(
    [ValidateSet('Release', 'Debug')][string]$Configuration = 'Release',
    [string]$OutputDirectory = './artifacts'
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'Packaging.ps1')
$version = '1.24.4'
$project = Join-Path $PSScriptRoot 'PhotoCraft.OnnxQnn.csproj'
$output = [IO.Path]::GetFullPath($OutputDirectory)
$publish = Join-Path $output 'publish-qnn-arm64'
$packages = Join-Path $output 'restore-packages'
$evidence = Join-Path $output 'qnn-evidence'
New-Item -ItemType Directory -Force $evidence | Out-Null
# Refuse to overwrite/delete earlier evidence or binaries; use a fresh output directory per run.
if ((Test-Path $publish) -or (Test-Path $packages)) { throw 'Publish/restore output already exists; use a fresh output directory.' }
$report = [ordered]@{
    scope = 'T4 ARM64 publish/inventory only; no model, NPU, quality or redistribution approval'
    ort_version = $version
    runtime_zip_created = $false
    redistribution = 'BLOCKED: Qualcomm PDF/privacy/third-party and .NET terms have not been reviewed'
    validation = 'started'
    limits = @{ files = 512; per_file_bytes = 268435456; aggregate_bytes = 536870912; manifest_bytes = 1048576 }
    commit = $env:GITHUB_SHA
    run_url = if ($env:GITHUB_RUN_ID) { "$env:GITHUB_SERVER_URL/$env:GITHUB_REPOSITORY/actions/runs/$env:GITHUB_RUN_ID" } else { $null }
}
function Write-JsonFile($Value, [string]$Path) {
    [IO.File]::WriteAllText($Path, ($Value | ConvertTo-Json -Depth 30), [Text.UTF8Encoding]::new($false))
}
function Copy-Verified([string]$Source, [string]$Destination) {
    if (-not (Test-Path -LiteralPath $Source -PathType Leaf) -or (Get-Item -LiteralPath $Source).Length -eq 0) {
        throw "Required source asset missing/empty: $Source"
    }
    New-Item -ItemType Directory -Force (Split-Path -Parent $Destination) | Out-Null
    Copy-Item -LiteralPath $Source -Destination $Destination -Force
    if ((Get-FileHash -LiteralPath $Source -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash) {
        throw 'Copied runtime/legal asset differs from its restored source.'
    }
}
try {
    # SDK audit warnings are errors via the project; audit every transitive dependency.
    # RID/SelfContained/PlatformTarget are deliberately identical for restore and --no-restore publish.
    dotnet restore $project --runtime win-arm64 --packages $packages -p:SelfContained=true -p:PlatformTarget=ARM64 -p:NuGetAudit=true -p:NuGetAuditMode=all
    if ($LASTEXITCODE -ne 0) { throw 'ARM64 restore/audit failed.' }
    dotnet publish $project --no-restore --configuration $Configuration --runtime win-arm64 --self-contained true `
        -p:PlatformTarget=ARM64 -p:RestorePackagesPath=$packages -p:PublishSingleFile=false -p:PublishTrimmed=false -p:PublishReadyToRun=false --output $publish
    if ($LASTEXITCODE -ne 0) { throw 'ARM64 full publish failed.' }

    $packageEvidence = @()
    foreach ($id in @('microsoft.ml.onnxruntime.qnn', 'microsoft.ml.onnxruntime.managed')) {
        $root = Join-Path $packages "$id/$version"
        $archive = Join-Path $root "$id.$version.nupkg"
        if (-not (Test-Path -LiteralPath $archive -PathType Leaf)) { throw 'Pinned package archive is missing from restore.' }
        $archiveHash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($id -eq 'microsoft.ml.onnxruntime.qnn' -and $archiveHash -cne 'e4d6eabb9e503d4f3c78494fc9400f02509b2ee315d9f707644a174ece8da17f') {
            throw 'Pinned QNN archive differs from the read-only reviewed upstream archive.'
        }
        dotnet nuget verify $archive --all
        if ($LASTEXITCODE -ne 0) { throw 'NuGet signature verification failed.' }
        $packageEvidence += [ordered]@{
            id = $id; version = $version; bytes = (Get-Item -LiteralPath $archive).Length
            sha256 = $archiveHash
            nuget_signature = 'verified by dotnet nuget verify --all'
        }
    }
    Write-JsonFile $packageEvidence (Join-Path $evidence 'nuget-packages.json')

    $qnnRoot = Join-Path $packages "microsoft.ml.onnxruntime.qnn/$version"
    $nativeRoot = Join-Path $qnnRoot 'runtimes/win-arm64/native'
    $requiredNative = @(
        'onnxruntime.dll', 'onnxruntime_providers_shared.dll', 'onnxruntime_providers_qnn.dll',
        'QnnCpu.dll', 'QnnGpu.dll', 'QnnHtp.dll', 'QnnHtpPrepare.dll', 'QnnSystem.dll',
        'QnnHtpV73Stub.dll', 'QnnHtpV81Stub.dll', 'libQnnHtpV73Skel.so', 'libQnnHtpV81Skel.so',
        'libqnnhtpv73.cat', 'libqnnhtpv81.cat', 'onnxruntime.lib'
    )
    $nativeAssets = @(Get-ChildItem -LiteralPath $nativeRoot -File | Sort-Object Name)
    if ($nativeAssets.Count -ne $requiredNative.Count) { throw 'Pinned native inventory changed: review all assets before continuing.' }
    $nativeEvidence = @()
    foreach ($name in $requiredNative) {
        $source = Join-Path $nativeRoot $name
        $destination = Join-Path $publish $name
        # Explicitly copy ALL source native assets; NuGet's props do not copy HTP DSP/CAT files.
        Copy-Verified $source $destination
        $machine = $null
        if ($name.EndsWith('.dll')) {
            $machine = Get-PeMachine $destination
            if ($machine -notin @(0xaa64, 0xa64e)) { throw "Native asset is not ARM64/ARM64X: $name" }
        }
        $nativeEvidence += [ordered]@{ path = $name; bytes = (Get-Item $destination).Length; sha256 = (Get-FileHash $destination).Hash.ToLowerInvariant(); pe_machine = $machine }
    }
    Write-JsonFile $nativeEvidence (Join-Path $evidence 'native-dsp-inventory.json')
    if ((Get-PeMachine (Join-Path $publish 'photocraft-onnx-qnn.exe')) -ne 0xaa64) { throw 'Worker apphost is not native ARM64.' }
    foreach ($name in @('photocraft-onnx-qnn.dll', 'photocraft-onnx-qnn.runtimeconfig.json', 'photocraft-onnx-qnn.deps.json', 'coreclr.dll', 'hostfxr.dll', 'hostpolicy.dll', 'System.Private.CoreLib.dll')) {
        if (-not (Test-Path (Join-Path $publish $name) -PathType Leaf)) { throw "Incomplete self-contained runtime: $name" }
    }
    $runtimeConfig = Get-Content -LiteralPath (Join-Path $publish 'photocraft-onnx-qnn.runtimeconfig.json') -Raw | ConvertFrom-Json -AsHashtable
    if ($runtimeConfig.runtimeOptions.ContainsKey('framework') -or $runtimeConfig.runtimeOptions.ContainsKey('frameworks')) { throw 'Publish unexpectedly depends on an installed .NET shared framework.' }

    # Copy real upstream legal files verbatim, not a locally invented MIT notice.
    $requiredLegal = @{
        'microsoft.ml.onnxruntime.qnn' = @('LICENSE', 'Privacy.md', 'Qualcomm_LICENSE.pdf', 'ThirdPartyNotices.txt')
        'microsoft.ml.onnxruntime.managed' = @('LICENSE.txt', 'Privacy.md', 'ThirdPartyNotices.txt')
    }
    foreach ($id in $requiredLegal.Keys) {
        foreach ($name in $requiredLegal[$id]) {
            Copy-Verified (Join-Path $packages "$id/$version/$name") (Join-Path $publish "notices/$id/$version/$name")
        }
    }
    $legalEvidence = @()
    # Includes transitive managed dependencies and the self-contained .NET runtime/host packs.
    foreach ($idDir in Get-ChildItem -LiteralPath $packages -Directory) {
        foreach ($versionDir in Get-ChildItem -LiteralPath $idDir.FullName -Directory) {
            $legalFiles = @(Get-ChildItem -LiteralPath $versionDir.FullName -File | Where-Object Name -Match '(?i)(license|licence|notice|privacy|copying)')
            foreach ($asset in $legalFiles) {
                $relative = "notices/$($idDir.Name)/$($versionDir.Name)/$($asset.Name)"
                Copy-Verified $asset.FullName (Join-Path $publish $relative)
                $legalEvidence += [ordered]@{ path = $relative; bytes = $asset.Length; sha256 = (Get-FileHash -LiteralPath $asset.FullName).Hash.ToLowerInvariant() }
            }
        }
    }
    if (@($legalEvidence | Where-Object path -Match '^notices/microsoft.netcore.app.runtime.win-arm64/').Count -lt 2) {
        throw 'Missing self-contained .NET runtime LICENSE/third-party notices; review restore layout.'
    }
    Write-JsonFile $legalEvidence (Join-Path $evidence 'legal-inventory.json')
    foreach ($doc in @('README.md', 'LEGAL-REVIEW.md', 'UPSTREAM-VERIFICATION.md')) {
        Copy-Verified (Join-Path $PSScriptRoot $doc) (Join-Path $publish "docs/$doc")
    }
    # No runnable model, no fabricated hashes/shapes, no model download and no v2 resource store.
    [IO.File]::WriteAllText((Join-Path $publish 'qnn-models.json'), '{"schemaVersion":1,"defaultModel":null,"profiles":[]}', [Text.UTF8Encoding]::new($false))

    $entries = @(Get-ChildItem -LiteralPath $publish -Recurse -Force -File | ForEach-Object {
        if ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Reparse point in publish payload.' }
        [ordered]@{ path = [IO.Path]::GetRelativePath($publish, $_.FullName).Replace('\', '/'); bytes = $_.Length; sha256 = (Get-FileHash -LiteralPath $_.FullName).Hash.ToLowerInvariant() }
    } | Sort-Object { $_.path })
    Write-JsonFile $entries (Join-Path $evidence 'publish-inventory.json')
    $total = Assert-PackageLimits -Entries $entries
    $manifest = New-AdapterManifest -Files @($entries | ForEach-Object { $_.path })
    $manifestJson = $manifest | ConvertTo-Json -Depth 30
    if ([Text.Encoding]::UTF8.GetByteCount($manifestJson) -gt 1MB) { throw 'Host manifest byte limit exceeded.' }
    [IO.File]::WriteAllText((Join-Path $publish 'adapter.json'), $manifestJson, [Text.UTF8Encoding]::new($false))
    # Manifest is intentionally separate from files: the host snapshots it itself (not self-hashed).
    Copy-Verified (Join-Path $publish 'adapter.json') (Join-Path $evidence 'adapter.json')
    $report.validation = 'passed'
    $report.file_count = $entries.Count
    $report.aggregate_bytes = $total
    $report.manifest_bytes = [Text.Encoding]::UTF8.GetByteCount($manifestJson)
    Write-Host "Complete ARM64 publish validated ($($entries.Count) files, $total bytes). Runtime redistribution remains BLOCKED; no ZIP created."
}
catch {
    $report.validation = 'failed'
    $report.failure = $_.Exception.Message
    throw
}
finally {
    Write-JsonFile $report (Join-Path $evidence 'validation-report.json')
}
# HARD-CLOSED LEGAL RELEASE GATE: no ZIP path, approval flag, release action or binary upload.
# A future reviewed change must record exact license digests, permitted scope and reviewer decision
# before adding any distribution path. An inventory or a green CI job is not that decision.
