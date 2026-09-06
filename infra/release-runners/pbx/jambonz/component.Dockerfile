FROM docker.io/library/node@sha256:2cf067cfed83d5ea958367df9f966191a942351a2df77d6f0193e162b5febfc0

ARG SOURCE_URL
ARG SOURCE_SHA256
ARG SOURCE_VERSION

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl build-essential python3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /opt/jambonz
RUN curl --fail --silent --show-error --location "$SOURCE_URL" --output /tmp/source.tar.gz \
    && echo "$SOURCE_SHA256  /tmp/source.tar.gz" | sha256sum --check --strict \
    && tar --extract --gzip --file /tmp/source.tar.gz --strip-components=1 \
    && test "$(node -p 'require("./package.json").version')" = "$SOURCE_VERSION" \
    && npm ci --omit=dev \
    && rm /tmp/source.tar.gz \
    && apt-get purge -y --auto-remove build-essential python3 curl

USER node
CMD ["node", "app.js"]
