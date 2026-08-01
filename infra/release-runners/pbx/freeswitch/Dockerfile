# syntax=docker/dockerfile:1

ARG DEBIAN_VERSION=12-slim

FROM debian:${DEBIAN_VERSION} AS build

ARG TARGETARCH
ARG FREESWITCH_VERSION=v1.10.12
ARG LIBKS_VERSION=v2.0.10
ARG SIGNALWIRE_C_VERSION=v2.0.5
ARG SOFIA_SIP_VERSION=v1.13.17
ARG SPANDSP_VERSION=master
ARG MAKE_JOBS=2

ENV DEBIAN_FRONTEND=noninteractive
ENV PKG_CONFIG_PATH=/usr/local/lib/pkgconfig:/usr/local/lib/aarch64-linux-gnu/pkgconfig:/usr/local/lib/x86_64-linux-gnu/pkgconfig
ENV LD_LIBRARY_PATH=/usr/local/lib:/usr/local/lib/aarch64-linux-gnu:/usr/local/lib/x86_64-linux-gnu

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        autoconf \
        automake \
        ca-certificates \
        cmake \
        curl \
        g++ \
        gcc \
        git \
        libedit-dev \
        libjpeg-dev \
        libldns-dev \
        liblua5.2-dev \
        libopus-dev \
        libpcre3-dev \
        libperl-dev \
        libsndfile1-dev \
        libspeex-dev \
        libspeexdsp-dev \
        libsqlite3-dev \
        libssl-dev \
        libtiff-dev \
        libtool \
        libtool-bin \
        libcurl4-openssl-dev \
        make \
        nasm \
        pkg-config \
        uuid-dev \
        yasm \
        zlib1g-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src

RUN git clone --branch "${LIBKS_VERSION}" https://github.com/signalwire/libks.git \
    && cd libks \
    && cmake -B build -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr/local -DWITH_PACKAGING=OFF \
    && cmake --build build --parallel "${MAKE_JOBS}" \
    && cmake --install build \
    && cd /usr/src \
    && rm -rf libks

RUN git clone --branch "${SIGNALWIRE_C_VERSION}" https://github.com/signalwire/signalwire-c.git \
    && cd signalwire-c \
    && cmake -B build -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/usr/local -DWITH_PACKAGING=OFF \
    && cmake --build build --parallel "${MAKE_JOBS}" \
    && : > build/changelog.Debian.gz \
    && : > build/copyright \
    && cmake --install build \
    && cd /usr/src \
    && rm -rf signalwire-c

RUN git clone --depth 1 --branch "${SOFIA_SIP_VERSION}" https://github.com/freeswitch/sofia-sip.git \
    && cd sofia-sip \
    && ./bootstrap.sh \
    && ./configure --prefix=/usr/local \
    && make -j"${MAKE_JOBS}" \
    && make install \
    && cd /usr/src \
    && rm -rf sofia-sip

RUN git clone --depth 1 --branch "${SPANDSP_VERSION}" https://github.com/freeswitch/spandsp.git \
    && cd spandsp \
    && ./bootstrap.sh \
    && ./configure --prefix=/usr/local \
    && make -j"${MAKE_JOBS}" \
    && make install \
    && cd /usr/src \
    && rm -rf spandsp

RUN git clone --depth 1 --branch "${FREESWITCH_VERSION}" https://github.com/signalwire/freeswitch.git

WORKDIR /usr/src/freeswitch

COPY freeswitch-modules.conf /tmp/freeswitch-modules.conf

RUN ./bootstrap.sh -j \
    && cp /tmp/freeswitch-modules.conf modules.conf

RUN ./configure --prefix=/usr/local/freeswitch \
    && make -j"${MAKE_JOBS}" \
    && make -j"${MAKE_JOBS}" modules \
    && make install \
    && make install_mod \
    && make samples \
    && rm -rf /usr/src/freeswitch

FROM debian:${DEBIAN_VERSION} AS runtime

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libcurl4 \
        libedit2 \
        libjpeg62-turbo \
        libldns3 \
        liblua5.2-0 \
        libopus0 \
        libpcre3 \
        libsndfile1 \
        libspeex1 \
        libspeexdsp1 \
        libsqlite3-0 \
        libssl3 \
        libtiff6 \
        libuuid1 \
        procps \
        tini \
        zlib1g \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /usr/local /usr/local
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh

RUN chmod +x /usr/local/bin/docker-entrypoint.sh \
    && groupadd --system freeswitch \
    && useradd --system --gid freeswitch --home /usr/local/freeswitch freeswitch \
    && chown -R freeswitch:freeswitch /usr/local/freeswitch

ENV PATH=/usr/local/freeswitch/bin:$PATH
ENV LD_LIBRARY_PATH=/usr/local/lib:/usr/local/lib/aarch64-linux-gnu:/usr/local/lib/x86_64-linux-gnu
ENV FS_DEFAULT_PASSWORD=<redacted>
ENV FS_EVENT_SOCKET_PASSWORD=<redacted>
ENV FS_RTP_START=16384
ENV FS_RTP_END=16484
ENV FS_RVOIP_UDP_SIP_PORT=5062
ENV FS_RVOIP_TLS_SIP_PORT=5063
ENV FS_EXTERNAL_SIP_IP=auto
ENV FS_EXTERNAL_RTP_IP=auto

EXPOSE 5060/tcp 5060/udp 5062/tcp 5062/udp 5063/tcp 16384-16484/udp

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/docker-entrypoint.sh"]
CMD ["freeswitch", "-nf", "-nonat"]
