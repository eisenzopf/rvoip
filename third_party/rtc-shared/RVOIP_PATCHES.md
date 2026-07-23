# rvoip `rtc-shared` patch

This directory vendors the published `rtc-shared` 0.20.0-alpha.1 crate and
applies the shared error variant from
`eisenzopf/rtc@1e5b7d4be6d94850694f2519f4c235d16c871d53` required by the paired
`../rtc` DataChannel reliability fix.

See `../rtc/RVOIP_PATCHES.md` for the complete provenance and removal
condition.
