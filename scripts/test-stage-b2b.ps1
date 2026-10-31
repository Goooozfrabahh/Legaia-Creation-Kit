param(
    [Parameter(Mandatory=$true)][string]$Source,
    [Parameter(Mandatory=$true)][string]$Output
)
$ErrorActionPreference='Stop'
if(Test-Path -LiteralPath $Output){throw 'Choose a new B2B verification directory; existing evidence is preserved.'}
New-Item -ItemType Directory -Path $Output | Out-Null
$env:LCK_P0_SOURCE=$Source
$env:LCK_AUDIT_SOURCE=$Source
$env:LCK_RETAIL_SOURCE=$Source
$env:LCK_FOREIGN_SOURCE=$Source
$env:LCK_LOADER_IMG=$Source
$env:LCK_NATIVE_INVENTORY_SOURCE=$Source
$env:LCK_M2_SOURCE='D:\AstraRimElmProject\Backups\LCK_Outdoor_Visual_20261027\LCK_EXPERIMENTAL_VISUAL_ONLY_town01_outdoor_foreign_props_20261027.img'
$env:LCK_REPAIR_DIR=Join-Path $Output 'FiveObjects'
$env:LCK_LIFECYCLE_DIR=Join-Path $Output 'OwnedLifecycle'
$env:LCK_B2B_REPORT=Join-Path $Output 'B2B_NATIVE_ANIMATION.json'
$env:LCK_B2B_OUTPUT=Join-Path $Output 'Candidate'
$env:LCK_BASE_AUDIT='D:\AstraRimElmProject\ManualTests\LCK_Gamewide_Resource_Audit_20261031'
$env:LCK_B1_REMAINING='D:\AstraRimElmProject\ManualTests\LCK_StageB1_20261031_Results\REMAINING_FAILURE_EVIDENCE.json'
$env:LCK_CLOSURE_OUTPUT=Join-Path $Output 'B1_1_Investigation'
$fixtures=@(
 'stage_b2b_original_installer_probe',
 'stage_b2b_stream_and_unsupported_inventory',
 'stage_b2b_native_animated_portability',
 'real_town01_pac_packaging_and_native_donor_gates',
 'real_plain_bundle_registration_uses_descriptor_lookup',
 'native_tim_payload_relocation_uses_real_pixels_without_export',
 'cross_town_native_payloads_form_replay_checked_packages',
 'original_tim_member_suffixes',
 'exact_five_terrain_export_readback_and_owned_lifecycle',
 'repeated_owned_move_rotate_delete_export_reopen',
 'native_base_reference_acceptance_and_reordered_draws',
 'exact_user_legacy_scene_migrates_idempotently_and_roundtrips_project',
 'record_workspace_duplicate_undo_redo_restores_formkeys_and_collision_associations',
 'project_bridge_preserves_existing_native_bytes_and_portable_owned_collision',
 'reference_move_delete_projects_exact_existing_owned_collision_operations',
 'audit_original_test12345_partial_native_bytes',
 'multiple_towns_and_categories_share_one_native_dependency_inventory',
 'rim_elm_static_copy_additive_contract_stays_blocked',
 'rim_elm_names_use_native_identity_and_verified_visual_aliases',
 'rim_elm_ctrl_d_opens_generated_collision_properties',
 'unbound_candidate_reports_scene_wall_dependencies',
 'selected_world_object_report_preserves_native_provenance',
 'inspect_rim_elm_binding_dependencies',
 'retail_native_wall_script_translation_readback',
 'representative_native_graphs_and_six_request_dependency_benchmark',
 'stage_b2a_native_dependency_census',
 'stage_b1_1_source_closure_investigation',
 'stage_b1_1_non_geometry_consumer_evidence'
)
$results=@()
Push-Location -LiteralPath (Split-Path -Parent $PSScriptRoot)
try {
    $log=Join-Path $Output 'regular-studio.log'
    & cargo test -p legaia-studio --release --lib --locked *> $log
    $results+=@{test='regular-studio';exit=$LASTEXITCODE;log=$log}
    $log=Join-Path $Output 'code-hooks.log'
    & cargo test -p legaia-code-hooks --release --lib --locked *> $log
    $results+=@{test='code-hooks';exit=$LASTEXITCODE;log=$log}
    foreach($fixture in $fixtures){
        $env:LCK_P0_OUTPUT_DIR=Join-Path $Output ('output-'+$fixture)
        New-Item -ItemType Directory -Path $env:LCK_P0_OUTPUT_DIR | Out-Null
        $log=Join-Path $Output ($fixture+'.log')
        & cargo test -p legaia-studio --release --lib --locked $fixture -- --ignored --nocapture *> $log
        $results+=@{test=$fixture;exit=$LASTEXITCODE;log=$log}
        $results | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 (Join-Path $Output 'REGRESSION_RESULTS.json')
    }
    $log=Join-Path $Output 'editor-check.log'
    & cargo check -p legaia-studio --bin legaia-studio --locked *> $log
    $results+=@{test='editor-check';exit=$LASTEXITCODE;log=$log}
    $results | ConvertTo-Json -Depth 5 | Set-Content -Encoding utf8 (Join-Path $Output 'REGRESSION_RESULTS.json')
} finally {Pop-Location}
if(@($results | Where-Object {$_.exit -ne 0}).Count -gt 0){exit 1}
