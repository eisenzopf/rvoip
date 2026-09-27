#!/usr/bin/env bash
set -Eeuo pipefail

LOG=/var/log/rvoip-performance-prebuild.log
exec > >(tee -a "$LOG") 2>&1

# The controller's user-data writes /etc/rvoip-release.env (mode 0600) before
# it runs this script. The builder reads the same keys as a release worker
# plus RVOIP_PREBUILD_CACHE_KEY.
set -a
# shellcheck source=/dev/null
source /etc/rvoip-release.env
set +a

CANDIDATE="$RVOIP_CANDIDATE"
RUN_ID="$RVOIP_RUN_ID"
BUCKET="$RVOIP_EVIDENCE_BUCKET"
CACHE_BUCKET="$RVOIP_CACHE_BUCKET"
PREFIX="$RVOIP_PREFIX"
CACHE_KEY="$RVOIP_PREBUILD_CACHE_KEY"
GATES="$(printf '%s' "$RVOIP_GATES_B64" | base64 --decode)"
ENVIRONMENT_ID="$(printf '%s' "$RVOIP_ENVIRONMENT_B64" | base64 --decode)"
AWS_REGION="$RVOIP_AWS_REGION"
export AWS_REGION
export AWS_DEFAULT_REGION="$AWS_REGION"
WORKSPACE=/opt/rvoip
BUNDLE_ROOT=/tmp/performance-prebuilt
BUNDLE=/tmp/performance-prebuilt.tar.gz
RESULT=/tmp/prebuild-result.json
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
START_SECONDS="$(date +%s)"
BUNDLE_SHA=""
MANIFEST_SHA=""
CACHE_PREFIX="release-cache/performance-prebuilt-v1/${CACHE_KEY}"
BUNDLE_URI=""
MANIFEST_URI=""
S3_CP_ATTEMPTS=3

# Evidence transfers use the AWS CLI with the runner instance role; there is
# no explicit credential anywhere on the builder. The CLI already retries
# throttled and transient API errors internally; the outer loop covers a
# whole-command failure such as a dropped connection mid-stream.
upload() {
  local source="$1"
  local object="$2"
  local attempt
  for attempt in $(seq 1 "$S3_CP_ATTEMPTS"); do
    if aws s3 cp --only-show-errors "$source" "s3://${BUCKET}/${object}"; then
      return 0
    fi
    echo "upload attempt ${attempt}/${S3_CP_ATTEMPTS} failed: ${object}" >&2
    if (( attempt < S3_CP_ATTEMPTS )); then
      sleep "$(( attempt * 5 ))"
    fi
  done
  return 1
}

download() {
  local object="$1"
  local destination="$2"
  local attempt
  for attempt in $(seq 1 "$S3_CP_ATTEMPTS"); do
    if aws s3 cp --only-show-errors "s3://${BUCKET}/${object}" "$destination"; then
      return 0
    fi
    echo "download attempt ${attempt}/${S3_CP_ATTEMPTS} failed: ${object}" >&2
    if (( attempt < S3_CP_ATTEMPTS )); then
      sleep "$(( attempt * 5 ))"
    fi
  done
  return 1
}

ensure_content_addressed() {
  local source="$1"
  local object="$2"
  local expected_sha="$3"
  local readback
  readback="$(mktemp /tmp/rvoip-prebuilt-object.XXXXXX)"

  # The evidence bucket is versioned and an interrupted or concurrent builder
  # may already have created this digest path. Verify those bytes instead of
  # writing a second version of an object that must stay content-addressed.
  if download "$object" "$readback" >/dev/null 2>&1; then
    if echo "${expected_sha}  ${readback}" | sha256sum --check --status; then
      rm -f "$readback"
      return 0
    fi
    rm -f "$readback"
    echo "content-addressed object digest mismatch: ${object}" >&2
    return 1
  fi
  rm -f "$readback"

  if upload "$source" "$object"; then
    return 0
  fi

  # A second builder can win after the first read and before this upload.
  readback="$(mktemp /tmp/rvoip-prebuilt-object.XXXXXX)"
  if download "$object" "$readback" >/dev/null 2>&1 \
    && echo "${expected_sha}  ${readback}" | sha256sum --check --status; then
    rm -f "$readback"
    return 0
  fi
  rm -f "$readback"
  echo "unable to create or verify content-addressed object: ${object}" >&2
  return 1
}

