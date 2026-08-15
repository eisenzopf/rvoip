#!/usr/bin/env bash
# Check our RFC 4867 RTP framing against Wireshark's, which is an independent
# implementation of the same specification.
#
# # The gap this closes
#
# The codec bits are bit-exact against 3GPP's reference implementations, and
# their sorting is checked against reference-produced `.amr` files. But
# everything RFC 4867 adds for RTP -- the CMR nibble, the table-of-contents
# chain, octet-aligned padding -- is otherwise verified only by packing and
# then unpacking with our own code, and a round trip cannot catch a symmetric
# mistake. Put the CMR in the wrong four bits and our depacker reads it out of
# the wrong four bits: perfect audio, unreadable by any peer.
#
# Two rvoip endpoints calling each other cannot find that class of bug either,
# for the same reason. tshark can, because nobody here wrote it.
#
# # What it does not prove
#
# That a *call* interoperates. This checks bytes against a dissector, not a
# negotiation against a PBX. The live FreeSWITCH and Asterisk runs are still
# the real thing.
#
# Wireshark is invoked as a binary and its source is never read.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO="$(cd "$HERE/../../../.." && pwd)"

for tool in tshark text2pcap; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "$tool is not installed; install Wireshark to run this check" >&2
    exit 2
  fi
done

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

total=0
mismatch=0

