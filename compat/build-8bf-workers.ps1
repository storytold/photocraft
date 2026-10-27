# Build original isolated workers and conformance DLLs using native Windows MSVC tools.
param(
  [string] $OutputDir,
  [string] $FixturesDir
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
if (-not $OutputDir) { $OutputDir = Join-Path $Root 'compat\8bf-worker\dist' }
if (-not $FixturesDir) { $FixturesDir = Join-Path $Root 'compat\8bf-worker\fixtures' }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'compat\8bf-worker\target' }
New-Item -ItemType Directory -Force $OutputDir | Out-Null
# Platform packaging may have installed only the editor's own target.
& rustup target add i686-pc-windows-msvc x86_64-pc-windows-msvc
if ($LASTEXITCODE -ne 0) { throw 'could not install the x86/x64 MSVC worker targets' }
foreach ($spec in @(@('i686', '32'), @('x86_64', '64'))) {
  $target = "$($spec[0])-pc-windows-msvc"
  $flagVar = 'CARGO_TARGET_' + ($target.ToUpper() -replace '-', '_') + '_RUSTFLAGS'
  $previous = [Environment]::GetEnvironmentVariable($flagVar)
  try {
    [Environment]::SetEnvironmentVariable($flagVar, '-C target-feature=+crt-static')
    & cargo build --manifest-path (Join-Path $Root 'compat\8bf-worker\Cargo.toml') --target-dir $TargetDir --release --locked --target $target
    if ($LASTEXITCODE -ne 0) { throw "worker build failed: $target" }
    Copy-Item (Join-Path $TargetDir "$target\release\photocraft-8bf-worker.exe") (Join-Path $OutputDir "photocraft-8bf-worker$($spec[1]).exe")
    $fixture = Join-Path $FixturesDir $spec[0]
    New-Item -ItemType Directory -Force $fixture | Out-Null
    foreach ($fixtureSpec in @(@('fixture', 'photocraft-conformance'), @('host_services', 'photocraft-host-services'), @('handle_services', 'photocraft-handle-services'))) {
      & rustc --edition 2024 --crate-type cdylib --target $target -C target-feature=+crt-static -O (Join-Path $Root "compat\8bf-worker\tests\$($fixtureSpec[0]).rs") -o (Join-Path $fixture "$($fixtureSpec[1]).8bf")
      if ($LASTEXITCODE -ne 0) { throw "fixture build failed: $target $($fixtureSpec[0])" }
    }
  } finally {
    [Environment]::SetEnvironmentVariable($flagVar, $previous)
  }
}
