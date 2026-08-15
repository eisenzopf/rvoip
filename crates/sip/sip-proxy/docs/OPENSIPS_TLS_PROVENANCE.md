# OpenSIPS TLS interop peer — package provenance and review record

The `opensips/*/tls` interop rows run a derived image
(`rvoip/opensips-tls-interop:3.6.7-1`) that adds exactly three reviewed OpenSIPS
module packages on top of a digest-pinned base. The pins live in **four places
that must always move together**:

| Site | What it pins |
|---|---|
| `tests/interop/images/opensips-tls/Dockerfile` | base digest + the three deb SHA-256s (build-time check) |
| `tests/interop/scripts/opensips_tls_provenance.py` | `REVIEWED_DEBS` + `MODULES` (runtime check against the live container) |
| `tests/interop/scripts/verify_tls_evidence.py` | same digests re-asserted in row evidence |
| `crates/sip/rvoip-sip/scripts/beta_release_report.py` | Dockerfile SHA + deb/module digests in the release gate |

Because `beta_release_report.py` also pins the **Dockerfile's own SHA-256**, any
Dockerfile edit is itself a pin update.

## Current pins (since f022220a, 2026-08-07)

- Base: `opensips/opensips:3.6@sha256:eba1396b…` (pulled 2026-06-18 era, tag still
  resolves to this digest as of 2026-08-12)
- `opensips-tls-module_3.6.7-1_amd64.deb` — `685f704f…`
- `opensips-tlsmgm-module_3.6.7-1_amd64.deb` — `20d83193…`
- `opensips-tls-openssl-module_3.6.7-1_amd64.deb` — `690c52e0…`
  (module `tls_openssl.so` — `77714d25…`)

## Incident: upstream republished a same-version deb (2026-07/08)

apt.opensips.org **replaced** `opensips-tls-openssl-module_3.6.7-1_amd64.deb` in
place — same filename, same version, different bytes — some time between the
original review (pins `05e0c80d…` deb / `ec8dbf71…` module, commit 64151984,
2026-07-29) and the first CI failure (release-qualification run 31145050423,
2026-08-07 03:40 UTC). The sibling debs from the same source version were *not*
re-uploaded. f022220a refreshed the four pin sites to the served bytes to
unblock 0.3.7 qualification; the binary-level review of the changed artifact was
completed afterwards (below). The old bytes are not recoverable (no local or
registry copies, no web.archive.org captures; the pool overwrites in place), so
the review compares against public source and version lineage instead.

## Review of the republished artifact (2026-08-12) — verdict: benign rebuild

Method and results, all reproducible from the commands noted:

1. **Repo signature chain.** `dists/bullseye/Release` (regenerated daily) is
   GPG-verified against `/usr/share/keyrings/opensips-org.gpg` taken from the
   *pinned* base image (trusted at original review time): good signature from
   "OpenSIPS Project <info@opensips.org>" (RSA 6173E83A…). The signed
   `Packages` index lists SHA-256 `690c52e0…` for the deb, matching the served
   pool file — a deliberate upstream republish, not a transport-level swap.
2. **Version lineage.** Against `…_3.6.6-1_amd64.deb` (still in the pool):
   `objdump -T` symbol tables are **identical**, `NEEDED` libraries identical
   (`libssl.so.1.1`, `libcrypto.so.1.1`, `libpthread`, `libc`), `strings` delta
   is version banners only. No new imports, exports, URLs, or paths.
3. **Embedded source revision.** OpenSIPS modules embed their build's git
   short-rev. The served module embeds `eaee48e28e` = the public
   "Bump version to 3.6.7" commit, which GitHub reports as **identical to tag
   3.6.7** (the 3.6.6-1 module likewise embeds its own tag commit `26c0c4e33`).
4. **From-source rebuild.** `modules/tls_openssl` built at tag 3.6.7 in a
   bullseye/amd64 container (gcc 10.2.1, Debian packaging flags
   `CFLAGS="-Wall -g -O2 -fPIC -DMOD_NAME=\"tls_openssl\"` + dpkg hardening,
   `-Wl,-z,relro -Wl,-z,now`): `.text` is the **same size at the same address**
   and the normalized disassembly diff is **0 lines over 15,156** instruction
   lines. Dynamic symbol tables identical. `.rodata` differs only by the
   embedded short-rev being 10 vs 7 chars (16 B of alignment, downstream
   3-byte string shifts); `strings` confirms no other content delta.

Conclusion: the republished deb is a **no-source-change rebuild of public
OpenSIPS 3.6.7** (deb internal timestamps are SOURCE_DATE_EPOCH-normalized to
the 2026-06-17 changelog date, so the rebuild date is not visible in the
artifact). Why upstream re-uploaded exactly one module package is unknown;
nothing in the artifact is security-relevant.

Reviewed inputs and comparison outputs are stashed content-addressed at
`~/Developer/rvoip-review-artifacts/opensips-tls-openssl-3.6.7-1/` (local
machine stash, includes the reviewed deb, both `.so` files, and diffs).

## Operational notes for the next republish

- A `running OpenSIPS module hashes do not match reviewed packages` failure can
  also mean **the checkout is stale** (branch predates the latest pin refresh)
  — check `docker history --no-trunc rvoip/opensips-tls-interop:…`; the
  `|N NAME=value` prefix shows the exact build-args the local image was built
  with before assuming the image is wrong.
- Re-review recipe when upstream mutates an artifact again: verify the signed
  index covers the new bytes (keyring from the pinned base image); diff
  dynsym/NEEDED/strings against the previous-version deb still in the pool;
  resolve the embedded short-rev against the release tag; rebuild the module
  from the tag and require a clean normalized-disassembly diff. Then update all
  four pin sites together and rebuild the image.
- **Vendor the reviewed debs.** Upstream mutates same-version artifacts and
  keeps no history (and archive.org has no captures), so the reviewed bytes
  should be preserved content-addressed at review time (as done above), ideally
  also somewhere CI can reach (release asset or object storage) so the
  Dockerfile could fall back to a mirror keyed by the pinned hash. Adopting a
  mirror URL in the Dockerfile is deliberately **not** done here because the
  Dockerfile hash is itself release-gate-pinned; fold it into the next planned
  pin update instead.
