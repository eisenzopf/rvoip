#!/bin/sh
set -eu

image="${RVOIP_REDIS_CLUSTER_IMAGE:-redis:7.2-alpine}"
base_port="${RVOIP_REDIS_CLUSTER_BASE_PORT:-17400}"
password="${RVOIP_REDIS_CLUSTER_PASSWORD:-rvoip-cluster-test-password}"
repository_root="$(CDPATH= cd -- "$(dirname "$0")/../../../.." && pwd)"
container="rvoip-redis-cluster-$$"
port_one="$base_port"
port_two="$((base_port + 1))"
port_three="$((base_port + 2))"
single_port="$((base_port + 3))"

case "$password" in
    *[!A-Za-z0-9._~-]*)
        echo "RVOIP_REDIS_CLUSTER_PASSWORD contains URL-unsafe characters" >&2
        exit 1
        ;;
esac

mkdir -p "$repository_root/target"
certificate_dir="$(mktemp -d "$repository_root/target/rvoip-redis-tls.XXXXXX")"

cleanup() {
    docker rm -f "$container" >/dev/null 2>&1 || true
    rm -rf "$certificate_dir"
}
trap cleanup EXIT INT TERM

openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$certificate_dir/ca.key" \
    -out "$certificate_dir/ca.crt" \
    -days 1 \
    -subj '/CN=rvoip-redis-test-ca' \
    -addext 'basicConstraints=critical,CA:TRUE' \
    -addext 'keyUsage=critical,keyCertSign,cRLSign' \
    >/dev/null 2>&1
openssl req -new -newkey rsa:2048 -nodes \
    -keyout "$certificate_dir/server.key" \
    -out "$certificate_dir/server.csr" \
    -subj '/CN=localhost' \
    -addext 'subjectAltName=DNS:localhost,IP:127.0.0.1' \
    -addext 'extendedKeyUsage=serverAuth,clientAuth' \
    >/dev/null 2>&1
openssl x509 -req \
    -in "$certificate_dir/server.csr" \
    -CA "$certificate_dir/ca.crt" \
    -CAkey "$certificate_dir/ca.key" \
    -CAcreateserial \
    -out "$certificate_dir/server.crt" \
    -days 1 \
    -copy_extensions copy \
    >/dev/null 2>&1
openssl req -new -newkey rsa:2048 -nodes \
    -keyout "$certificate_dir/client.key" \
    -out "$certificate_dir/client.csr" \
    -subj '/CN=rvoip-redis-test-client' \
    -addext 'extendedKeyUsage=clientAuth' \
    >/dev/null 2>&1
openssl x509 -req \
    -in "$certificate_dir/client.csr" \
    -CA "$certificate_dir/ca.crt" \
    -CAkey "$certificate_dir/ca.key" \
    -CAcreateserial \
    -out "$certificate_dir/client.crt" \
    -days 1 \
    -copy_extensions copy \
    >/dev/null 2>&1
openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$certificate_dir/untrusted-ca.key" \
    -out "$certificate_dir/untrusted-ca.crt" \
    -days 1 \
    -subj '/CN=rvoip-redis-untrusted-test-ca' \
    -addext 'basicConstraints=critical,CA:TRUE' \
    -addext 'keyUsage=critical,keyCertSign,cRLSign' \
    >/dev/null 2>&1
