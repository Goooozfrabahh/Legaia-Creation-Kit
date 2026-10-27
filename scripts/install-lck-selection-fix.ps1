param(
    [Parameter(Mandatory=$true)][ValidatePattern('^[0-9a-fA-F]{64}$')][string]$ExpectedSha256,
    [Parameter(Mandatory=$true)][ValidatePattern('^[0-9a-fA-F]{40}$')][string]$SourceCommit
)
$ErrorActionPreference='Stop'
$repo=Split-Path -Parent $PSScriptRoot
$built=Join-Path $repo 'target\release\legaia-studio.exe'
$template=Join-Path $PSScriptRoot 'lck-latest-build.cmd'
$target='D:\AstraRimElmProject\legaiasource\legend-of-legaia-re-main\target\release\legaia-studio.exe'
$launcher='C:\Users\dupre\OneDrive\Desktop\LatestBuild__LCK.cmd'
$baseline='D:\AstraRimElmProject\ManualTests\M11_20261027_01\bin\legaia-studio-M11-2667653.exe'
$run='D:\AstraRimElmProject\ManualTests\M11_SelectionFix_20261027_01'
$backup='D:\AstraRimElmProject\Backups\LCK_M11_SelectionFix_20261027_01'
if((Get-FileHash -LiteralPath $built).Hash -ne $ExpectedSha256){throw 'Release hash mismatch'}
if((Get-FileHash -LiteralPath $baseline).Hash -ne '87235b719b5c03fac85a4f6ee82842fd9294a0f0bac614b4c1566d58f15b1cc4'){throw 'M1.1 rollback hash mismatch'}
if((git -C $repo rev-parse HEAD) -ne $SourceCommit){throw 'Source checkpoint changed'}
if(git -C $repo status --porcelain){throw 'Source has uncommitted changes'}
if(Test-Path -LiteralPath $backup){throw 'Rollback directory already exists; inspect it before retrying'}
$staged=$target+'.selection-fix-new'
if(Test-Path -LiteralPath $staged){throw 'An installation staging file already exists'}
New-Item -ItemType Directory -Path $backup | Out-Null
Copy-Item -LiteralPath $target -Destination "$backup\installed-before-fix.exe"
Copy-Item -LiteralPath $launcher -Destination "$backup\LatestBuild__LCK.cmd.before"
Copy-Item -LiteralPath $baseline -Destination "$backup\legaia-studio-M11-2667653.exe"
Copy-Item -LiteralPath $built -Destination "$run\bin\legaia-studio-selection-fix.exe"
Copy-Item -LiteralPath $built -Destination $staged
if((Get-FileHash -LiteralPath $staged).Hash -ne $ExpectedSha256){throw 'Staged executable hash mismatch'}
# Move only this exact executable after preserving a separate recovery copy.
# No process is stopped, launched or controlled. A locked path fails here.
Move-Item -LiteralPath $target -Destination "$backup\installed-before-fix-renamed.exe"
try {Move-Item -LiteralPath $staged -Destination $target}
catch {Move-Item -LiteralPath "$backup\installed-before-fix-renamed.exe" -Destination $target;throw}
if((Get-FileHash -LiteralPath $target).Hash -ne $ExpectedSha256){throw 'Installed executable hash mismatch'}
# The desktop launcher must never compile the older production checkout again.
[System.IO.File]::WriteAllText($launcher,([System.IO.File]::ReadAllText($template) -replace '\r?\n',"`r`n"),[System.Text.Encoding]::ASCII)
$installedLauncher=[System.IO.File]::ReadAllText($launcher)
if($installedLauncher -match 'cargo\s+(run|build)' -or !$installedLauncher.Contains($target)){throw 'Launcher target validation failed'}
$manifest=[ordered]@{
    source_commit=$SourceCommit
    source_worktree=$repo
    installed_path=$target
    sha256=(Get-FileHash -LiteralPath $target).Hash
    manual_test_copy="$run\bin\legaia-studio-selection-fix.exe"
    launcher=$launcher
    launcher_sha256=(Get-FileHash -LiteralPath $launcher).Hash
    rollback_directory=$backup
    preserved_M11_sha256=(Get-FileHash -LiteralPath $baseline).Hash
    automated_verified=$true
    manual_gui_verified=$false
    native_gameplay_verified=$false
    application_launched=$false
}
$manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath "$run\results\installation.json"
$manifest | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath "$backup\installation.json"
$manifest | ConvertTo-Json -Depth 4
