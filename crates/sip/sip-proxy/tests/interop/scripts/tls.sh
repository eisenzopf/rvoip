#!/usr/bin/env bash

# TLS helpers for the real Kamailio/OpenSIPS proxy matrix.
#
# This file is sourced by run.sh. It deliberately keeps private keys in a
# mode-0700 directory outside the packaged artifact tree. Only public
# certificates and the unmodified output of verification tools are copied
# into row evidence.

PROXY_INTEROP_TLS_SCRIPT_DIR=$(
  CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd
)
PROXY_INTEROP_TLS_RVOIP_IDENTITY=${PROXY_INTEROP_TLS_RVOIP_IDENTITY:-rvoip.proxy.test}
PROXY_INTEROP_TLS_SIPP_IDENTITY=${PROXY_INTEROP_TLS_SIPP_IDENTITY:-sipp.proxy.test}
PROXY_INTEROP_TLS_OPENSSL=${PROXY_INTEROP_TLS_OPENSSL:-openssl}

tls_sha256_stdin() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256
  else
    "$PROXY_INTEROP_TLS_OPENSSL" dgst -sha256 | sed 's/^.*= //'
  fi
}

tls_peer_identity() {
  case "$1" in
    kamailio) printf '%s\n' "${PROXY_INTEROP_TLS_KAMAILIO_IDENTITY:-kamailio.proxy.test}" ;;
    opensips) printf '%s\n' "${PROXY_INTEROP_TLS_OPENSIPS_IDENTITY:-opensips.proxy.test}" ;;
    *)
      echo "unsupported TLS peer: $1" >&2
      return 2
      ;;
  esac
}

tls_leaf_serial() {
  case "$1" in
    rvoip) printf '%s\n' 1001 ;;
    kamailio) printf '%s\n' 1002 ;;
    opensips) printf '%s\n' 1003 ;;
    sipp) printf '%s\n' 1004 ;;
    *) return 2 ;;
  esac
}

tls_generate_leaf() {
  local name=$1
  local identity=$2
  local serial
  serial=$(tls_leaf_serial "$name")

  "$PROXY_INTEROP_TLS_OPENSSL" req \
    -new \
    -newkey rsa:2048 \
    -nodes \
    -sha256 \
    -subj "/CN=$identity/O=rvoip proxy interoperability" \
    -addext "subjectAltName=DNS:$identity" \
    -keyout "$PROXY_INTEROP_TLS_PKI_DIR/$name.key.pem" \
    -out "$PROXY_INTEROP_TLS_PKI_DIR/$name.csr.pem" \
    >"$PROXY_INTEROP_TLS_PKI_DIR/$name.req.log" 2>&1

  {
    echo "basicConstraints=critical,CA:FALSE"
    echo "keyUsage=critical,digitalSignature,keyEncipherment"
    echo "extendedKeyUsage=serverAuth,clientAuth"
    echo "subjectAltName=DNS:$identity"
  } >"$PROXY_INTEROP_TLS_PKI_DIR/$name.ext"

  "$PROXY_INTEROP_TLS_OPENSSL" x509 \
    -req \
    -sha256 \
    -days 2 \
    -CA "$PROXY_INTEROP_TLS_PKI_DIR/ca.pem" \
    -CAkey "$PROXY_INTEROP_TLS_PKI_DIR/ca.key.pem" \
    -set_serial "$serial" \
    -extfile "$PROXY_INTEROP_TLS_PKI_DIR/$name.ext" \
    -in "$PROXY_INTEROP_TLS_PKI_DIR/$name.csr.pem" \
    -out "$PROXY_INTEROP_TLS_PKI_DIR/$name.pem" \
    >"$PROXY_INTEROP_TLS_PKI_DIR/$name.sign.log" 2>&1
}

