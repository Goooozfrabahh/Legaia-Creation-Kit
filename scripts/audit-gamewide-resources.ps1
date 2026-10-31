param(
 [Parameter(Mandatory=$true)][string]$Source,
 [Parameter(Mandatory=$true)][string]$Output
)
$ErrorActionPreference='Stop'
if (!(Test-Path -LiteralPath $Source -PathType Leaf)) {throw 'Original disc does not exist'}
if (![IO.Path]::IsPathRooted($Output) -or (Test-Path -LiteralPath $Output)) {throw 'Use a NEW absolute report folder'}
$repo=Split-Path -Parent $PSScriptRoot
$priorSource=$env:LCK_AUDIT_SOURCE
$priorOutput=$env:LCK_AUDIT_OUTPUT
Push-Location -LiteralPath $repo
try {
 $env:LCK_AUDIT_SOURCE=[IO.Path]::GetFullPath($Source)
 $env:LCK_AUDIT_OUTPUT=[IO.Path]::GetFullPath($Output)
 # Test harness only. Never cargo run, editor, renderer, emulator or desktop input.
 cargo test -p legaia-studio --release --lib --locked --config 'profile.release.package.legaia-studio.opt-level=1' stage_b_complete_gamewide_resource_audit -- --ignored --nocapture
 if ($LASTEXITCODE -ne 0) {throw 'Audit failed; retain partial output and log; do not report complete'}
 cargo test -p legaia-studio --release --lib --locked --config 'profile.release.package.legaia-studio.opt-level=1' stage_b_all_scene_memory_profiles -- --ignored --nocapture
 if ($LASTEXITCODE -ne 0) {throw 'Per-scene memory census failed'}
 python -X utf8 scripts/render-gamewide-resource-audit.py $Output
 if ($LASTEXITCODE -ne 0) {throw 'Report rendering failed; retain census results'}
 python -X utf8 scripts/verify-gamewide-resource-audit.py $Output
 if ($LASTEXITCODE -ne 0) {throw 'Audit integrity check failed; do not report complete'}
} finally {
 $env:LCK_AUDIT_SOURCE=$priorSource
 $env:LCK_AUDIT_OUTPUT=$priorOutput
 Pop-Location
}
