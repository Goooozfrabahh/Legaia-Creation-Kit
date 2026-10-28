param(
    [Parameter(Mandatory=$true)][string]$Source,
    [Parameter(Mandatory=$true)][string]$Output
)
$ErrorActionPreference='Stop'
$repo=Split-Path -Parent $PSScriptRoot
if (!(Test-Path -LiteralPath $Source -PathType Leaf)) {throw 'Source raw disc image does not exist'}
$resolvedSource=(Resolve-Path -LiteralPath $Source).Path
if ([IO.Path]::GetExtension($resolvedSource) -notin @('.img','.bin')) {throw 'Use a raw .img/.bin disc image'}
if (![IO.Path]::IsPathRooted($Output) -or (Test-Path -LiteralPath $Output)) {throw 'Output must be a NEW absolute directory'}
if (!(Test-Path -LiteralPath (Split-Path -Parent $Output) -PathType Container)) {throw 'Output parent directory must exist'}
$priorSource=[Environment]::GetEnvironmentVariable('LCK_NATIVE_INVENTORY_SOURCE','Process')
$priorOutput=[Environment]::GetEnvironmentVariable('LCK_NATIVE_INVENTORY_OUTPUT','Process')
Push-Location -LiteralPath $repo
try {
    $env:LCK_NATIVE_INVENTORY_SOURCE=$resolvedSource
    $env:LCK_NATIVE_INVENTORY_OUTPUT=[IO.Path]::GetFullPath($Output)
    # Execute ONLY the automated read-only inventory test harness. Never use
    # cargo run, the LCK GUI executable, a window, input automation or emulator.
    cargo test -p legaia-studio --lib --locked original_disc_game_wide_native_inventory -- --ignored --nocapture
    if ($LASTEXITCODE -ne 0) {throw "Native inventory failed with exit code $LASTEXITCODE; retain partial reports for diagnosis"}
}
finally {
    Pop-Location
    [Environment]::SetEnvironmentVariable('LCK_NATIVE_INVENTORY_SOURCE',$priorSource,'Process')
    [Environment]::SetEnvironmentVariable('LCK_NATIVE_INVENTORY_OUTPUT',$priorOutput,'Process')
}
