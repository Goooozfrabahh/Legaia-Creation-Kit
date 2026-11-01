# PROT and disc layout differences

Extraction-space entry identities are retained. Sizes are true next-start footprints, not the historical overread window.

## A

IMG SHA256 `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`; bytes 466714416. PROT starts at disc LBA 242, sectors 59206.

Changed decoded-user-data entry identities: []. PROT prefix (TOC/boot region) SHA256 `eead17d7084f76c8302b8bc83f77ce564c1bb55d1fc7b084cfe64bdf963e1caf`; retail `eead17d7084f76c8302b8bc83f77ce564c1bb55d1fc7b084cfe64bdf963e1caf`.


Exact shifted entry ranges (inclusive; constant relative/disc LBA delta; unchanged entries retain exact user-data hashes):

- None.

Bundle SHA `34844ba248e9373bba3c3a34e71c1bedcc56282e31e9c187b834f539e22c9e73`; PAC SHA `45aec0de8cc4c1898064f11711491ed90cdeca2cdd6605e4eaf7de90e37aa9d6`. Descriptor order `[2, 3, 5, 6, 7, 20]`.

## B

IMG SHA256 `dc84ddf7ff1e75bbb58dad1ae8eefd539c8b38dbec0d8d2abe1044d3c566f13d`; bytes 466716768. PROT starts at disc LBA 242, sectors 59207.

Changed decoded-user-data entry identities: [4, 5]. PROT prefix (TOC/boot region) SHA256 `ab844d0b3979f9bb20484197d75ce16422fa5403a34bc9f2305a309a5745bd72`; retail `eead17d7084f76c8302b8bc83f77ce564c1bb55d1fc7b084cfe64bdf963e1caf`.

- Entry4: relative LBA 243->243, disc LBA 485->485, bytes 227328->227328; old SHA `34844ba248e9373bba3c3a34e71c1bedcc56282e31e9c187b834f539e22c9e73`, new `95157a5864ae6ec535ce77f96519d6f0030d8fc3f89a90384979fc9fcd769c4b`.
- Entry5: relative LBA 354->354, disc LBA 596->596, bytes 368640->370688; old SHA `45aec0de8cc4c1898064f11711491ed90cdeca2cdd6605e4eaf7de90e37aa9d6`, new `5f245ceb12326dc8b12342c47d2b0f8e28f8154e557b2a844a9dc4bf1953889f`.

Exact shifted entry ranges (inclusive; constant relative/disc LBA delta; unchanged entries retain exact user-data hashes):

- Entries 6..1232: +1 sectors.

Bundle SHA `95157a5864ae6ec535ce77f96519d6f0030d8fc3f89a90384979fc9fcd769c4b`; PAC SHA `5f245ceb12326dc8b12342c47d2b0f8e28f8154e557b2a844a9dc4bf1953889f`. Descriptor order `[2, 3, 5, 6, 7, 20]`.

## C

IMG SHA256 `9b13a75ae0c0f2efc96b0009bab09a13d9d2c3701c3e0c1919c6e4b7da0ca5ac`; bytes 466716768. PROT starts at disc LBA 242, sectors 59207.

Changed decoded-user-data entry identities: [4, 5]. PROT prefix (TOC/boot region) SHA256 `ec89eeac5d8f194cd089850ccdd608894e16f3b3010bcc79a5fea6422004f3c7`; retail `eead17d7084f76c8302b8bc83f77ce564c1bb55d1fc7b084cfe64bdf963e1caf`.

- Entry4: relative LBA 243->243, disc LBA 485->485, bytes 227328->229376; old SHA `34844ba248e9373bba3c3a34e71c1bedcc56282e31e9c187b834f539e22c9e73`, new `1d950774edb4f58855fa7014e9304256797f00ff9ca335c2204313fea88c268e`.
- Entry5: relative LBA 354->355, disc LBA 596->597, bytes 368640->368640; old SHA `45aec0de8cc4c1898064f11711491ed90cdeca2cdd6605e4eaf7de90e37aa9d6`, new `335855d6a619a1923eb3a02c33541a2c21570bebeebd467038e19233a640b874`.

Exact shifted entry ranges (inclusive; constant relative/disc LBA delta; unchanged entries retain exact user-data hashes):

- Entries 5..1232: +1 sectors.

