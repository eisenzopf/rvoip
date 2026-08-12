# Probe fixtures

Captured PBX CLI output that `tests/pbx_amr_probe.rs` feeds to
`amr_probe.sh parse`, so the parser is pinned against the real column layout
of each PBX rather than a remembered one.

- `asterisk-core-show-codecs-with-amr.txt` — genuine capture from the
  AMR-patched Asterisk 20.20.1 lab image (`rvoip-asterisk:amr`,
  `asterisk -rx "core show codecs"`).
- `freeswitch-show-codec-with-amr.txt` — genuine capture from the lab
  FreeSWITCH 1.10.12 with mod_amr/mod_amrwb (`fs_cli -x "show codec"`).
- The `-without-amr` twins are the same captures with the AMR rows removed,
  standing in for the release-runner images (packaged Alpine Asterisk and a
  FreeSWITCH built without the AMR modules) until genuine captures from those
  images replace them. The surrounding rows — which is what the parser has to
  not trip over — are real.

These files exist to make a mutated always-supported parser fail in CI: the
`-without-amr` fixtures must parse to `amr=no amrwb=no`.
