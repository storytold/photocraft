<#
.SYNOPSIS
  Build the PhotoCraft MCP server (photocraft-cli.exe) in release mode and run its test suites.
  Windows. Linux and macOS: scripts/build-mcp.sh.

.EXAMPLE
  scripts\build-mcp.ps1                 # build + test
  scripts\build-mcp.ps1 -Install        # ... then cargo install photocraft-cli onto PATH
  scripts\build-mcp.ps1 -CheckTargets   # ... also type-check the MCP crates for other installed targets
  scripts\build-mcp.ps1 -NoTest         # build only

  Honours CARGO_TARGET_DIR. Extra cargo flags: $env:CARGO_FLAGS = "--features heif".
#>
param(
    [switch]$Install,
    [switch]$CheckTargets,
    [switch]$NoTest
)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

$flags = @()
if ($env:CARGO_FLAGS) { $flags = $env:CARGO_FLAGS -split '\s+' | Where-Object { $_ } }

function Invoke-Cargo {
    & cargo @args
    if ($LASTEXITCODE -ne 0) { throw "cargo $($args -join ' ') failed with exit code $LASTEXITCODE" }
}

Write-Host '==> building photocraft-cli (release)'
# Take the binary's path from Cargo's own report: with --target or CARGO_BUILD_TARGET it lives
# under target\<triple>\release, not target\release. Diagnostics still go to stderr.
$messages = & cargo build --release --locked -p photocraft-cli @flags --message-format=json-render-diagnostics
if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
$bin = $messages | Where-Object { $_.StartsWith('{') } | ForEach-Object { $_ | ConvertFrom-Json } |
    Where-Object { $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'photocraft-cli' -and $_.executable } |
    Select-Object -Last 1 -ExpandProperty executable
if (-not $bin -or -not (Test-Path $bin)) { throw 'cargo did not report a photocraft-cli executable' }

if (-not $NoTest) {
    Write-Host '==> testing photocraft-automation and photocraft-cli (unit, MCP protocol, image, file-permission tests)'
    Invoke-Cargo test --locked -p photocraft-automation -p photocraft-cli --lib --tests @flags
}

if ($CheckTargets) {
    $hostTarget = ((rustc -vV) | Select-String '^host: (.+)$').Matches[0].Groups[1].Value
    $installed = @(rustup target list --installed 2>$null)
    foreach ($t in 'x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc', 'x86_64-pc-windows-gnu', 'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu', 'aarch64-apple-darwin', 'x86_64-apple-darwin') {
        if ($t -eq $hostTarget) { continue }
        if ($installed -contains $t) {
            Write-Host "==> cargo check --target $t"
            Invoke-Cargo check --locked -p photocraft-automation -p photocraft-cli --target $t @flags
        } else {
            Write-Host "--- skipping $t (rustup target add $t to check it)"
        }
    }
}

Write-Host "==> built $bin"
& $bin --version

if ($Install) {
    Write-Host '==> installing photocraft-cli with cargo install'
    Invoke-Cargo install --path apps/photocraft-cli --locked @flags
    Write-Host 'Installed into %USERPROFILE%\.cargo\bin (or your CARGO_INSTALL_ROOT); register it with your MCP client (docs/mcp.md).'
}
