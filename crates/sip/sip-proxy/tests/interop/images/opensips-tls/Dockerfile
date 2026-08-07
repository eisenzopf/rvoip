FROM opensips/opensips:3.6@sha256:eba1396b438a7f8a9d33c17017aae4670cb43361eb7130359240cf85fc3e6979

ARG OPENSIPS_VERSION=3.6.7-1
ARG OPENSIPS_POOL=https://apt.opensips.org/pool/bullseye/3.6-releases/o/opensips
ARG PROTO_TLS_SHA256=685f704faf2ce6b9015a0c059a32333bf789cabd8a467c7068aa1cea363de799
ARG TLS_MGM_SHA256=20d83193399a9ee02b8c3f1cc2fbe311231ec22c0b43656add63f77110a68545
ARG TLS_OPENSSL_SHA256=690c52e06b9d0f8d76483900a4185f3a3d7cc3ec30a752ff762bdca254750209

LABEL org.opencontainers.image.title="rvoip OpenSIPS 3.6.7 TLS interop peer" \
      org.opencontainers.image.description="Pinned OpenSIPS beta-gate peer with only the reviewed TLS modules added" \
      org.opencontainers.image.version="3.6.7-1" \
      org.opencontainers.image.base.digest="sha256:eba1396b438a7f8a9d33c17017aae4670cb43361eb7130359240cf85fc3e6979"

RUN set -eux; \
    proto_tls="opensips-tls-module_${OPENSIPS_VERSION}_amd64.deb"; \
    tls_mgm="opensips-tlsmgm-module_${OPENSIPS_VERSION}_amd64.deb"; \
    tls_openssl="opensips-tls-openssl-module_${OPENSIPS_VERSION}_amd64.deb"; \
    curl --fail --location --proto '=https' --tlsv1.2 \
        --output "/tmp/${proto_tls}" "${OPENSIPS_POOL}/${proto_tls}"; \
    curl --fail --location --proto '=https' --tlsv1.2 \
        --output "/tmp/${tls_mgm}" "${OPENSIPS_POOL}/${tls_mgm}"; \
    curl --fail --location --proto '=https' --tlsv1.2 \
        --output "/tmp/${tls_openssl}" "${OPENSIPS_POOL}/${tls_openssl}"; \
    printf '%s  %s\n' "${PROTO_TLS_SHA256}" "/tmp/${proto_tls}" \
        | sha256sum --check --strict; \
    printf '%s  %s\n' "${TLS_MGM_SHA256}" "/tmp/${tls_mgm}" \
        | sha256sum --check --strict; \
    printf '%s  %s\n' "${TLS_OPENSSL_SHA256}" "/tmp/${tls_openssl}" \
        | sha256sum --check --strict; \
    dpkg --install "/tmp/${tls_mgm}" "/tmp/${tls_openssl}" "/tmp/${proto_tls}"; \
    test "$(dpkg-query -W -f='${Version}' opensips)" = "${OPENSIPS_VERSION}"; \
    test "$(dpkg-query -W -f='${Version}' opensips-tls-module)" = "${OPENSIPS_VERSION}"; \
    test "$(dpkg-query -W -f='${Version}' opensips-tlsmgm-module)" = "${OPENSIPS_VERSION}"; \
    test "$(dpkg-query -W -f='${Version}' opensips-tls-openssl-module)" = "${OPENSIPS_VERSION}"; \
    test -f /usr/lib/x86_64-linux-gnu/opensips/modules/proto_tls.so; \
    test -f /usr/lib/x86_64-linux-gnu/opensips/modules/tls_mgm.so; \
    test -f /usr/lib/x86_64-linux-gnu/opensips/modules/tls_openssl.so; \
    rm -f "/tmp/${proto_tls}" "/tmp/${tls_mgm}" "/tmp/${tls_openssl}"
