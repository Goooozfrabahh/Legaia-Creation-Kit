# Stage B game-wide native resource audit

The audit is isolated in `gamewide_resource_audit.rs`, included only by the Studio unit-test harness. It does not change the production editor, exporter, Base Form schema, native allocation behavior, collision journal or Latest Build.

Run from a native development shell with the original retail IMG and a **new** absolute report directory:

```powershell
.\scripts\audit-gamewide-resources.ps1 -Source 'D:\Emulators\Games\Legend of Legaia\LegendOfLegaia.img' -Output 'D:\AstraRimElmProject\ManualTests\LCK_Gamewide_Resource_Audit_NEW'
```

This compiles/runs a headless Rust test harness, not LCK, a renderer or an emulator. Never replace that command with a GUI/emulator launch. The retail file is opened read-only and hashed before/after. Files are create-only; preserve interrupted output rather than treating it as complete.

The Rust pass inventories every indexed PROT entry, all ISO file extents in the existing logical 2048-byte view, bounded nested containers and opaque ranges, all named CDNAME blocks/variants, native MAP descriptors/reference occurrences, wall/floor grids, MAN/actor/binding records and native registration banks. Hash equality never merges semantic Base Forms or independent References. Structural scan hits are not evidence of registration or runtime execution. Recognized geometry/image leaves are never reinterpreted as typed stream headers.

The repaired Stage A resolver reconstructs boot TOC0 uploads before scene descriptors/PAC. Each material-package check uses strict existing writers, protects field runtime regions, and compares every primitive UV bounding-box sample before and after serialization. Package checks use an empty destination to measure intrinsic package capability; bounded registration probes separately use actual original destination scene VRAM, boot/character reservations and independent static native-loader SHA comparisons. Neither proves live engine behavior or arbitrary cross-cell fit.

The Python report renderer preserves source identities in the requested TSV/JSON reports, separates additional raw TMD candidates from registered model checks, and groups actual failure reasons. The independent verifier checks counts, hashes-as-identities, graph endpoints, compatibility rows, no gameplay claims and streamed JSON syntax. No external libraries are needed for these scripts.

The reports explicitly distinguish geometry/material-package closure, whole behavior closure, package readback, bounded registration, whole IMG fixtures, preview/GUI status and gameplay status. Unknown or unsupported assets are inventoried; no compatibility whitelist or editor restrictions are introduced. Runtime estimates separate encoded staging, decoded destinations and omitted live allocations.

Object Window design: preserve primary category and add orthogonal Animated & Scripted filters (Animated Only, Scripted Only, Animated + Scripted, Unknown Dependencies). Positive animation/script operands do not prove active playback or absence of other dependencies. Static visual detachment must never imply preservation of behavior. This audit implements no UI changes.

Local deliverables: `D:\AstraRimElmProject\ManualTests\LCK_Gamewide_Resource_Audit_20261031`. Consult GAMEWIDE_AUDIT_SUMMARY.md, MISSING_CAPABILITIES.md and AUDIT_INTEGRITY.md before interpreting coverage. Large inventories and original-data snippets remain local; only source utilities and compact audit summaries are checkpointed.

Known scope limits include strict parser plausibility rules, undecoded opaque and non-field layouts, multimedia frames and XA Form2 tails, animation clip/dynamic script closure, original wall ownership and new walkable floors. Do not label the whole game compatible from package checks. The existing user-verified Bylon/R8 evidence remains historical and object-specific.
