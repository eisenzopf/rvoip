#!/bin/sh
set -eu

image="${RVOIP_REDIS_CLUSTER_IMAGE:-redis:7.2-alpine}"
base_port="${RVOIP_REDIS_CLUSTER_BASE_PORT:-17400}"
container="rvoip-redis-cluster-$$"
port_one="$base_port"
port_two="$((base_port + 1))"
port_three="$((base_port + 2))"

cleanup() {
    docker rm -f "$container" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

docker run --detach \
    --name "$container" \
    --publish "$port_one:$port_one" \
    --publish "$port_two:$port_two" \
    --publish "$port_three:$port_three" \
    "$image" \
    sh -ec "
        for port in $port_one $port_two $port_three; do
            redis-server \
                --port \"\$port\" \
                --bind 0.0.0.0 \
                --protected-mode no \
                --appendonly no \
                --save '' \
                --cluster-enabled yes \
                --cluster-config-file \"nodes-\$port.conf\" \
                --cluster-node-timeout 5000 \
                --cluster-announce-ip 127.0.0.1 \
                --cluster-announce-port \"\$port\" \
                --cluster-announce-bus-port \"\$((port + 10000))\" \
                --daemonize yes
        done
        for port in $port_one $port_two $port_three; do
            until redis-cli -p \"\$port\" ping >/dev/null 2>&1; do
                sleep 1
            done
        done
        redis-cli --cluster create \
            127.0.0.1:$port_one \
            127.0.0.1:$port_two \
            127.0.0.1:$port_three \
            --cluster-replicas 0 \
            --cluster-yes
        tail -f /dev/null
    " >/dev/null

attempt=0
until docker exec "$container" redis-cli -p "$port_one" cluster info \
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

RVOIP_REDIS_CLUSTER_URLS="redis://127.0.0.1:$port_one,redis://127.0.0.1:$port_two,redis://127.0.0.1:$port_three" \
    cargo test -p rvoip-redis --test redis_cluster_live -- --nocapture
