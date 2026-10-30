# Stage A implementation ledger

Baseline: 4476a7293b8245d6b4dd7ab1141a7ba213b36be4. Branch: lck/stage-a-base-forms-references. Latest Build remains unchanged until final acceptance and manual-testing approval. No GUI/emulator is launched.

## A1 — identity and database

Logical FileId plus 24-bit per-file local_id is the permanent FormKey. The displayed 32-bit FormID adds an 8-bit load-order position at query time. The import registry allocates master identities and is serialized independently of source fingerprints. Equal resources do not merge definitions. Base and Reference maps cannot share keys. Native provenance, shared definition properties, reference-specific state and collision ownership handles are separate.

Database transactions clone, validate and commit the full effective state and reverse indexes. Command history restores complete records and retains allocation high-water marks so abandoned identities are never reused. Retail reference edits/deletions are active-layer overrides; the master layer is untouched. Disabled and deleted are separate states. Use Info counts known dependencies and reports incomplete coverage explicitly.

Automated A1: 7 focused tests passed (allocation/load-order persistence, independent transforms/shared appearance revision, duplicate definitions, grouped undo/redo/collision handles, master overrides/deletion/disable, rollback and non-placeable assets). No UI/native compiler changes in this checkpoint. Stage A is not yet complete.