finish() {
  local exit_code=$?
  local ended_at duration status
  trap - EXIT
  ended_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  duration="$(( $(date +%s) - START_SECONDS ))"
  status=FAIL
  if (( exit_code == 0 )); then
    status=PASS
  fi
  if [[ "${RVOIP_SCCACHE_ACTIVE:-0}" == "1" ]]; then
    {
      sccache --show-stats
      sccache --stop-server
    } > /tmp/prebuild-sccache-stats.txt 2>&1 || true
    upload /tmp/prebuild-sccache-stats.txt "${PREFIX}/prebuild-sccache-stats.txt" || true
  fi
  python3 - "$RESULT" "$CANDIDATE" "$RUN_ID" "$ENVIRONMENT_ID" "$GATES" \
    "$CACHE_KEY" \
    "$STARTED_AT" "$ended_at" "$duration" "$exit_code" "$status" \
    "$BUNDLE_URI" "$BUNDLE_SHA" "$MANIFEST_URI" "$MANIFEST_SHA" <<'PY'
import json
import sys

(
    path,
    candidate,
    run_id,
    environment_id,
    gates,
    cache_key,
    started,
    ended,
    duration,
    code,
    status,
    bundle_uri,
    bundle_sha,
    manifest_uri,
    manifest_sha,
) = sys.argv[1:]
payload = {
    "schema": "rvoip-ec2-performance-prebuild-result-v1",
    "candidate_sha": candidate,
    "github_run_id": run_id,
    "environment_id": environment_id,
    "selected_gate_ids": sorted({value for value in gates.split(",") if value}),
    "cache_key_sha256": cache_key,
    "started_at": started,
    "ended_at": ended,
    "duration_seconds": int(duration),
    "exit_code": int(code),
    "status": status,
    "bundle_uri": bundle_uri if bundle_sha else None,
    "bundle_sha256": bundle_sha or None,
    "manifest_uri": manifest_uri if manifest_sha else None,
    "manifest_sha256": manifest_sha or None,
    "publishing_attempted": False,
}
with open(path, "w", encoding="utf-8") as handle:
    json.dump(payload, handle, indent=2, sort_keys=True)
    handle.write("\n")
PY
  upload "$LOG" "${PREFIX}/prebuild.log" || true
  upload "$RESULT" "${PREFIX}/prebuild-result.json" || true
  # The run-scoped result remains authoritative for this invocation. Publish
  # the cache pointer only after every content-addressed object exists and the
  # build has passed, so interrupted builders can never create a false hit.
  if (( exit_code == 0 )); then
    upload "$RESULT" "${CACHE_PREFIX}/prebuild-result.json" || true
  fi
  sync
  # The builder was created with InstanceInitiatedShutdownBehavior=terminate
  # and a delete-on-termination root volume, so this halt is its termination.
  shutdown -h now || true
  exit "$exit_code"
}
trap finish EXIT

