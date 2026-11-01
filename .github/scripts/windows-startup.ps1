param(
    [ValidatePattern('^$|^[0-9a-f]{40}$')]
    [string]$BaselineRef = $env:PHOTOCRAFT_STARTUP_IMPORT_BASELINE
)

$ErrorActionPreference = 'Stop'
$testedHead = (git rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $testedHead -ne $env:EXPECTED_HEAD) {
    throw 'Checkout does not match the requested head commit'
}
$results = Join-Path (Get-Location) 'target/windows-startup-ci'
New-Item -ItemType Directory -Path $results -Force | Out-Null
$testedHead | Set-Content -LiteralPath (Join-Path $results 'tested-head.txt')
"## Windows startup validation`n`nTested commit: ``$testedHead``" >> $env:GITHUB_STEP_SUMMARY

function Invoke-Logged {
    param([string]$Program, [string[]]$Arguments, [string]$Log)
    & $Program @Arguments 2>&1 | Tee-Object -FilePath (Join-Path $results $Log)
    if ($LASTEXITCODE -ne 0) {
        throw "$Program failed with exit code $LASTEXITCODE (see $Log)"
    }
}

Invoke-Logged rustc @('--version', '--verbose') 'rustc.txt'
# The test profile shares dependencies with cargo test, avoiding a second optimized UI build.
# This still links the complete native executable with the normal default feature set.
Invoke-Logged cargo @('build', '--locked', '--profile', 'test', '-p', 'photocraft', '--bin', 'photocraft') 'build.txt'
$executable = Join-Path (Get-Location) 'target/debug/photocraft.exe'
Invoke-Logged $executable @('--version') 'version.txt'
Invoke-Logged cargo @('test', '--locked', '-p', 'photocraft', '--bin', 'photocraft', '--', '--test-threads=1') 'tests.txt'
$testOutput = Get-Content -LiteralPath (Join-Path $results 'tests.txt') -Raw
if ($testOutput -notmatch 'test result: ok\. (\d+) passed; 0 failed; (\d+) ignored') {
    throw 'Missing successful desktop test summary'
}
$passed = [int]$Matches[1]
$ignored = [int]$Matches[2]
if ($passed -eq 0 -or $testOutput -notmatch 'test gpu_startup::tests::native_dxgi_fast_path_preserves_the_d3d12_intel_decision \.\.\. ok') {
    throw 'The native DXGI parity regression did not run successfully'
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$dumpbin = & $vswhere -latest -products '*' -find 'VC/Tools/MSVC/*/bin/Hostx64/x64/dumpbin.exe' | Select-Object -Last 1
if (-not $dumpbin) { throw 'Visual Studio dumpbin was not found' }
Invoke-Logged $dumpbin @('/imports', $executable) 'candidate-imports.txt'
$candidateImports = Get-Content -LiteralPath (Join-Path $results 'candidate-imports.txt') -Raw
$candidateHasDxgi = $candidateImports -match '(?im)^\s*dxgi\.dll\s*$'
$report = [ordered]@{ head = $testedHead; passed = $passed; failed = 0; ignored = $ignored; candidate_imports_dxgi = $candidateHasDxgi }

if ($BaselineRef) {
    # A comparison can be enabled temporarily on a fork. Restore the tested source even when
    # fetching/building the baseline fails; no modified or untrusted shell text is evaluated.
    try {
        Invoke-Logged git @('fetch', '--no-tags', '--depth=1', 'origin', $BaselineRef) 'baseline-fetch.txt'
        Invoke-Logged git @('switch', '--detach', $BaselineRef) 'baseline-checkout.txt'
        Invoke-Logged cargo @('build', '--locked', '--profile', 'test', '-p', 'photocraft', '--bin', 'photocraft') 'baseline-build.txt'
        Invoke-Logged $dumpbin @('/imports', $executable) 'baseline-imports.txt'
        $baselineImports = Get-Content -LiteralPath (Join-Path $results 'baseline-imports.txt') -Raw
        $baselineHasDxgi = $baselineImports -match '(?im)^\s*dxgi\.dll\s*$'
        $report['baseline'] = $BaselineRef
        $report['baseline_imports_dxgi'] = $baselineHasDxgi
        $report['baseline_imports_factory1'] = $baselineImports -match '\bCreateDXGIFactory1\b'
        if ($candidateHasDxgi -and -not $baselineHasDxgi) {
            throw 'Candidate adds a mandatory DXGI DLL import absent from the baseline'
        }
    } finally {
        Invoke-Logged git @('switch', '--detach', $testedHead) 'restore-head.txt'
    }
}

$report | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $results 'result.json')
"`nDesktop tests: **$passed passed, 0 failed, $ignored ignored**. Native DXGI parity test passed." >> $env:GITHUB_STEP_SUMMARY
if ($BaselineRef) {
    "`nBaseline ``$BaselineRef`` imports dxgi.dll: **$($report.baseline_imports_dxgi)**; candidate: **$candidateHasDxgi**." >> $env:GITHUB_STEP_SUMMARY
}
