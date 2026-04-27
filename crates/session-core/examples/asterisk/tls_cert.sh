#!/usr/bin/env sh
# Helpers for Asterisk examples that need a local SIP TLS listener.
#
# Reachable-contact mode requires RVOIP to present a listener certificate
# when Asterisk opens TLS connections to the registered Contact. If the user
# has not configured TLS_CERT_PATH/TLS_KEY_PATH, generate a short-lived
# self-signed certificate for the example run.

asterisk_tls_contact_mode() {
  mode=$(printf '%s' "${ASTERISK_TLS_CONTACT_MODE:-reachable-contact}" | tr '[:upper:]' '[:lower:]')
  flow_reuse=$(printf '%s' "${ASTERISK_TLS_FLOW_REUSE:-0}" | tr '[:upper:]' '[:lower:]')
  case "$flow_reuse" in
    1|true|yes|on) mode=registered-flow ;;
  esac
  printf '%s\n' "$mode"
}

asterisk_tls_uses_reachable_contact() {
  case "$(asterisk_tls_contact_mode)" in
    reachable-contact|reachable|listener|uas) return 0 ;;
    *) return 1 ;;
  esac
}

asterisk_is_ip_literal() {
  value=$1
  case "$value" in
    ""|0.0.0.0|::) return 1 ;;
    *:*) return 0 ;;
    *[!0-9.]*) return 1 ;;
    *.*) return 0 ;;
    *) return 1 ;;
  esac
}

asterisk_generate_tls_cert() {
  cert_dir=$1

  if ! command -v openssl >/dev/null 2>&1; then
    echo "openssl not found; install openssl or set TLS_CERT_PATH/TLS_KEY_PATH in examples/asterisk/.env" >&2
    return 1
  fi

  mkdir -p "$cert_dir"
  cert_conf="$cert_dir/openssl.cnf"
  export TLS_CERT_PATH="$cert_dir/rvoip-asterisk-listener.pem"
  export TLS_KEY_PATH="$cert_dir/rvoip-asterisk-listener-key.pem"

  cert_cn=${TLS_CERT_CN:-rvoip-asterisk-example.local}
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

    if [ -n "${ADVERTISED_IP:-}" ]; then
      if asterisk_is_ip_literal "$ADVERTISED_IP"; then
        printf 'IP.%s = %s\n' "$ip_i" "$ADVERTISED_IP"
        ip_i=$((ip_i + 1))
      else
        printf 'DNS.%s = %s\n' "$dns_i" "$ADVERTISED_IP"
        dns_i=$((dns_i + 1))
      fi
    fi

    if [ -n "${LOCAL_IP:-}" ] && asterisk_is_ip_literal "$LOCAL_IP"; then
      printf 'IP.%s = %s\n' "$ip_i" "$LOCAL_IP"
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

ensure_asterisk_tls_listener_cert() {
  cert_dir=$1

  if ! asterisk_tls_uses_reachable_contact; then
    return 0
  fi

  if [ -n "${TLS_CERT_PATH:-}" ] && [ -n "${TLS_KEY_PATH:-}" ]; then
    if [ -s "$TLS_CERT_PATH" ] && [ -s "$TLS_KEY_PATH" ]; then
      export TLS_CERT_PATH
      export TLS_KEY_PATH
      return 0
    fi
    echo "Configured TLS_CERT_PATH/TLS_KEY_PATH are not readable; generating a self-signed pair for this run."
  elif [ -n "${TLS_CERT_PATH:-}" ] || [ -n "${TLS_KEY_PATH:-}" ]; then
    echo "TLS listener cert/key config is incomplete or unreadable; generating a matching self-signed pair for this run."
  fi

  asterisk_generate_tls_cert "$cert_dir"
}