chmod 755 "$certificate_dir"
chmod 644 "$certificate_dir"/*

docker run --detach \
    --name "$container" \
    --publish "$port_one:$port_one" \
    --publish "$port_two:$port_two" \
    --publish "$port_three:$port_three" \
    --publish "$single_port:$single_port" \
    --volume "$certificate_dir:/tls:ro" \
    "$image" \
    sh -ec "
        for port in $port_one $port_two $port_three; do
            redis-server \
                --port 0 \
                --tls-port \"\$port\" \
                --tls-cert-file /tls/server.crt \
                --tls-key-file /tls/server.key \
                --tls-ca-cert-file /tls/ca.crt \
                --tls-auth-clients yes \
                --tls-cluster yes \
                --bind 0.0.0.0 \
                --protected-mode no \
                --requirepass "$password" \
                --masterauth "$password" \
                --appendonly no \
                --save '' \
                --cluster-enabled yes \
                --cluster-config-file \"nodes-\$port.conf\" \
                --cluster-node-timeout 5000 \
                --cluster-announce-ip 127.0.0.1 \
                --cluster-announce-port 0 \
                --cluster-announce-tls-port \"\$port\" \
                --cluster-announce-bus-port \"\$((port + 10000))\" \
                --daemonize yes
        done
        redis-server \
            --port 0 \
            --tls-port $single_port \
            --tls-cert-file /tls/server.crt \
            --tls-key-file /tls/server.key \
            --tls-ca-cert-file /tls/ca.crt \
            --tls-auth-clients yes \
            --bind 0.0.0.0 \
            --protected-mode no \
            --requirepass "$password" \
            --appendonly no \
            --save '' \
            --daemonize yes
        for port in $port_one $port_two $port_three; do
            until REDISCLI_AUTH="$password" redis-cli --tls \
                --cacert /tls/ca.crt \
                --cert /tls/client.crt \
                --key /tls/client.key \
                -p \"\$port\" ping >/dev/null 2>&1; do
                sleep 1
            done
        done
        REDISCLI_AUTH="$password" redis-cli --tls \
            --cacert /tls/ca.crt \
            --cert /tls/client.crt \
            --key /tls/client.key \
            --cluster create \
            127.0.0.1:$port_one \
            127.0.0.1:$port_two \
            127.0.0.1:$port_three \
            --cluster-replicas 0 \
            --cluster-yes
        tail -f /dev/null
    " >/dev/null

attempt=0
until docker exec -e REDISCLI_AUTH="$password" "$container" \
    redis-cli --tls \
        --cacert /tls/ca.crt \
        --cert /tls/client.crt \
        --key /tls/client.key \
        -p "$port_one" cluster info \
    | grep -q 'cluster_state:ok'; do
    if ! docker inspect --format '{{.State.Running}}' "$container" 2>/dev/null \
        | grep -q true; then
        docker logs "$container"
        exit 1
    fi
    attempt=$((attempt + 1))
    if [ "$attempt" -ge 60 ]; then
        docker logs "$container"
        exit 1
    fi
    sleep 1
done

RVOIP_REDIS_CLUSTER_URLS="rediss://:$password@127.0.0.1:$port_one,rediss://:$password@127.0.0.1:$port_two,rediss://:$password@127.0.0.1:$port_three" \
RVOIP_REDIS_CLUSTER_TLS_URLS="rediss://:$password@127.0.0.1:$port_one,rediss://:$password@127.0.0.1:$port_two,rediss://:$password@127.0.0.1:$port_three" \
RVOIP_REDIS_SINGLE_TLS_URL="rediss://:$password@127.0.0.1:$single_port" \
RVOIP_REDIS_TLS_CA_CERT="$certificate_dir/ca.crt" \
RVOIP_REDIS_TLS_CLIENT_CERT="$certificate_dir/client.crt" \
RVOIP_REDIS_TLS_CLIENT_KEY="$certificate_dir/client.key" \
RVOIP_REDIS_TLS_UNTRUSTED_CA_CERT="$certificate_dir/untrusted-ca.crt" \
RVOIP_REDIS_CLUSTER_DOCKER_CONTAINER="$container" \
RVOIP_REDIS_CLUSTER_PASSWORD="$password" \
RVOIP_REDIS_CLUSTER_PORTS="$port_one,$port_two,$port_three" \
RVOIP_REDIS_CLUSTER_DOCKER_TLS=true \
    cargo test -p rvoip-redis --test redis_cluster_live -- --nocapture --test-threads=1