tls_generate_untrusted_client() {
  local identity=untrusted-client.proxy.test
  "$PROXY_INTEROP_TLS_OPENSSL" req \
    -new \
    -newkey rsa:2048 \
    -nodes \
    -sha256 \
    -subj "/CN=$identity/O=rvoip negative control" \
    -addext "subjectAltName=DNS:$identity" \
    -keyout "$PROXY_INTEROP_TLS_PKI_DIR/untrusted-client.key.pem" \
    -out "$PROXY_INTEROP_TLS_PKI_DIR/untrusted-client.csr.pem" \
    >"$PROXY_INTEROP_TLS_PKI_DIR/untrusted-client.req.log" 2>&1

  {
    echo "basicConstraints=critical,CA:FALSE"
    echo "keyUsage=critical,digitalSignature,keyEncipherment"
    echo "extendedKeyUsage=clientAuth"
    echo "subjectAltName=DNS:$identity"
  } >"$PROXY_INTEROP_TLS_PKI_DIR/untrusted-client.ext"

  "$PROXY_INTEROP_TLS_OPENSSL" x509 \
    -req \
    -sha256 \
    -days 2 \
    -CA "$PROXY_INTEROP_TLS_PKI_DIR/untrusted-ca.pem" \
    -CAkey "$PROXY_INTEROP_TLS_PKI_DIR/untrusted-ca.key.pem" \
    -set_serial 2001 \
    -extfile "$PROXY_INTEROP_TLS_PKI_DIR/untrusted-client.ext" \
    -in "$PROXY_INTEROP_TLS_PKI_DIR/untrusted-client.csr.pem" \
    -out "$PROXY_INTEROP_TLS_PKI_DIR/untrusted-client.pem" \
    >"$PROXY_INTEROP_TLS_PKI_DIR/untrusted-client.sign.log" 2>&1
}