Bundle SHA `1d950774edb4f58855fa7014e9304256797f00ff9ca335c2204313fea88c268e`; PAC SHA `335855d6a619a1923eb3a02c33541a2c21570bebeebd467038e19233a640b874`. Descriptor order `[2, 3, 5, 6, 7, 20]`.

## M

IMG SHA256 `6842f78fb771b0a5c77fc9efe4ad903c6f2a36932d8194279fa91c3704b49aac`; bytes 466721472. PROT starts at disc LBA 242, sectors 59209.

Changed decoded-user-data entry identities: [4, 5]. PROT prefix (TOC/boot region) SHA256 `973ad9997b5ef71458ac4e742f828a079763a3974a95cfd85ba1133765861cc6`; retail `eead17d7084f76c8302b8bc83f77ce564c1bb55d1fc7b084cfe64bdf963e1caf`.

- Entry4: relative LBA 243->243, disc LBA 485->485, bytes 227328->231424; old SHA `34844ba248e9373bba3c3a34e71c1bedcc56282e31e9c187b834f539e22c9e73`, new `77ebc0867e12e023a1d34cf215cca699fd46f407c14217b74cab6821e64d1889`.
- Entry5: relative LBA 354->356, disc LBA 596->598, bytes 368640->370688; old SHA `45aec0de8cc4c1898064f11711491ed90cdeca2cdd6605e4eaf7de90e37aa9d6`, new `ff4ad1aa1f6f707681e29fd2206ab60c3dcb4f99d04d4ffbf26a5a8512c39154`.

Exact shifted entry ranges (inclusive; constant relative/disc LBA delta; unchanged entries retain exact user-data hashes):

- Entries 5..5: +2 sectors.
- Entries 6..1232: +3 sectors.

Bundle SHA `77ebc0867e12e023a1d34cf215cca699fd46f407c14217b74cab6821e64d1889`; PAC SHA `ff4ad1aa1f6f707681e29fd2206ab60c3dcb4f99d04d4ffbf26a5a8512c39154`. Descriptor order `[2, 3, 5, 6, 7, 20]`.

## Detailed inventory and physical-sector caveat

All 1233 valid extraction entries were independently compared for each case.
Full exact old/new entry LBAs, sizes and hashes remain local at `D:\AstraRimElmProject\ManualTests\LCK_B2B_Forensic_Review_20261031\ALL_PROT_ENTRY_LAYOUT_AND_HASHES.json`,
SHA256 `1dcedd561bbd5f3298fd9e73554d5a9e65b949f7d03f968b4c4fdeb49cc3b2e4` (1941775 bytes). The compact JSON
contains every changed entry and lossless constant-delta ranges for moved entries.
PROT growth also cascades downstream ISO extents/MSF headers; large physical raw
sector differences do not mean those resource payloads changed. Existing sector
validation manifests are hash-indexed in the JSON; no IMG is committed.
The bundle starts at disc LBA485 in all four cases. Its first raw sector header
is BCD `00:08:35`, mode2. Native pending E seek uses that same target.

## Exact prefix and post-PROT movement

### A

TOC prefix differences only at word offsets `16 + 4*i`: none. Index1233 is the terminal end bound. All non-TOC boot/prefix bytes match retail. Original post-PROT sectors [59448, 198432] shift by +0 sectors; ISO extents/volume size and MSF headers follow that growth.

### B

TOC prefix differences only at word offsets `16 + 4*i`: [[6, 1233, 1]]. Index1233 is the terminal end bound. All non-TOC boot/prefix bytes match retail. Original post-PROT sectors [59448, 198432] shift by +1 sectors; ISO extents/volume size and MSF headers follow that growth.

### C

TOC prefix differences only at word offsets `16 + 4*i`: [[5, 1233, 1]]. Index1233 is the terminal end bound. All non-TOC boot/prefix bytes match retail. Original post-PROT sectors [59448, 198432] shift by +1 sectors; ISO extents/volume size and MSF headers follow that growth.

### M

TOC prefix differences only at word offsets `16 + 4*i`: [[5, 5, 2], [6, 1233, 3]]. Index1233 is the terminal end bound. All non-TOC boot/prefix bytes match retail. Original post-PROT sectors [59448, 198432] shift by +3 sectors; ISO extents/volume size and MSF headers follow that growth.
