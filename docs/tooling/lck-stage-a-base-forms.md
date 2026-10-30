# Stage A implementation ledger

Baseline: 4476a7293b8245d6b4dd7ab1141a7ba213b36be4. Branch: lck/stage-a-base-forms-references. Latest Build remains unchanged until final acceptance and manual-testing approval. No GUI/emulator is launched.

## A1 — identity and database

Logical FileId plus 24-bit per-file local_id is the permanent FormKey. The displayed 32-bit FormID adds an 8-bit load-order position at query time. The import registry allocates master identities and is serialized independently of source fingerprints. Equal resources do not merge definitions. Base and Reference maps cannot share keys. Native provenance, shared definition properties, reference-specific state and collision ownership handles are separate.

Database transactions clone, validate and commit the full effective state and reverse indexes. Command history restores complete records and retains allocation high-water marks so abandoned identities are never reused. Retail reference edits/deletions are active-layer overrides; the master layer is untouched. Disabled and deleted are separate states. Use Info counts known dependencies and reports incomplete coverage explicitly.

Automated A1: 7 focused tests passed (allocation/load-order persistence, independent transforms/shared appearance revision, duplicate definitions, grouped undo/redo/collision handles, master overrides/deletion/disable, rollback and non-placeable assets). No UI/native compiler changes in this checkpoint. Stage A is not yet complete.

## A2 — project persistence and migration

`lck_project.rs` stores LCK_PROJECT_V1 with schema version, logical project/master IDs, disc fingerprint, complete master/import registry and allocator state, active records/overrides/tombstones, cell-copy identities, legacy UID bindings, native adapter provenance, collision associations and exact preserved sidecar bytes. Save validates and reparses before calling the existing optimistic, durable atomic writer. The extension guard prohibits saving over a retail IMG. Missing/stale master or donor evidence yields diagnostics without deleting records.

`form_legacy.rs` adapts original source records and existing `.lckscene` state into shared definitions and individual references. The source registry uses original scene/record/cell identities; hashes only validate evidence. Legacy source text is retained exactly. The adapter preserves names, native/foreign identities, static-copy flags, per-axis transforms, NPC choices, authored/generated collision payloads and cell-copy IDs. Reimport retains existing mappings and marks missing created draws as project tombstones.

Automated A2: 4 focused project tests plus the exact preserved full222 scene migration passed. The native fixture retains all seven created UIDs and collision assignments, two hidden-native tombstones, repeat-import identity/allocator state and exact project save/reopen. The real fixture exposed f64 JSON payload drift; serde_json float_roundtrip is now enabled so serialized collision evidence roundtrips exactly. Existing native compiler/GUI remain unchanged at this checkpoint. Stage A is not yet complete.
