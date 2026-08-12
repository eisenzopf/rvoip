# syntax=docker/dockerfile:1
#
# Release-runner Asterisk with AMR / AMR-WB support.
#
# Built from source because AMR support in Asterisk is a source patch, not a
# loadable module: the packaged Alpine asterisk this image previously used has
# no way to add it, which is why the release gates could only SKIP the AMR
# matrix rows. The patch set is traud/asterisk-amr, fetched at build time
# pinned to an immutable commit and checksum — third-party (GPL) sources are
# build inputs of this container image and never enter the rvoip repository,
# the same posture as the 3GPP reference material and the run-only GPL
# containers.
#
# The AMR codec libraries are Apache-2.0: opencore-amr provides AMR-NB
# encode/decode and AMR-WB decode; VisualOn's vo-amrwbenc provides AMR-WB
# encode.

ARG DEBIAN_VERSION=12-slim

# Immutable inputs. asterisk-20.20.1 rather than the mutable -current tarball;
# the traud tarball is addressed by commit (unchanged upstream since 2022).
ARG ASTERISK_VERSION=20.20.1
ARG ASTERISK_SHA256=32ef7db8e449629128eb5f7e42d3d704f03daacb91d76ee694a9acf7f1fd53e2
ARG AMR_PATCH_COMMIT=420ab33f236e15955351e45bf9fbb256228afe21
ARG AMR_PATCH_SHA256=ac86a10451c37572b42b24cc0531be40362f742286336f99904e62f48f180baa

FROM debian:${DEBIAN_VERSION} AS build

ARG ASTERISK_VERSION
ARG ASTERISK_SHA256
ARG AMR_PATCH_COMMIT
ARG AMR_PATCH_SHA256
ARG MAKE_JOBS=4
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        autoconf \
        automake \
        build-essential \
        ca-certificates \
        curl \
        libedit-dev \
        libjansson-dev \
        libsqlite3-dev \
        libssl-dev \
        libsrtp2-dev \
        libxml2-dev \
        libxslt1-dev \
        patch \
        pkg-config \
        uuid-dev \
        xmlstarlet \
        libopencore-amrnb-dev \
        libopencore-amrwb-dev \
        libvo-amrwbenc-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/src

RUN set -eux; \
    curl -fsSL -o asterisk.tar.gz \
        "https://downloads.asterisk.org/pub/telephony/asterisk/releases/asterisk-${ASTERISK_VERSION}.tar.gz" \
        || curl -fsSL -o asterisk.tar.gz \
        "https://downloads.asterisk.org/pub/telephony/asterisk/asterisk-${ASTERISK_VERSION}.tar.gz"; \
    printf '%s  %s\n' "${ASTERISK_SHA256}" asterisk.tar.gz | sha256sum --check --strict; \
    mkdir -p asterisk; \
    tar xzf asterisk.tar.gz -C asterisk --strip-components=1; \
    rm asterisk.tar.gz; \
    curl -fsSL -o amr-patch.tar.gz \
        "https://codeload.github.com/traud/asterisk-amr/tar.gz/${AMR_PATCH_COMMIT}"; \
    printf '%s  %s\n' "${AMR_PATCH_SHA256}" amr-patch.tar.gz | sha256sum --check --strict; \
    mkdir -p asterisk-amr; \
    tar xzf amr-patch.tar.gz -C asterisk-amr --strip-components=1; \
    rm amr-patch.tar.gz

WORKDIR /usr/src/asterisk

# New source files first (pure additions), then the two patches: codec_amr
# registers the formats in the core (format cache, codec table, RTP mime
# types) and build_tools teaches ./configure to find the three AMR libraries.
# --fuzz=5 absorbs line-number drift from the patch's Asterisk 13 baseline;
# context still has to match, so an incompatible tree fails here rather than
# silently producing an Asterisk without AMR.
RUN set -eux; \
    cp ../asterisk-amr/codecs/codec_amr.c codecs/codec_amr.c; \
    cp ../asterisk-amr/codecs/ex_amr.h codecs/ex_amr.h; \
    cp ../asterisk-amr/res/res_format_attr_amr.c res/res_format_attr_amr.c; \
    cp ../asterisk-amr/include/asterisk/amr.h include/asterisk/amr.h; \
    patch -p0 --fuzz=5 < ../asterisk-amr/codec_amr.patch; \
    patch -p0 --fuzz=5 < ../asterisk-amr/build_tools.patch

RUN ./bootstrap.sh

RUN ./configure --with-jansson-bundled --with-pjproject-bundled

# Fail early and clearly if configure did not find the AMR libraries: without
# them codec_amr is silently skipped and the image looks fine until a call
# fails to negotiate.
RUN set -eux; \
    grep -q '^AMR_NB_LIB=.\+' makeopts || { echo "ERROR: opencore-amrnb not detected by configure"; exit 1; }; \
    grep -q '^AMR_WB_DECODER_LIB=.\+' makeopts || { echo "ERROR: opencore-amrwb not detected by configure"; exit 1; }; \
    grep -q '^AMR_WB_ENCODER_LIB=.\+' makeopts || { echo "ERROR: vo-amrwbenc not detected by configure"; exit 1; }

RUN make menuselect.makeopts \
    && ./menuselect/menuselect --enable codec_amr --enable res_format_attr_amr menuselect.makeopts

# `third-party` (bundled pjproject and jansson) is built serially on purpose:
# its makefiles race under -j and fail in libpjnath with no diagnostic beyond
# "Error 2". Everything after it parallelises fine.
RUN make -j1 third-party \
    && make -j"${MAKE_JOBS}" \
    && make install DESTDIR=/build-root \
    && make samples DESTDIR=/build-root

# Both modules must exist, or the image is not what it claims to be.
RUN set -eux; \
    test -f /build-root/usr/lib/asterisk/modules/codec_amr.so; \
    test -f /build-root/usr/lib/asterisk/modules/res_format_attr_amr.so

FROM debian:${DEBIAN_VERSION} AS runtime

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        libedit2 \
        libsqlite3-0 \
        libssl3 \
        libsrtp2-1 \
        libuuid1 \
        libxml2 \
        libxslt1.1 \
        procps \
        libopencore-amrnb0 \
        libopencore-amrwb0 \
        libvo-amrwbenc0 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /build-root/ /

RUN mkdir -p /var/log/asterisk /var/spool/asterisk /var/run/asterisk

EXPOSE 5060/udp 5061/tcp 10000-10100/udp

CMD ["asterisk", "-f", "-vvv"]
