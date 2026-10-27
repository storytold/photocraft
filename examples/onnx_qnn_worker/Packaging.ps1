# Pure packaging policy helpers. Dot-sourcing does not restore, build, publish or execute workers.
Set-StrictMode -Version Latest

function Assert-PackageLimits {
    param([Parameter(Mandatory)][AllowEmptyCollection()][object[]]$Entries)
    if ($Entries.Count -eq 0 -or $Entries.Count -gt 512) { throw 'Package must contain 1..512 files (current host limit).' }
    $seen = [System.Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    [long]$total = 0
    foreach ($entry in $Entries) {
        $name = [string]$entry.path
        if ($name.Length -eq 0 -or $name.Length -gt 512 -or $name.Contains('\') -or $name.Contains(':') -or
            $name -eq 'record.json' -or -not $seen.Add($name)) { throw 'Invalid/duplicate/reserved package path.' }
        foreach ($leaf in $name.Split('/')) {
            if ($leaf.Length -eq 0 -or $leaf.Length -gt 128 -or $leaf.StartsWith('.') -or
                $leaf.EndsWith('.') -or $leaf -cnotmatch '^[A-Za-z0-9._-]+$' -or
                $leaf -match '^(CON|PRN|AUX|NUL|COM[0-9]|LPT[0-9])(\.|$)') { throw 'Unsafe package path component.' }
        }
        [long]$length = $entry.bytes
        if ($length -lt 0 -or $length -gt 256MB) { throw "Per-file host byte limit exceeded: $name" }
        # Subtract-before-add avoids overflow even for hostile length/count metadata.
        if ($length -gt (512MB - $total)) { throw 'Aggregate host byte limit exceeded.' }
        $total += $length
    }
    return $total
}

function New-AdapterManifest {
    param([Parameter(Mandatory)][string[]]$Files)
    return [ordered]@{
        package_version = 1
        descriptor = [ordered]@{
            api_version = 1
            id = 'org.photocraft.qnn.subject'
            name = 'PhotoCraft QNN scalar segmentation (unverified hardware)'
            version = '0.1.0-draft'
            operations = @(@{ id = 'select-subject'; output_kind = 'mask'; color_modes = @('rgb') })
            devices = @(@{ id = 'qnn.htp'; kind = 'npu'; backend = 'qnn'; name = 'Qualcomm HTP (adapter claim, not attestation)'; operations = @('select-subject') })
            resource_formats = @('onnx')
            params_schema = @{
                type = 'object'
                properties = @{ colorPolicy = @{ type = 'string'; enum = @('assume-srgb') } }
                required = @('colorPolicy')
                additionalProperties = $false
            }
        }
        launch = @{ kind = 'executable'; path = 'photocraft-onnx-qnn.exe' }
        files = @($Files)
        timeout_ms = 120000
    }
}

function Get-PeMachine {
    param([Parameter(Mandatory)][string]$Path)
    $file = [IO.File]::OpenRead($Path)
    $reader = [IO.BinaryReader]::new($file)
    try {
        if ($file.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5a4d) { throw 'Not a bounded PE file.' }
        $file.Position = 0x3c
        [long]$offset = $reader.ReadUInt32()
        if ($offset -gt $file.Length - 6) { throw 'Invalid PE offset.' }
        $file.Position = $offset
        if ($reader.ReadUInt32() -ne 0x00004550) { throw 'Invalid PE signature.' }
        return $reader.ReadUInt16()
    }
    finally { $reader.Dispose(); $file.Dispose() }
}