tls_prepare_pki() {
  local workspace_root=$1
  local pki_parent="$workspace_root/target"

  command -v "$PROXY_INTEROP_TLS_OPENSSL" >/dev/null 2>&1 || {
    echo "OpenSSL is required for verified TLS interoperability" >&2
    return 1
  }
  mkdir -p "$pki_parent"
  PROXY_INTEROP_TLS_PKI_DIR=$(mktemp -d "$pki_parent/.proxy-interop-pki.XXXXXX")
  export PROXY_INTEROP_TLS_PKI_DIR
  chmod 0700 "$PROXY_INTEROP_TLS_PKI_DIR"
  umask 077

  "$PROXY_INTEROP_TLS_OPENSSL" req \
    -x509 \
    -newkey rsa:3072 \
    -nodes \
    -sha256 \
    -days 2 \
    -subj "/CN=rvoip proxy interoperability test CA/O=rvoip" \
    -addext "basicConstraints=critical,CA:TRUE,pathlen:0" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" \
    -keyout "$PROXY_INTEROP_TLS_PKI_DIR/ca.key.pem" \
    -out "$PROXY_INTEROP_TLS_PKI_DIR/ca.pem" \
    >"$PROXY_INTEROP_TLS_PKI_DIR/ca.req.log" 2>&1

  "$PROXY_INTEROP_TLS_OPENSSL" req \
    -x509 \
    -newkey rsa:2048 \
    -nodes \
    -sha256 \
    -days 2 \
    -subj "/CN=untrusted proxy interoperability test CA/O=rvoip" \
    -addext "basicConstraints=critical,CA:TRUE,pathlen:0" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" \
    -keyout "$PROXY_INTEROP_TLS_PKI_DIR/untrusted-ca.key.pem" \
    -out "$PROXY_INTEROP_TLS_PKI_DIR/untrusted-ca.pem" \
    >"$PROXY_INTEROP_TLS_PKI_DIR/untrusted-ca.req.log" 2>&1

  tls_generate_leaf rvoip "$PROXY_INTEROP_TLS_RVOIP_IDENTITY"
  tls_generate_leaf kamailio "$(tls_peer_identity kamailio)"
  tls_generate_leaf opensips "$(tls_peer_identity opensips)"
  tls_generate_leaf sipp "$PROXY_INTEROP_TLS_SIPP_IDENTITY"
  tls_generate_untrusted_client

  chmod 0600 "$PROXY_INTEROP_TLS_PKI_DIR"/*.key.pem
  chmod 0644 "$PROXY_INTEROP_TLS_PKI_DIR"/*.pem

  export PROXY_INTEROP_TLS_CA="$PROXY_INTEROP_TLS_PKI_DIR/ca.pem"
  export PROXY_INTEROP_TLS_RVOIP_CERT="$PROXY_INTEROP_TLS_PKI_DIR/rvoip.pem"
  export PROXY_INTEROP_TLS_RVOIP_KEY="$PROXY_INTEROP_TLS_PKI_DIR/rvoip.key.pem"
  export PROXY_INTEROP_TLS_SIPP_CERT="$PROXY_INTEROP_TLS_PKI_DIR/sipp.pem"
  export PROXY_INTEROP_TLS_SIPP_KEY="$PROXY_INTEROP_TLS_PKI_DIR/sipp.key.pem"
}

tls_select_peer() {
  local peer=$1
  PROXY_INTEROP_TLS_PEER_IDENTITY=$(tls_peer_identity "$peer")
  PROXY_INTEROP_TLS_PEER_CERT="$PROXY_INTEROP_TLS_PKI_DIR/$peer.pem"
  PROXY_INTEROP_TLS_PEER_KEY="$PROXY_INTEROP_TLS_PKI_DIR/$peer.key.pem"
  export PROXY_INTEROP_TLS_PEER_IDENTITY
  export PROXY_INTEROP_TLS_PEER_CERT
  export PROXY_INTEROP_TLS_PEER_KEY
}

tls_upstream_identity() {
  case "$1" in
    rvoip-first) printf '%s\n' "$PROXY_INTEROP_TLS_RVOIP_IDENTITY" ;;
    peer-first) printf '%s\n' "$PROXY_INTEROP_TLS_SIPP_IDENTITY" ;;
    *)
      echo "unsupported TLS adjacency order: $1" >&2
      return 2
      ;;
  esac
}

tls_next_hop_identity() {
  case "$1" in
    rvoip-first) printf '%s\n' "$PROXY_INTEROP_TLS_SIPP_IDENTITY" ;;
    peer-first) printf '%s\n' "$PROXY_INTEROP_TLS_RVOIP_IDENTITY" ;;
    *)
      echo "unsupported TLS adjacency order: $1" >&2
      return 2
      ;;
  esac
}

tls_first_proxy_identity() {
  case "$1" in
    rvoip-first) printf '%s\n' "$PROXY_INTEROP_TLS_RVOIP_IDENTITY" ;;
    peer-first) printf '%s\n' "$PROXY_INTEROP_TLS_PEER_IDENTITY" ;;
    *)
      echo "unsupported TLS adjacency order: $1" >&2
      return 2
      ;;
  esac
}

tls_last_proxy_identity() {
  case "$1" in
    rvoip-first) printf '%s\n' "$PROXY_INTEROP_TLS_PEER_IDENTITY" ;;
    peer-first) printf '%s\n' "$PROXY_INTEROP_TLS_RVOIP_IDENTITY" ;;
    *)
      echo "unsupported TLS adjacency order: $1" >&2
      return 2
      ;;
  esac
}

tls_render_peer_config() {
  local interop_dir=$1
  local peer=$2
  local order=$3
  local public_host=$4
  local public_port=$5
  local next_hop_host=$6
  local next_hop_port=$7
  local output=$8
  local kamailio_domain_output=$9
  local tcp_egress_port=${10}
  local upstream_identity

  tls_select_peer "$peer"
  upstream_identity=$(tls_upstream_identity "$order")

  sed \
    -e "s|__PUBLIC_HOST__|$public_host|g" \
    -e "s|__PUBLIC_PORT__|$public_port|g" \
    -e "s|__NEXT_HOP_HOST__|$next_hop_host|g" \
    -e "s|__NEXT_HOP_PORT__|$next_hop_port|g" \
    -e "s|__TCP_EGRESS_PORT__|$tcp_egress_port|g" \
    -e "s|__UPSTREAM_IDENTITY__|$upstream_identity|g" \
    -e "s|__PEER_CERT_BASENAME__|$peer.pem|g" \
    -e "s|__PEER_KEY_BASENAME__|$peer.key.pem|g" \
    "$interop_dir/config/$peer-tls.cfg.in" >"$output"

  sed \
    -e "s|__PEER_CERT_BASENAME__|$peer.pem|g" \
    -e "s|__PEER_KEY_BASENAME__|$peer.key.pem|g" \
    "$interop_dir/config/kamailio-tls-domain.cfg.in" >"$kamailio_domain_output"
  PROXY_INTEROP_KAMAILIO_TLS_DOMAIN_CONFIG=$kamailio_domain_output
  export PROXY_INTEROP_KAMAILIO_TLS_DOMAIN_CONFIG
}

tls_enable_compose_overlay() {
  local interop_dir=$1
  COMPOSE_FILE_ARGS+=(--file "$interop_dir/docker-compose.tls.yml")
  export PROXY_INTEROP_TLS_ENABLED=1
  export OPENSIPS_TLS_IMAGE=${OPENSIPS_TLS_IMAGE:-rvoip/opensips-tls-interop:3.6.7-1}
}

tls_disable_compose_overlay() {
  unset PROXY_INTEROP_TLS_ENABLED
}

tls_proxy_args() {
  TLS_PROXY_ARGS=(
    --certificate "$PROXY_INTEROP_TLS_RVOIP_CERT"
    --private-key "$PROXY_INTEROP_TLS_RVOIP_KEY"
    --ca-certificate "$PROXY_INTEROP_TLS_CA"
    --client-certificate "$PROXY_INTEROP_TLS_RVOIP_CERT"
    --client-private-key "$PROXY_INTEROP_TLS_RVOIP_KEY"
    --client-ca-certificate "$PROXY_INTEROP_TLS_CA"
  )
}

tls_copy_public_evidence() {
  local output_dir=$1
  local peer=$2
  local public_dir="$output_dir/tls-public"
  mkdir -p "$public_dir"
  chmod 0755 "$public_dir"

  cp "$PROXY_INTEROP_TLS_PKI_DIR/ca.pem" "$public_dir/ca.pem"
  cp "$PROXY_INTEROP_TLS_PKI_DIR/rvoip.pem" "$public_dir/rvoip.pem"
  cp "$PROXY_INTEROP_TLS_PKI_DIR/$peer.pem" "$public_dir/peer.pem"
  cp "$PROXY_INTEROP_TLS_PKI_DIR/sipp.pem" "$public_dir/sipp.pem"
  chmod 0644 "$public_dir"/*.pem

  for certificate in ca rvoip peer sipp; do
    "$PROXY_INTEROP_TLS_OPENSSL" x509 \
      -in "$public_dir/$certificate.pem" \
      -noout \
      -subject \
      -issuer \
      -serial \
      -dates \
      -ext subjectAltName \
      >"$public_dir/$certificate.metadata.txt" 2>&1
    "$PROXY_INTEROP_TLS_OPENSSL" x509 \
      -in "$public_dir/$certificate.pem" \
      -outform DER \
      | tls_sha256_stdin >"$public_dir/$certificate.der.sha256"
  done
}

tls_run_positive_handshake() {
  local destination=$1
  local expected_name=$2
  local client_certificate=$3
  local client_key=$4
  local output=$5

  if ! printf '' | "$PROXY_INTEROP_TLS_OPENSSL" s_client \
    -connect "$destination" \
    -servername "$expected_name" \
    -verify_hostname "$expected_name" \
    -verify_return_error \
    -CAfile "$PROXY_INTEROP_TLS_CA" \
    -cert "$client_certificate" \
    -key "$client_key" \
    -tls1_2 \
    -brief >"$output" 2>&1; then
    echo "verified TLS handshake failed: $destination as $expected_name" >&2
    return 1
  fi
  grep -Eq 'Verification: OK|Verify return code: 0 \\(ok\\)' "$output"
}

tls_run_wrong_name_control() {
  local destination=$1
  local expected_name=$2
  local client_certificate=$3
  local client_key=$4
  local output=$5
  local wrong_name="wrong-${expected_name}"

  if printf '' | "$PROXY_INTEROP_TLS_OPENSSL" s_client \
    -connect "$destination" \
    -servername "$expected_name" \
    -verify_hostname "$wrong_name" \
    -verify_return_error \
    -CAfile "$PROXY_INTEROP_TLS_CA" \
    -cert "$client_certificate" \
    -key "$client_key" \
    -tls1_2 \
    -brief >"$output" 2>&1; then
    echo "wrong-name TLS negative control unexpectedly succeeded: $destination" >&2
    return 1
  fi
  grep -Eqi 'hostname mismatch|does not match|verification error' "$output"
}

tls_run_wrong_ca_control() {
  local destination=$1
  local expected_name=$2
  local client_certificate=$3
  local client_key=$4
  local output=$5

  if printf '' | "$PROXY_INTEROP_TLS_OPENSSL" s_client \
    -connect "$destination" \
    -servername "$expected_name" \
    -verify_hostname "$expected_name" \
    -verify_return_error \
    -CAfile "$PROXY_INTEROP_TLS_PKI_DIR/untrusted-ca.pem" \
    -cert "$client_certificate" \
    -key "$client_key" \
    -tls1_2 \
    -brief >"$output" 2>&1; then
    echo "wrong-CA TLS negative control unexpectedly succeeded: $destination" >&2
    return 1
  fi
  grep -Eqi \
    'unable to get local issuer certificate|unable to verify|unknown ca|verification error|verify error|self-signed certificate|certificate verify failed' \
    "$output"
}

tls_run_untrusted_client_control() {
  local destination=$1
  local expected_name=$2
  local output=$3

  if printf '' | "$PROXY_INTEROP_TLS_OPENSSL" s_client \
    -connect "$destination" \
    -servername "$expected_name" \
    -verify_hostname "$expected_name" \
    -verify_return_error \
    -CAfile "$PROXY_INTEROP_TLS_CA" \
    -cert "$PROXY_INTEROP_TLS_PKI_DIR/untrusted-client.pem" \
    -key "$PROXY_INTEROP_TLS_PKI_DIR/untrusted-client.key.pem" \
    -tls1_2 \
    -brief >"$output" 2>&1; then
    echo "untrusted-client mTLS negative control unexpectedly succeeded: $destination" >&2
    return 1
  fi
  grep -Eqi \
    'unknown ca|certificate unknown|bad certificate|certificate verify failed|tlsv1 alert' \
    "$output"
}

tls_verify_live_endpoints() {
  local output_dir=$1
  local peer=$2
  local order=$3
  local peer_destination=$4
  local rvoip_destination=$5
  local peer_identity
  local peer_client_certificate
  local peer_client_key
  local rvoip_client_certificate
  local rvoip_client_key
  peer_identity=$(tls_peer_identity "$peer")
  case "$order" in
    rvoip-first)
      peer_client_certificate=$PROXY_INTEROP_TLS_RVOIP_CERT
      peer_client_key=$PROXY_INTEROP_TLS_RVOIP_KEY
      rvoip_client_certificate=$PROXY_INTEROP_TLS_SIPP_CERT
      rvoip_client_key=$PROXY_INTEROP_TLS_SIPP_KEY
      ;;
    peer-first)
      peer_client_certificate=$PROXY_INTEROP_TLS_SIPP_CERT
      peer_client_key=$PROXY_INTEROP_TLS_SIPP_KEY
      rvoip_client_certificate=$PROXY_INTEROP_TLS_PEER_CERT
      rvoip_client_key=$PROXY_INTEROP_TLS_PEER_KEY
      ;;
    *)
      echo "unsupported TLS adjacency order: $order" >&2
      return 2
      ;;
  esac

  tls_copy_public_evidence "$output_dir" "$peer"

  tls_run_positive_handshake \
    "$peer_destination" \
    "$peer_identity" \
    "$peer_client_certificate" \
    "$peer_client_key" \
    "$output_dir/tls-rvoip-to-peer-positive.log"
  tls_run_wrong_name_control \
    "$peer_destination" \
    "$peer_identity" \
    "$peer_client_certificate" \
    "$peer_client_key" \
    "$output_dir/tls-rvoip-to-peer-wrong-name.log"
  tls_run_wrong_ca_control \
    "$peer_destination" \
    "$peer_identity" \
    "$peer_client_certificate" \
    "$peer_client_key" \
    "$output_dir/tls-rvoip-to-peer-wrong-ca.log"
  tls_run_untrusted_client_control \
    "$peer_destination" \
    "$peer_identity" \
    "$output_dir/tls-peer-rejects-untrusted-client.log"

  tls_run_positive_handshake \
    "$rvoip_destination" \
    "$PROXY_INTEROP_TLS_RVOIP_IDENTITY" \
    "$rvoip_client_certificate" \
    "$rvoip_client_key" \
    "$output_dir/tls-peer-to-rvoip-positive.log"
  tls_run_wrong_name_control \
    "$rvoip_destination" \
    "$PROXY_INTEROP_TLS_RVOIP_IDENTITY" \
    "$rvoip_client_certificate" \
    "$rvoip_client_key" \
    "$output_dir/tls-peer-to-rvoip-wrong-name.log"
  tls_run_wrong_ca_control \
    "$rvoip_destination" \
    "$PROXY_INTEROP_TLS_RVOIP_IDENTITY" \
    "$rvoip_client_certificate" \
    "$rvoip_client_key" \
    "$output_dir/tls-peer-to-rvoip-wrong-ca.log"
  tls_run_untrusted_client_control \
    "$rvoip_destination" \
    "$PROXY_INTEROP_TLS_RVOIP_IDENTITY" \
    "$output_dir/tls-rvoip-rejects-untrusted-client.log"
}

tls_derive_verification_result() {
  local output_dir=$1
  local peer=$2
  local order=$3

  python3 "$PROXY_INTEROP_TLS_SCRIPT_DIR/verify_tls_evidence.py" \
    --row-dir "$output_dir" \
    --peer "$peer" \
    --order "$order" \
    --write-result "$output_dir/tls-verifier-result.json"
}

tls_capture_opensips_image_provenance() {
  local interop_dir=$1
  local container_id=$2
  local output_dir=$3

  python3 -B "$PROXY_INTEROP_TLS_SCRIPT_DIR/opensips_tls_provenance.py" \
    --container-id "$container_id" \
    --dockerfile "$interop_dir/images/opensips-tls/Dockerfile" \
    --output "$output_dir/opensips-tls-image-provenance.json"
}

tls_cleanup_pki() {
  if [[ -n "${PROXY_INTEROP_TLS_PKI_DIR:-}" \
    && "$PROXY_INTEROP_TLS_PKI_DIR" == */target/.proxy-interop-pki.* \
    && -d "$PROXY_INTEROP_TLS_PKI_DIR" ]]; then
    rm -rf -- "$PROXY_INTEROP_TLS_PKI_DIR"
  fi
  unset PROXY_INTEROP_TLS_PKI_DIR
}
