#!/usr/bin/env bash
set -euo pipefail

readonly LIBSRTP_VERSION="2.8.0"
readonly LIBSRTP_COMMIT="24b3bf8f19b6f5ab4cd2bcceb4f4064efca86fd5"
readonly LIBSRTP_REPOSITORY="https://github.com/cisco/libsrtp.git"

RVOIP_REPOSITORY_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INTEROP_WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/rvoip-libsrtp-interop.XXXXXX")"
LIBSRTP_SOURCE_DIR="${RVOIP_LIBSRTP_SOURCE_DIR:-${INTEROP_WORK_DIR}/libsrtp}"
LIBSRTP_BUILD_DIR="${INTEROP_WORK_DIR}/build"
LIBSRTP_DRIVER="${INTEROP_WORK_DIR}/libsrtp_driver"

cleanup() {
    rm -rf "${INTEROP_WORK_DIR}"
}
trap cleanup EXIT

if [[ -n "${RVOIP_LIBSRTP_SOURCE_DIR:-}" ]]; then
    if [[ ! -d "${LIBSRTP_SOURCE_DIR}/.git" ]]; then
        echo "RVOIP_LIBSRTP_SOURCE_DIR is not a Git checkout: ${LIBSRTP_SOURCE_DIR}" >&2
        exit 1
    fi
else
    git clone --quiet --no-checkout --filter=blob:none "${LIBSRTP_REPOSITORY}" "${LIBSRTP_SOURCE_DIR}"
    git -C "${LIBSRTP_SOURCE_DIR}" fetch --quiet --depth 1 origin "${LIBSRTP_COMMIT}"
    git -C "${LIBSRTP_SOURCE_DIR}" checkout --quiet --detach FETCH_HEAD
fi

actual_commit="$(git -C "${LIBSRTP_SOURCE_DIR}" rev-parse HEAD)"
if [[ "${actual_commit}" != "${LIBSRTP_COMMIT}" ]]; then
    echo "libSRTP source is not the pinned commit" >&2
    echo "expected: ${LIBSRTP_COMMIT}" >&2
    echo "actual:   ${actual_commit}" >&2
    exit 1
fi

cmake \
    -S "${LIBSRTP_SOURCE_DIR}" \
    -B "${LIBSRTP_BUILD_DIR}" \
    -DCMAKE_BUILD_TYPE=Release \
    -DBUILD_SHARED_LIBS=OFF \
    -DENABLE_OPENSSL=OFF \
    -DENABLE_MBEDTLS=OFF \
    -DENABLE_NSS=OFF \
    -DENABLE_WARNINGS_AS_ERRORS=OFF \
    -DLIBSRTP_TEST_APPS=OFF \
    >/dev/null
cmake --build "${LIBSRTP_BUILD_DIR}" --parallel >/dev/null

"${CC:-cc}" \
    -std=c99 \
    -Wall \
    -Wextra \
    -Werror \
    -I"${LIBSRTP_SOURCE_DIR}/include" \
    -I"${LIBSRTP_BUILD_DIR}" \
    "${RVOIP_REPOSITORY_ROOT}/crates/media/rtp-core/tests/interop/libsrtp_driver.c" \
    "${LIBSRTP_BUILD_DIR}/libsrtp2.a" \
    -o "${LIBSRTP_DRIVER}"

reported_version="$("${LIBSRTP_DRIVER}" version)"
if [[ "${reported_version}" != "libsrtp2 ${LIBSRTP_VERSION}" ]]; then
    echo "unexpected libSRTP runtime version: ${reported_version}" >&2
    exit 1
fi

rvoip_driver() {
    cargo run \
        --locked \
        --quiet \
        --manifest-path "${RVOIP_REPOSITORY_ROOT}/Cargo.toml" \
        -p rvoip-rtp-core \
        --example libsrtp_interop_driver \
        -- "$@"
}

for profile in sha1-80 sha1-32 aes256-sha1-80 aes256-sha1-32; do
    rvoip_srtp="$(rvoip_driver "${profile}" protect-rtp)"
    [[ "$("${LIBSRTP_DRIVER}" "${profile}" unprotect-rtp "${rvoip_srtp}")" == "ok" ]]

    libsrtp_srtp="$("${LIBSRTP_DRIVER}" "${profile}" protect-rtp)"
    [[ "$(rvoip_driver "${profile}" unprotect-rtp "${libsrtp_srtp}")" == "ok" ]]
    [[ "${rvoip_srtp}" == "${libsrtp_srtp}" ]]

    rvoip_srtcp="$(rvoip_driver "${profile}" protect-rtcp)"
    [[ "$("${LIBSRTP_DRIVER}" "${profile}" unprotect-rtcp "${rvoip_srtcp}")" == "ok" ]]

    libsrtp_srtcp="$("${LIBSRTP_DRIVER}" "${profile}" protect-rtcp)"
    [[ "$(rvoip_driver "${profile}" unprotect-rtcp "${libsrtp_srtcp}")" == "ok" ]]
    [[ "${rvoip_srtcp}" == "${libsrtp_srtcp}" ]]

    rvoip_rollover="$(rvoip_driver "${profile}" protect-rtp-rollover)"
    [[ "$("${LIBSRTP_DRIVER}" "${profile}" unprotect-rtp-rollover "${rvoip_rollover}")" == "ok" ]]

    libsrtp_rollover="$("${LIBSRTP_DRIVER}" "${profile}" protect-rtp-rollover)"
    [[ "$(rvoip_driver "${profile}" unprotect-rtp-rollover "${libsrtp_rollover}")" == "ok" ]]
    [[ "${rvoip_rollover}" == "${libsrtp_rollover}" ]]
done

echo "SRTP/SRTCP interoperability passed against libSRTP ${LIBSRTP_VERSION} (${LIBSRTP_COMMIT})."
echo "Verified rvoip -> libSRTP and libSRTP -> rvoip for RTP, RTCP, and RTP rollover with AES-128/AES-256 SHA1-80 and SHA1-32."