# Every evidence transfer, including the EXIT-trap result upload, goes through
# the AWS CLI, so it is the first thing installed. The archive is the pinned
# official build and its detached PGP signature must verify against the AWS
# CLI Team public key embedded below (fingerprint
# FB5D B77F D5C1 18B8 0511 ADA8 A631 0ACC 4672 475C, expires 2027-07-01).
# Any download, key, or signature problem fails closed before the CLI is used.
AWS_CLI_VERSION=2.37.4
AWS_CLI_ARCHIVE="awscli-exe-linux-x86_64-${AWS_CLI_VERSION}.zip"
AWS_CLI_URL="https://awscli.amazonaws.com/${AWS_CLI_ARCHIVE}"
AWS_CLI_KEY_FINGERPRINT=FB5DB77FD5C118B80511ADA8A6310ACC4672475C
install_aws_cli() {
  local workdir
  workdir="$(mktemp -d /tmp/rvoip-awscli.XXXXXX)"
  cat > "$workdir/aws-cli-team.asc" <<'KEY'
-----BEGIN PGP PUBLIC KEY BLOCK-----

mQINBF2Cr7UBEADJZHcgusOJl7ENSyumXh85z0TRV0xJorM2B/JL0kHOyigQluUG
ZMLhENaG0bYatdrKP+3H91lvK050pXwnO/R7fB/FSTouki4ciIx5OuLlnJZIxSzx
PqGl0mkxImLNbGWoi6Lto0LYxqHN2iQtzlwTVmq9733zd3XfcXrZ3+LblHAgEt5G
TfNxEKJ8soPLyWmwDH6HWCnjZ/aIQRBTIQ05uVeEoYxSh6wOai7ss/KveoSNBbYz
gbdzoqI2Y8cgH2nbfgp3DSasaLZEdCSsIsK1u05CinE7k2qZ7KgKAUIcT/cR/grk
C6VwsnDU0OUCideXcQ8WeHutqvgZH1JgKDbznoIzeQHJD238GEu+eKhRHcz8/jeG
94zkcgJOz3KbZGYMiTh277Fvj9zzvZsbMBCedV1BTg3TqgvdX4bdkhf5cH+7NtWO
lrFj6UwAsGukBTAOxC0l/dnSmZhJ7Z1KmEWilro/gOrjtOxqRQutlIqG22TaqoPG
fYVN+en3Zwbt97kcgZDwqbuykNt64oZWc4XKCa3mprEGC3IbJTBFqglXmZ7l9ywG
EEUJYOlb2XrSuPWml39beWdKM8kzr1OjnlOm6+lpTRCBfo0wa9F8YZRhHPAkwKkX
XDeOGpWRj4ohOx0d2GWkyV5xyN14p2tQOCdOODmz80yUTgRpPVQUtOEhXQARAQAB
tCFBV1MgQ0xJIFRlYW0gPGF3cy1jbGlAYW1hem9uLmNvbT6JAlQEEwEIAD4CGwMF
CwkIBwIGFQoJCAsCBBYCAwECHgECF4AWIQT7Xbd/1cEYuAURraimMQrMRnJHXAUC
akV0ygUJDqP4lQAKCRCmMQrMRnJHXFHjD/9eyZLYcKuQOlLvtqSDtUBiEZf6ZZjM
i3ygYH8rJNtuToUH+HvSpe819urJCquXhDrlK6N+aqW0hCLtNABJG/vsafIgvIYJ
hSGgpgtNnQyMV1jViRWqPjbouw8OkYKBThUfT1i2Y+wn58ifs6ODBCmTexWtXspA
Si+Gt49xDOW0APmbOPnI+a4HJW6tVEo6MWS0WjzpiBayR3d1A4pt4YrPfSdDgpLo
h2SLQqlRqvvVZJaWBjhkErNFpfsBA06sDcPEOb0G8LBUbR4WOcdvhe5LubJbZuxC
AG9kNPCVeQP1ixwjgjXKysaxeQ6rv0VzIQgRp6tLVLWhy6AKDNvLjFSsmXZ1Wl08
Y/RlOHXlzLuQMRE6sR1wOdRxc9TsrNWTGiBK65cvSWOy03JeBkQQ8pesqltiyxI9
U21kkgiXtTSKNGfKK8pO27D81YANhRqPK7iTp6kuFiY2WtOg90KTMNlIT+Ff85Y2
b1rHj6Z0SrCkJujhWk3IBPic/wJgz01LEc/OAdUPlby90RJZcIBhSlWhT7mXnXIO
c0HWlNQrns2s3CTyYwZSiSlYe9ApeLwhjDo8NhbFuCAy61l6O5UsR4AfZxx/rGKv
2wFb1/RN/P4gNe6vmxZAPjR0AQcwD3tc2McimOLr/22kmPz8IH3I0X7WoSFr0Biz
E91G7bb0hOb/cA==
=knv7
-----END PGP PUBLIC KEY BLOCK-----
KEY
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error \
    --retry 3 --retry-all-errors \
    "$AWS_CLI_URL" -o "$workdir/awscliv2.zip"
  curl --proto '=https' --tlsv1.2 --fail --silent --show-error \
    --retry 3 --retry-all-errors \
    "${AWS_CLI_URL}.sig" -o "$workdir/awscliv2.sig"
  mkdir -m 0700 "$workdir/gnupg"
  gpg --homedir "$workdir/gnupg" --batch --quiet \
    --import "$workdir/aws-cli-team.asc"
  # The scratch keyring must contain exactly the published AWS CLI key.
  gpg --homedir "$workdir/gnupg" --batch --with-colons --fingerprint \
    > "$workdir/keyring-fingerprints"
  test "$(grep -c '^fpr:' "$workdir/keyring-fingerprints")" -eq 1
  grep -q "^fpr:::::::::${AWS_CLI_KEY_FINGERPRINT}:$" \
    "$workdir/keyring-fingerprints"
  # Require a VALIDSIG made by that key, not merely a zero exit status.
  gpg --homedir "$workdir/gnupg" --batch --status-fd 3 \
    --verify "$workdir/awscliv2.sig" "$workdir/awscliv2.zip" \
    3> "$workdir/verify-status"
  awk -v key="$AWS_CLI_KEY_FINGERPRINT" \
    '$1 == "[GNUPG:]" && $2 == "VALIDSIG" && ($3 == key || $NF == key) { ok = 1 }
     END { exit !ok }' "$workdir/verify-status"
  unzip -q "$workdir/awscliv2.zip" -d "$workdir"
  "$workdir/aws/install" --install-dir /usr/local/aws-cli --bin-dir /usr/local/bin
  test "$(aws --version | awk '{print $1}')" = "aws-cli/${AWS_CLI_VERSION}"
  gpgconf --homedir "$workdir/gnupg" --kill all >/dev/null 2>&1 || true
  rm -rf "$workdir"
}

export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends ca-certificates curl gnupg unzip
install_aws_cli
apt-get install -y --no-install-recommends \
  build-essential ca-certificates cmake curl git libasound2-dev libopus-dev \
  libssl-dev lld pigz pkg-config protobuf-compiler

