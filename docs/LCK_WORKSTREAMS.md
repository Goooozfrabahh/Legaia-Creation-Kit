# LCK Development Task Board

## Overview
This document tracks the verified GitHub development checkpoints.
The workstreams are cumulative historical snapshots, not isolated modules.
Completed means the named implementation milestone, not universal gameplay certification.

## Organized branches

| Category | Task branch | Historical source |
|---|---|---|
| completed | [lck/completed/cross-cell-rendering](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/completed/cross-cell-rendering) | [Original](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/foundation-02) |
| in-progress | [lck/in-progress/selection-and-collision](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/in-progress/selection-and-collision) | [Original](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/multiselection-m1) |
| completed | [lck/completed/static-resource-resolver](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/completed/static-resource-resolver) | [Original](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/stage-b1-static-resource-resolution) |
| research | [lck/research/static-material-gaps](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/research/static-material-gaps) | [Original](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/stage-b1-1-static-resource-closure) |
| completed | [lck/completed/behavior-dependency-schema](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/completed/behavior-dependency-schema) | [Original](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/stage-b2a-animated-scripted-resources) |
| in-progress | [lck/in-progress/animated-actor-portability](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/in-progress/animated-actor-portability) | [Original](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/stage-b2b-native-animation-portability) |
| blocked | [lck/blocked/combined-resource-runtime-freeze](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/blocked/combined-resource-runtime-freeze) | [Original](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/stage-b2b-loading-repair) |
| in-progress | [lck/in-progress/native-static-placement](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/in-progress/native-static-placement) | [Original](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/stage-c1-static-placement) |
| research | [lck/research/floor-model-identification](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/research/floor-model-identification) | [Original](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/stage-c2-floor-model-research) |
| in-progress | [lck/in-progress/floor-tile-authoring](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/in-progress/floor-tile-authoring) | [Original](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/tree/lck/stage-c2-floor-tiles) |

## Completed implementation milestones

- [editor-multiselection](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/commit/7dc88047c43127e9fa2c3a474f097e3bd459b0e2)
- [native-quarter-turn](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/commit/e919e65157a6f20254ac0a930358b33d0e01c853)
- [experimental-floor-sampler](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/commit/61ab36bf49633aaf47885e0aa501a9645d30e621)
- [unrestricted-export-diagnostics](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/commit/3ccd15eb36de7e3763b73c8a802c2e73cfbc40ea)
- [foreign-texture-vram-protection](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/commit/083b99aca244325fac019549bed9b73073a45c1b)
- [mixed-resource-adapter](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/commit/ec2a0395517664c28f703acf13293f5588da32d6)
- [character-texture-protection](https://github.com/Goooozfrabahh/Legaia-Creation-Kit/commit/31e125bcf76baf082e77671c2ec89c33b7851dcd)

## Current unfinished tasks

- Finish native collision ownership for original retail objects.
- Complete floor tile drag/drop and walkable-ground painting.
- Finish universal object and resource importing.
- Resolve 45 remaining B1.1 static-material identities.
- Diagnose B2B combined-resource loading and freezing.
- Complete script, trigger, animation and NPC portability.
- Add floor-height-aware grid controls.
- Confirm and push the latest local marquee-selection improvements.
- Implement the two-mouse-button marquee shortcut.
- Complete GUI regression, export and DuckStation gameplay testing.

## Project safety

- Original branches remain available.
- No history rewriting or force-pushing.
- Original retail IMG and Latest Build remain protected.
- Experimental automated verification is distinct from gameplay testing.
