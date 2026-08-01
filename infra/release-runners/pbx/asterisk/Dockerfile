FROM alpine:3.20

RUN apk add --no-cache \
        asterisk \
        asterisk-srtp \
        ca-certificates \
        bind-tools \
        iproute2 \
        netcat-openbsd \
        openssl \
        procps

EXPOSE 5060/udp 5061/tcp 10000-10100/udp

CMD ["asterisk", "-f", "-vvv"]
