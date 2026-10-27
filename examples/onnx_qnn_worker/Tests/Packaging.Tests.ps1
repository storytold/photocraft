# Pure policy/manifest and PowerShell syntax assertions. No dotnet, native DLL or worker process.
param([Parameter(Mandatory)][string]$ResultsPath)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot '../Packaging.ps1')
$results = [Collections.Generic.List[object]]::new()
function Test([string]$Name, [scriptblock]$Body) {
    try { & $Body; $results.Add(@{ name = $Name; passed = $true }); Write-Host "PASS $Name" }
    catch { $results.Add(@{ name = $Name; passed = $false; error = $_.Exception.Message }); Write-Host "FAIL $Name : $_" }
}
function Require([bool]$Condition) { if (-not $Condition) { throw 'Assertion failed.' } }
function Reject([scriptblock]$Body) { $rejected = $false; try { & $Body | Out-Null } catch { $rejected = $true }; Require $rejected }
Test 'policy/path-and-duplicate-reserved' {
    foreach ($path in @('../x', '/x', 'C:/x', 'a\x', 'a//x', '.hidden', 'record.json', 'CON.txt', 'x./y')) {
        Reject { Assert-PackageLimits @(@{ path = $path; bytes = 1 }) }
    }
    Reject { Assert-PackageLimits @(@{ path = 'a'; bytes = 1 }, @{ path = 'A'; bytes = 1 }) }
}
Test 'policy/per-file-aggregate-and-file-count-boundaries' {
    Require ((Assert-PackageLimits @(@{ path = 'a'; bytes = 256MB }, @{ path = 'b'; bytes = 256MB })) -eq 512MB)
    Reject { Assert-PackageLimits @(@{ path = 'a'; bytes = 256MB + 1 }) }
    Reject { Assert-PackageLimits @(@{ path = 'a'; bytes = 256MB }, @{ path = 'b'; bytes = 256MB }, @{ path = 'c'; bytes = 1 }) }
    Reject { Assert-PackageLimits @(@{ path = 'a'; bytes = [long]::MaxValue }) }
    $files = @(1..512 | ForEach-Object { @{ path = "f$_"; bytes = 0 } })
    Require ((Assert-PackageLimits $files) -eq 0)
    Reject { Assert-PackageLimits @($files + @{ path = 'f513'; bytes = 0 }) }
    Reject { Assert-PackageLimits @() }
}
Test 'manifest/current-host-structure-files-include-docs-and-notices' {
    $files = @('photocraft-onnx-qnn.exe', 'docs/README.md', 'notices/qnn/Qualcomm_LICENSE.pdf')
    $m = New-AdapterManifest $files
    Require (($m.Keys | Sort-Object) -join ',' -eq 'descriptor,files,launch,package_version,timeout_ms')
    Require ($m.package_version -eq 1 -and $m.descriptor.api_version -eq 1 -and $m.launch.kind -eq 'executable')
    Require ($m.launch.path -in $m.files -and $m.files.Count -eq 3 -and $m.timeout_ms -le 300000)
    Require ($m.descriptor.devices[0].backend -eq 'qnn' -and $m.descriptor.operations[0].output_kind -eq 'mask')
    $roundtrip = $m | ConvertTo-Json -Depth 30 | ConvertFrom-Json -AsHashtable
    Require ($roundtrip.files[1] -is [string] -and $roundtrip.files[2] -eq $files[2])
}
Test 'scripts/parse-every-powershell-source' {
    foreach ($file in Get-ChildItem (Join-Path $PSScriptRoot '..') -Recurse -Filter '*.ps1') {
        $tokens = $null; $errors = $null
        [Management.Automation.Language.Parser]::ParseFile($file.FullName, [ref]$tokens, [ref]$errors) | Out-Null
        if ($errors.Count -ne 0) { throw "$($file.Name): $($errors -join '; ')" }
    }
}
$full = [IO.Path]::GetFullPath($ResultsPath)
New-Item -ItemType Directory -Force (Split-Path -Parent $full) | Out-Null
@{ scope = 'T1 pure packaging-policy and script syntax; no builds, native loading, process or NPU inference'; tests = @($results.ToArray()); failed = @($results | Where-Object { -not $_.passed }).Count } |
    ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $full -Encoding utf8NoBOM
if (@($results | Where-Object { -not $_.passed }).Count -gt 0) { throw 'Packaging pure tests failed.' }
