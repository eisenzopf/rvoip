#!/usr/bin/env sh
# Shared SIP TLS listener certificate helper for PBX interop examples.

pbx_tls_contact_mode() {
  provider=${PBX_PROVIDER:-${PBX:-asterisk}}
  case "$provider" in
    freeswitch|free-switch|fs)
      mode=$(printf '%s' "${FREESWITCH_TLS_CONTACT_MODE:-reachable-contact}" | tr '[:upper:]' '[:lower:]')
      flow_reuse=$(printf '%s' "${FREESWITCH_TLS_FLOW_REUSE:-0}" | tr '[:upper:]' '[:lower:]')
      ;;
    *)
      mode=$(printf '%s' "${ASTERISK_TLS_CONTACT_MODE:-reachable-contact}" | tr '[:upper:]' '[:lower:]')
      flow_reuse=$(printf '%s' "${ASTERISK_TLS_FLOW_REUSE:-0}" | tr '[:upper:]' '[:lower:]')
      ;;
  esac
  case "$flow_reuse" in
    1|true|yes|on) mode=registered-flow-symmetric ;;
  esac
  printf '%s\n' "$mode"
}

pbx_tls_uses_reachable_contact() {
  case "$(pbx_tls_contact_mode)" in
    reachable-contact|reachable|listener|uas) return 0 ;;
    *) return 1 ;;
  esac
}

pbx_is_ip_literal() {
  value=$1
  case "$value" in
    ""|0.0.0.0|::) return 1 ;;
    *:*) return 0 ;;
    *[!0-9.]*) return 1 ;;
    *.*) return 0 ;;
    *) return 1 ;;
  esac
}

pbx_generate_tls_cert() {
  cert_dir=$1
  provider=${PBX_PROVIDER:-${PBX:-asterisk}}

  if ! command -v openssl >/dev/null 2>&1; then
    echo "openssl not found; install openssl or set TLS_CERT_PATH/TLS_KEY_PATH" >&2
    return 1
  fi

  mkdir -p "$cert_dir"
  cert_conf="$cert_dir/openssl.cnf"
  export TLS_CERT_PATH="$cert_dir/rvoip-${provider}-listener.pem"
  export TLS_KEY_PATH="$cert_dir/rvoip-${provider}-listener-key.pem"

  cert_cn=${TLS_CERT_CN:-rvoip-pbx-example.local}
  advertised_ip=${RVOIP_ADVERTISED_IP:-${ADVERTISED_IP:-}}
  local_ip=${RVOIP_LOCAL_IP:-${LOCAL_IP:-}}
  dns_i=1
  ip_i=1

  {
    printf '%s\n' '[req]'
    printf '%s\n' 'distinguished_name = dn'
    printf '%s\n' 'x509_extensions = v3_req'
    printf '%s\n' 'prompt = no'
    printf '%s\n' '[dn]'
    printf 'CN = %s\n' "$cert_cn"
    printf '%s\n' '[v3_req]'
    printf '%s\n' 'basicConstraints = CA:FALSE'
    printf '%s\n' 'keyUsage = digitalSignature, keyEncipherment'
    printf '%s\n' 'extendedKeyUsage = serverAuth'
    printf '%s\n' 'subjectAltName = @alt_names'
    printf '%s\n' '[alt_names]'
    printf 'DNS.%s = %s\n' "$dns_i" "$cert_cn"
    dns_i=$((dns_i + 1))
    printf 'DNS.%s = localhost\n' "$dns_i"
    dns_i=$((dns_i + 1))
    printf 'IP.%s = 127.0.0.1\n' "$ip_i"
    ip_i=$((ip_i + 1))
    if [ -n "$advertised_ip" ]; then
      if pbx_is_ip_literal "$advertised_ip"; then
        printf 'IP.%s = %s\n' "$ip_i" "$advertised_ip"
        ip_i=$((ip_i + 1))
      else
        printf 'DNS.%s = %s\n' "$dns_i" "$advertised_ip"
        dns_i=$((dns_i + 1))
      fi
    fi
    if [ -n "$local_ip" ] && pbx_is_ip_literal "$local_ip"; then
      printf 'IP.%s = %s\n' "$ip_i" "$local_ip"
    fi
  } >"$cert_conf"

  openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$TLS_KEY_PATH" \
    -out "$TLS_CERT_PATH" \
    -days "${TLS_CERT_DAYS:-1}" \
    -config "$cert_conf" \
    -sha256 >/dev/null 2>&1 || return 1

  echo "Generated self-signed SIP TLS listener cert:"
  echo "  TLS_CERT_PATH=$TLS_CERT_PATH"
  echo "  TLS_KEY_PATH=$TLS_KEY_PATH"
}

ensure_pbx_tls_listener_cert() {
  cert_dir=$1
  if ! pbx_tls_uses_reachable_contact; then
    return 0
  fi
  if [ -n "${TLS_CERT_PATH:-}" ] && [ -n "${TLS_KEY_PATH:-}" ]; then
    if [ -s "$TLS_CERT_PATH" ] && [ -s "$TLS_KEY_PATH" ]; then
      export TLS_CERT_PATH
      export TLS_KEY_PATH
      return 0
    fi
    echo "Configured TLS_CERT_PATH/TLS_KEY_PATH are not readable; generating a self-signed pair."
  fi
  pbx_generate_tls_cert "$cert_dir"
}
