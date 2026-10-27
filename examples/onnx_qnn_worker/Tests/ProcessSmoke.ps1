# Real published ARM64 process, no models and no native/HTP initialization expected.
param([Parameter(Mandatory)][string]$WorkerDirectory, [Parameter(Mandatory)][string]$ResultsPath)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = [IO.Path]::GetFullPath($WorkerDirectory)
$exe = Join-Path $root 'photocraft-onnx-qnn.exe'
if (-not $IsWindows -or [Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture -ne 'Arm64') { throw 'T4 smoke requires a native Windows ARM64 PowerShell process; do not silently skip.' }
$registry = Get-Content -LiteralPath (Join-Path $root 'qnn-models.json') -Raw | ConvertFrom-Json
if ($registry.profiles.Count -ne 0 -or $null -ne $registry.defaultModel) { throw 'No-model smoke requires the published empty registry.' }
$job = Join-Path ([IO.Path]::GetTempPath()) ('photocraft-qnn-smoke-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory $job | Out-Null
$results = [Collections.Generic.List[object]]::new()
try {
    foreach ($case in @(
        @{ name = 'probe-no-model'; action = 'probe'; json = '{"protocol":1}'; expected = 'needs_model' },
        @{ name = 'prepare-no-model'; action = 'prepare'; json = '{"protocol":1}'; expected = 'needs_model' },
        @{ name = 'wrong-version'; action = 'probe'; json = '{"protocol":2}'; expected = 'error' },
        @{ name = 'unknown-control-field'; action = 'probe'; json = '{"protocol":1,"extra":0}'; expected = 'error' },
        @{ name = 'trailing-control-bytes'; action = 'probe'; json = '{"protocol":1}'; expected = 'error'; trailing = $true }
    )) {
        $request = Join-Path $job ($case.name + '.request')
        $response = Join-Path $job ($case.name + '.response')
        $writer = [IO.BinaryWriter]::new([IO.File]::Create($request))
        try {
            $header = [Text.Encoding]::UTF8.GetBytes($case.json)
            $writer.Write([Text.Encoding]::ASCII.GetBytes('PCADREQ1')); $writer.Write([uint32]$header.Length); $writer.Write($header)
            if ($case.ContainsKey('trailing')) { $writer.Write([byte]0) }
        } finally { $writer.Dispose() }
        $start = [Diagnostics.ProcessStartInfo]::new($exe)
        $start.UseShellExecute = $false
        $start.WorkingDirectory = $job # AppContext.BaseDirectory must still bind the package, not CWD.
        foreach ($arg in @('--action', $case.action, '--request', $request, '--response', $response)) { $start.ArgumentList.Add($arg) }
        $process = [Diagnostics.Process]::new(); $process.StartInfo = $start
        try {
            if (-not $process.Start()) { throw 'Worker did not start.' }
            if (-not $process.WaitForExit(30000)) { $process.Kill($true); $process.WaitForExit(); throw 'Worker exceeded 30s smoke deadline; process tree killed.' }
            if ($process.ExitCode -ne 0) { throw "Unexpected worker exit $($process.ExitCode)." }
        } finally { $process.Dispose() }
        $file = [IO.File]::OpenRead($response); $reader = [IO.BinaryReader]::new($file)
        try {
            if ($file.Length -lt 12 -or $file.Length -gt 1048588) { throw 'Invalid smoke response size.' }
            if ([Text.Encoding]::ASCII.GetString($reader.ReadBytes(8)) -cne 'PCADRES1') { throw 'Bad response magic.' }
            $length = $reader.ReadUInt32()
            if ($length -eq 0 -or $length -gt 1048576 -or $file.Length -ne 12 + $length) { throw 'Unexpected status response payload/length.' }
            $reply = [Text.Encoding]::UTF8.GetString($reader.ReadBytes([int]$length)) | ConvertFrom-Json -AsHashtable
            if ((($reply.Keys | Sort-Object) -join ',') -cne 'execution,message,output,protocol,status') { throw 'Unexpected v1 response keys.' }
            if ($reply.protocol -ne 1 -or $reply.status -cne $case.expected -or $null -ne $reply.output -or $null -ne $reply.execution) { throw 'Wrong no-model/error response.' }
            $results.Add(@{ name = $case.name; passed = $true; status = $reply.status; exit_code = 0 })
        } finally { $reader.Dispose(); $file.Dispose() }
    }
    # Invalid argument shape must fail before publishing a response.
    $start = [Diagnostics.ProcessStartInfo]::new($exe); $start.UseShellExecute = $false
    $start.ArgumentList.Add('--invalid')
    $process = [Diagnostics.Process]::Start($start)
    if ($null -eq $process) { throw 'Could not launch invalid-argument smoke.' }
    try {
        if (-not $process.WaitForExit(30000)) { $process.Kill($true); $process.WaitForExit(); throw 'Invalid-argument worker hung.' }
        if ($process.ExitCode -ne 2) { throw 'Invalid arguments must exit 2.' }
        $results.Add(@{ name = 'invalid-arguments'; passed = $true; exit_code = 2 })
    } finally { $process.Dispose() }
}
catch { $results.Add(@{ name = 'smoke-failure'; passed = $false; error = $_.Exception.Message }); throw }
finally {
    $path = [IO.Path]::GetFullPath($ResultsPath)
    New-Item -ItemType Directory -Force (Split-Path -Parent $path) | Out-Null
    @{ scope = 'T3/T4 real ARM64 apphost startup/exit and no-model/malformed-control frames only; no ORT native load, HTP session, inference, NPU, model quality or cancellation proof'; tests = @($results.ToArray()) } |
        ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $path -Encoding utf8NoBOM
    # Do not mutate the inventory-validated publish tree. Only bounded synthetic job files live here.
    $resolved = [IO.Path]::GetFullPath($job)
    if (-not $resolved.StartsWith([IO.Path]::GetFullPath([IO.Path]::GetTempPath()), [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($resolved) -notmatch '^photocraft-qnn-smoke-[0-9a-f]{32}$') { throw 'Refusing unexpected smoke cleanup path.' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
Write-Output "PASS ProcessSmoke: $($results.Count) checks passed; results written to '$path' (scope: real ARM64 apphost startup/exit without model; 无 NPU 推理 / no NPU inference)."