# The -2f selections pack two frames per payload, so the F-bit chain in the
# table of contents is dissected by an implementation nobody here wrote —
# production sends one frame per packet, which never exercises F=1.
#
# Measured tshark defect (4.6.6), worked around below and asserted so a fixed
# tshark surfaces: for an N>=2-frame OCTET-ALIGNED payload its AMR dissector
# demands data for one phantom extra frame — mode 0: "24 Bytes available, 36
# would be needed" for two 12-octet frames; the phantom is round(bits/8), so
# 12/13/15/17/18/20/25/30 across the NB modes. Feeding it the demanded pad
# octets clears the error *without* flagging them superfluous, which no
# self-consistent accounting could do; and single-frame payloads of exactly
# the RFC 4867 §4.4 size are accepted. Frame types and CMR still dissect
# correctly first, so the ToC-chain comparison below remains valid; only the
# not_enough_data expert info is expected on the *-oa-2f selections.
for selection in nb-oa nb-be wb-oa wb-be nb-oa-2f nb-be-2f wb-oa-2f wb-be-2f; do
  # The dissector names its fields per variant: amr.nb.* and amr.wb.*.
  case "$selection" in
    nb-oa*) mode="Narrowband AMR"; version="RFC 3267 octet aligned"; ft=amr.nb.toc.ft; cmr=amr.nb.cmr; pt=107 ;;
    nb-be*) mode="Narrowband AMR"; version="RFC 3267 BW-efficient"; ft=amr.nb.toc.ft; cmr=amr.nb.cmr; pt=106 ;;
    wb-oa*) mode="Wideband AMR";   version="RFC 3267 octet aligned"; ft=amr.wb.toc.ft; cmr=amr.wb.cmr; pt=105 ;;
    wb-be*) mode="Wideband AMR";   version="RFC 3267 BW-efficient"; ft=amr.wb.toc.ft; cmr=amr.wb.cmr; pt=104 ;;
  esac

  hex="$work/$selection.hex"
  manifest="$work/$selection.manifest"
  pcap="$work/$selection.pcap"

  (cd "$REPO" && cargo run --quiet -p rvoip-codec-core --all-features \
      --example amr_rtp_vectors -- "$selection") >"$hex" 2>"$manifest"

  text2pcap -q -u 5004,5004 "$hex" "$pcap" 2>/dev/null

  # Ask tshark for the frame type and mode request it reads out of each
  # payload. Two `-d` rules are needed and the second is the one that matters:
  # a *dynamic* payload type carries no codec identity, so without an explicit
  # `rtp.pt==N,amr` the RTP dissector hands the payload to nobody and the whole
  # check silently reports zero packets. The `-o` lines set the AMR dissector's
  # global framing and variant, which is why one capture holds one combination.
  # Repeated occurrences of a field within one packet (two ToC entries in a
  # -2f payload) are comma-joined, matching the manifest's `ft=7,7` notation.
  # The last two fields are the dissector's own length accounting: either one
  # being non-empty means it disagreed with the ToC about where frames end.
  dissected="$(tshark -r "$pcap" \
      -d "udp.port==5004,rtp" \
      -d "rtp.pt==$pt,amr" \
      -o "rtp.heuristic_rtp:FALSE" \
      -o "amr.encoding.version:$version" \
      -o "amr.mode:$mode" \
      -E occurrence=a -E aggregator=, \
      -T fields -e "$ft" -e "$cmr" \
      -e amr.not_enough_data_for_frames -e amr.superfluous_data \
      2>/dev/null | grep -v '^\s*$' || true)"

  expected="$(grep -oE 'ft=[0-9,]+ cmr=(none|[0-9]+)' "$manifest" \
      | sed -E 's/ft=([0-9,]+) cmr=(none|[0-9]+)/\1 \2/')"

  count_expected="$(printf '%s\n' "$expected" | grep -c . || true)"
  count_seen="$(printf '%s\n' "$dissected" | grep -c . || true)"

  if [[ "$count_seen" -eq 0 ]]; then
    echo "FAIL $selection: tshark dissected no AMR payloads at all" >&2
    mismatch=$((mismatch + 1))
    continue
  fi
  if [[ "$count_seen" -ne "$count_expected" ]]; then
    echo "FAIL $selection: tshark read $count_seen payloads, we sent $count_expected" >&2
    mismatch=$((mismatch + 1))
    continue
  fi

  bad=0
  while IFS= read -r want && IFS= read -r got <&3; do
    want_ft="${want%% *}"
    want_cmr="${want##* }"
    got_ft="$(printf '%s' "$got" | cut -f1)"
    got_cmr="$(printf '%s' "$got" | cut -f2)"
    got_short="$(printf '%s' "$got" | cut -f3)"
    got_extra="$(printf '%s' "$got" | cut -f4)"
    # A packet with no mode request carries CMR 15, which is what a conforming
    # dissector reports; our manifest calls that "none".
    [[ "$want_cmr" == "none" ]] && want_cmr=15
    if [[ "$got_ft" != "$want_ft" || "$got_cmr" != "$want_cmr" ]]; then
      echo "FAIL $selection: sent ft=$want_ft cmr=$want_cmr, tshark read ft=$got_ft cmr=$got_cmr" >&2
      bad=$((bad + 1))
    fi
    case "$selection" in
      *-oa-2f)
        # The documented tshark phantom-frame defect: the complaint must be
        # PRESENT. If a fixed tshark stops complaining, this fails loudly and
        # the workaround gets deleted rather than rotting.
        if [[ -z "$got_short" ]]; then
          echo "FAIL $selection: tshark no longer under-counts octet-aligned multi-frame payloads; remove the phantom-frame workaround" >&2
          bad=$((bad + 1))
        fi
        ;;
      *)
        if [[ -n "$got_short" ]]; then
          echo "FAIL $selection: dissector says the payload is short of its ToC (short='$got_short') for ft=$want_ft" >&2
          bad=$((bad + 1))
        fi
        ;;
    esac
    if [[ -n "$got_extra" ]]; then
      echo "FAIL $selection: dissector found data beyond the ToC (extra='$got_extra') for ft=$want_ft" >&2
      bad=$((bad + 1))
    fi
    total=$((total + 1))
  done < <(printf '%s\n' "$expected") 3< <(printf '%s\n' "$dissected")

  if [[ "$bad" -eq 0 ]]; then
    echo "  ok   $selection: $count_seen payloads, frame type and CMR agree"
  else
    mismatch=$((mismatch + bad))
  fi
done

echo
if [[ "$total" -eq 0 ]]; then
  echo "no payloads were compared; this is a failure, not a pass" >&2
  exit 2
fi
if [[ "$mismatch" -ne 0 ]]; then
  echo "$mismatch of $total payloads disagreed with Wireshark's dissector" >&2
  exit 1
fi
echo "$total payloads: our RFC 4867 framing matches Wireshark's dissector"