curl --proto '=https' --tlsv1.2 --fail --silent --show-error \
  https://sh.rustup.rs -o /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain 1.91.0
export PATH=/root/.cargo/bin:$PATH
command -v ld.lld >/dev/null
ld.lld --version
# GNU ld dominates an otherwise cache-hot exact-candidate prebuild. Keep the
# linker choice explicit and release-environment-versioned so evidence built
# with a different linker can never be reused accidentally.
export RUSTFLAGS="-C link-arg=-fuse-ld=lld"

git clone --filter=blob:none https://github.com/eisenzopf/rvoip.git "$WORKSPACE"
cd "$WORKSPACE"
git fetch --depth=1 origin "$CANDIDATE"
git checkout --detach "$CANDIDATE"
test "$(git rev-parse HEAD)" = "$CANDIDATE"

SCCACHE_VERSION=0.15.0
SCCACHE_ARCHIVE="sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl.tar.gz"
SCCACHE_SHA256=782d2b5dd7ae0a55ebe368ab258114d0928d019ac2d949ab85d5d02f3926709e
SCCACHE_URL="https://github.com/mozilla/sccache/releases/download/v${SCCACHE_VERSION}/${SCCACHE_ARCHIVE}"
if curl --proto '=https' --tlsv1.2 --fail --silent --show-error --location \
    "$SCCACHE_URL" -o "/tmp/$SCCACHE_ARCHIVE" \
  && echo "$SCCACHE_SHA256  /tmp/$SCCACHE_ARCHIVE" | sha256sum --check --status \
  && tar -C /tmp -xzf "/tmp/$SCCACHE_ARCHIVE" \
  && install -m 0755 \
    "/tmp/sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl/sccache" \
    /usr/local/bin/sccache; then
  export CARGO_INCREMENTAL=0
  export RUSTC_WRAPPER=sccache
  export SCCACHE_BASEDIRS="$WORKSPACE"
  export SCCACHE_CACHE_SIZE=40G
  export SCCACHE_DIR=/var/cache/rvoip-sccache
  # sccache authenticates to S3 through the same instance role as the CLI.
  export SCCACHE_BUCKET="$CACHE_BUCKET"
  export SCCACHE_REGION="$AWS_REGION"
  export SCCACHE_S3_KEY_PREFIX=rvoip-release-v2-lld/rust-1.91.0/x86_64-unknown-linux-gnu
  export SCCACHE_S3_USE_SSL=true
  export SCCACHE_IDLE_TIMEOUT=0
  export SCCACHE_MULTILEVEL_CHAIN=disk,s3
  export SCCACHE_MULTILEVEL_WRITE_ERROR_POLICY=ignore
  if sccache --start-server; then
    export RVOIP_SCCACHE_ACTIVE=1
    echo "shared S3 compiler cache enabled"
  else
    unset RUSTC_WRAPPER
    echo "shared compiler cache unavailable; continuing with direct rustc" >&2
  fi
fi

echo "building exact-candidate performance executables once on ${CANDIDATE}"
python3 scripts/release/prebuilt_performance.py build \
  --workspace "$WORKSPACE" \
  --catalog "$WORKSPACE/scripts/release/gates.json" \
  --gates "$GATES" \
  --candidate "$CANDIDATE" \
  --environment-id "$ENVIRONMENT_ID" \
  --output "$BUNDLE_ROOT"

MANIFEST_SHA="$(sha256sum "$BUNDLE_ROOT/manifest.json" | awk '{print $1}')"
tar -C /tmp -I 'pigz -3' -cf "$BUNDLE" performance-prebuilt
BUNDLE_SHA="$(sha256sum "$BUNDLE" | awk '{print $1}')"
BUNDLE_OBJECT="${CACHE_PREFIX}/bundles/${BUNDLE_SHA}.tar.gz"
MANIFEST_OBJECT="${CACHE_PREFIX}/manifests/${MANIFEST_SHA}.json"
BUNDLE_URI="s3://${BUCKET}/${BUNDLE_OBJECT}"
MANIFEST_URI="s3://${BUCKET}/${MANIFEST_OBJECT}"
ensure_content_addressed "$BUNDLE_ROOT/manifest.json" "$MANIFEST_OBJECT" "$MANIFEST_SHA"
ensure_content_addressed "$BUNDLE" "$BUNDLE_OBJECT" "$BUNDLE_SHA"

# Prove that the runner instance role can read evidence before terminating the
# builder and creating the measurement fleet. Put-only IAM otherwise fails one
# instance later and obscures an infrastructure defect as a gate failure.
download "$MANIFEST_OBJECT" /tmp/performance-manifest-readback.json
echo "${MANIFEST_SHA}  /tmp/performance-manifest-readback.json" \
  | sha256sum --check --status
