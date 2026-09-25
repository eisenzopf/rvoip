ARG SOURCE_BUILDER_IMAGE=docker.io/library/node@sha256:2cf067cfed83d5ea958367df9f966191a942351a2df77d6f0193e162b5febfc0
ARG MYSQL_IMAGE=docker.io/library/mysql@sha256:4bc6bc963e6d8443453676cae56536f4b8156d78bae03c0145cbe47c2aad73bb
FROM ${SOURCE_BUILDER_IMAGE} AS source

ARG SOURCE_URL
ARG SOURCE_SHA256
ARG SOURCE_VERSION

WORKDIR /source
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/* \
    && curl --fail --silent --show-error --location "$SOURCE_URL" --output /tmp/source.tar.gz \
    && echo "$SOURCE_SHA256  /tmp/source.tar.gz" | sha256sum --check --strict \
    && tar --extract --gzip --file /tmp/source.tar.gz --strip-components=1 \
    && test "$(node -p 'require("./package.json").version')" = "$SOURCE_VERSION"

FROM ${MYSQL_IMAGE} AS mysql
COPY --from=source /source/test/db/jambones-sql.sql /docker-entrypoint-initdb.d/01-schema.sql
COPY --from=source /source/test/db/populate-test-data.sql /docker-entrypoint-initdb.d/02-populate.sql
COPY rvoip.sql /docker-entrypoint-initdb.d/03-rvoip.sql
