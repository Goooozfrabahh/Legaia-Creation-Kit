# Foreign duplication and verified IMG export — 2026-10-28

Based on the actual installed `3ccd15e` source and its user export logs.
Unrestricted mode remains enabled by default. No host GUI/emulator was launched.

## Causes and corrections

* Ctrl+D followed `source_ref` on a foreign draw. That reference is a placeholder
  (`usize::MAX`), not a destination MAP record. Duplication now selects the actual
  foreign draw, copies its geometry/attributes and donor metadata, shares immutable
  donor VRAM, and allocates a new draw ID and UID. Foreign source identities,
  native descriptors, collision recipes, naming keys and transforms are retained.
  Duplicating a foreign copy follows the same path. Originals keep their prior path.
* The native `0x14` descriptor is a loader flag. Its size word was incorrectly
  treated as an opaque payload extent, causing the Conkram import to panic. The
  generic descriptor reader/repacker now preserves the declared flag word and
  actual stored bytes independently. No scene or object ID is special-cased.
* Fractional editor positions were refused by integer MAP encoding. The user
  explicitly approved nearest native precision on 2026-10-28. Shared export
  writers now log requested/encoded native units and yaw ticks; editor transforms
  are unchanged. Unsupported scale/pitch/roll/actor fields remain explicit failures,
  with no pretend encoding or replacement geometry.
* Generated mesh collision was unnecessarily subjected to manual wall-grid step
  checks. Those checks now apply only when explicit manual cells are present.
  No inferred ownership restriction was reinstated.
* The logged foreign export IMGs actually existed in the Stage E source folder;
  their imports had failed and the resulting IMGs were incomplete. Export now
  asks for the destination before background preparation, displays its full path,
  provides **Open IMG output folder**, clears stale writing status on failure,
  and checks existence and size again on UI completion. Experimental publication
  additionally compares disk-readback SHA256 to intended bytes before success.
  Worker errors/panics produce failure diagnostics rather than a successful result.

Each per-export `debug.txt` records the full manifest, requested/encoded precision,
TMD/TIM resource identities and upload regions, allocated MAP record/grid/selector,
changed resource entries and hashes, changed MAP descriptor bytes, failures,
output path, final size and disk-readback hash. Encoded resources are distinguished
from committed writes. Partial output remains **NOT PROVEN COMPLETE**.

## Generic scope and limits

The shared donor MAP/resource path now accepts placement and terrain-decoration
records according to their native record kind. There are no donor/destination
scene allowlists. Representative real-disc exports below cover foreign placements;
foreign terrain export and additional destination types remain manually unverified.

This is not universal engine support. Raw meshes without a native placement
adapter, MAN/animated actor transplantation, general scale/pitch/roll composition,
script closure, and complete native collision/walkability ownership still need
their corresponding writers. Their missing capabilities are reported explicitly;
no claim is made that successful TMD/TIM registration proves those behaviors.

## Evidence and preservation

Automated: 271 unit tests and 9 integration tests passed. New tests cover foreign
copy/copy-of-copy identity, donor texture sharing and collision provenance,
oversized flag words surviving resource repacking, requested/encoded precision,
and missing/empty/truncated output detection. Existing selection, native copying
and floor regressions passed. 27 optional tests were ignored in the regular run.

Two explicit headless runs replayed the user's existing saved edit manifests,
without creating a new arrangement:

* `town01_1793230668043386700.txt`: Conkram placement #19, including its requested
  non-integer position and yaw. Native MAP, TMD bundle and TIM/PAC writes succeeded.
* `town01_1793230962225054900.txt`: three user-positioned Bylon imports. Each
  received an independent native reference; all three resolved on reopening.

Both runs produced real new IMGs, verified their disk hash and native MAP readback,
reopened them through LCK's library/scene loader and resolved every imported draw's
TMD. Export left the editor manifest unchanged. Outputs and debug logs are under:
`D:\AstraRimElmProject\ManualTests\Foreign_Export_Fix_20261028_01`.

User-reported native gameplay success: original Rim Elm duplication, including
lower-floor/staircase behavior in `3ccd15e`. That path was preserved. This new
build's GUI and foreign native gameplay are **not verified**.

Rollback directory: `D:\AstraRimElmProject\Backups\LCK_ForeignFix_20261028_01`.
It preserves the installed `3ccd15e` executable (SHA256
`e5e24d51ab522dff7217404b20df6bc2e7189f265c02aacfd961bcd670d5b536`) and full
pre-fix Git recovery. The separate M1.1 rollback, retail and prior Stage E/user
images remain untouched. Installation records its own hash in installation.json.

## Next manual check

Launch Latest Build yourself. Import an object, Ctrl+D, then Ctrl+D its copy.
Confirm both copies retain textures, transforms and independent selection; move
one and save/reopen the scene to check persistence. Export to a new filename,
confirm the displayed full path and **Open IMG output folder** behavior, and review
debug.txt for failures or omitted dependencies before testing the IMG yourself.
No GUI or gameplay result is presumed by these instructions.
