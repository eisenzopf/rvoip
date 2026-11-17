#!/bin/bash
# rvoip bench host agent. run_lane.py uploads this file per lane run and calls
# it over SSM:  bench-agent.sh <command> <s3 prefix> [args]
#
#   install     <run prefix>              fetch binaries/TLS/sampler, verify sha256, print host facts
#   dut-start   <trial prefix> <bin>      fresh DUT (or ea-uas) process, wait for ready.json
#   dut-finish  <trial prefix> <bin>      drain (finish file or SIGTERM), pack and upload dut.tar.gz
#   caller-run  <trial prefix> <index>    disk guard, start barrier, run the caller, upload caller-<i>.tar.gz
#
# Around each DUT and caller run the agent snapshots the network counters (net-before.txt, net-after.txt:
# ethtool -S, sysfs statistics, softnet_stat, /proc/net/snmp + netstat, conntrack count) into the run dir.
#   cleanup                               stop every bench process
set -euo pipefail
export PATH=$PATH:/snap/bin
ROOT=/opt/rvoip-bench
BIN=$ROOT/bin
WORK=/var/lib/rvoip-bench
MIN_FREE_GIB=${MIN_FREE_GIB:-20}
cmd=$1; shift

log() { echo "$(date -u +%FT%TZ) $*" >&2; }
free_gib() { df -BG --output=avail "$WORK" | tail -1 | tr -dc 0-9; }
disk_guard() {
  local free; free=$(free_gib)
  if [ "$free" -lt "$MIN_FREE_GIB" ]; then
    log "disk guard: ${free} GiB free < ${MIN_FREE_GIB} GiB"; echo '{"disk_guard":"refused","free_gib":'"$free"'}'; exit 75
  fi
}
upload_sha() {  # upload_sha <file> <s3 dest dir>
  local f=$1 d=$2 sum
  sum=$(sha256sum "$f" | cut -d' ' -f1)
  echo "$sum  $(basename "$f")" > "$f.sha256"
  aws s3 cp --only-show-errors "$f" "$d$(basename "$f")"
  aws s3 cp --only-show-errors "$f.sha256" "$d$(basename "$f").sha256"
  echo "$sum"
}
stop_all() {
  for name in rvoip-bench-uas ea-uas rvoip-loadgen; do pkill -TERM -x "$name" 2>/dev/null || true; done
  sleep 2
  for name in rvoip-bench-uas ea-uas rvoip-loadgen; do pkill -KILL -x "$name" 2>/dev/null || true; done
  pkill -f host_sampler.py 2>/dev/null || true
}
net_snapshot() {  # net_snapshot <file>: NIC, softnet, SNMP and conntrack counters; summarize_trial.py parses the deltas
  local f=$1 iface s
  iface=$(awk '$2 == "00000000" {print $1; exit}' /proc/net/route 2>/dev/null || true)
  {
    echo "### at $(date +%s.%N)"
    echo "### iface ${iface}"
    if [ -n "$iface" ]; then
      echo "### ethtool -S $iface"; ethtool -S "$iface" 2>&1 || true
      echo "### sysfs /sys/class/net/$iface/statistics"
      for s in /sys/class/net/"$iface"/statistics/*; do echo "$(basename "$s"): $(cat "$s" 2>/dev/null)"; done
    fi
    echo "### softnet /proc/net/softnet_stat"; cat /proc/net/softnet_stat 2>&1 || true
    echo "### snmp /proc/net/snmp"; cat /proc/net/snmp 2>&1 || true
    echo "### snmp /proc/net/netstat"; cat /proc/net/netstat 2>&1 || true
    echo "### conntrack"
    if command -v conntrack >/dev/null 2>&1; then conntrack -C 2>/dev/null || true
    elif [ -r /proc/sys/net/netfilter/nf_conntrack_count ]; then cat /proc/sys/net/netfilter/nf_conntrack_count; fi
  } > "$f" 2>&1 || true
}
facts() {
  python3 - <<'PY'
import json, os, subprocess
def out(*a):
    return subprocess.run(a, capture_output=True, text=True).stdout.strip()
def tok():
    return out('curl', '-s', '-X', 'PUT', 'http://169.254.169.254/latest/api/token', '-H', 'X-aws-ec2-metadata-token-ttl-seconds: 60')
t = tok()
md = lambda p: out('curl', '-s', '-H', 'X-aws-ec2-metadata-token: ' + t, 'http://169.254.169.254/latest/meta-data/' + p)
facts = {
  'instance_id': md('instance-id'), 'instance_type': md('instance-type'), 'az': md('placement/availability-zone'),
  'ami_id': md('ami-id'), 'kernel': os.uname().release, 'nproc': os.cpu_count(),
  'lscpu': out('lscpu', '-J'), 'chrony': out('chronyc', 'tracking'),
  'ips': out('ip', '-j', '-4', 'addr', 'show'),
  'tuning': json.load(open('/var/lib/rvoip-bench/tuning.json')) if os.path.exists('/var/lib/rvoip-bench/tuning.json') else None,
  'ami': json.load(open('/opt/rvoip-bench/ami.json')) if os.path.exists('/opt/rvoip-bench/ami.json') else None,
  'free_gib': int(out('df', '-BG', '--output=avail', '/var/lib/rvoip-bench').splitlines()[-1].strip().rstrip('G')),
}
print(json.dumps(facts))
PY
}

case "$cmd" in
install)
  run=$1
  mkdir -p "$BIN" "$WORK" "$ROOT/tls" "$ROOT/py"
  aws s3 cp --only-show-errors --recursive "${run}bin/" "$BIN/"
  (cd "$BIN" && for s in *.sha256; do sha256sum -c --quiet "$s"; done)
  chmod 0755 "$BIN"/* ; rm -f "$BIN"/*.sha256.sha256
  aws s3 cp --only-show-errors --recursive "${run}tls/" "$ROOT/tls/"
  chmod 0600 "$ROOT"/tls/*.key
  aws s3 cp --only-show-errors --recursive "${run}py/" "$ROOT/py/"
  stop_all
  facts
  ;;
dut-start)
  trial=$1 bin=$2
  stop_all
  rm -rf "$WORK/run"; mkdir -p "$WORK/run"
  aws s3 cp --only-show-errors "${trial}dut.json" "$WORK/run/config.json"
  cd "$WORK/run"
  net_snapshot "$WORK/run/net-before.txt"
  ulimit -n 4194304
  if [ "$bin" = ea-uas ]; then
    setsid nohup "$BIN/ea-uas" "$WORK/run/config.json" > "$WORK/run/dut.log" 2>&1 < /dev/null &
  else
    setsid nohup "$BIN/$bin" --config "$WORK/run/config.json" > "$WORK/run/dut.log" 2>&1 < /dev/null &
  fi
  pid=$!
  echo "$pid" > "$WORK/run/pid"
  setsid nohup python3 "$ROOT/py/host_sampler.py" --pid "$pid" --out "$WORK/run/resources.jsonl" > "$WORK/run/sampler.log" 2>&1 < /dev/null &
  for _ in $(seq 1 120); do
    if [ "$bin" = ea-uas ] && grep -q ea_uas_started "$WORK/run/stats.jsonl" 2>/dev/null; then
      python3 - "$pid" > "$WORK/run/ready.json" <<'PY'
import json, sys, time
print(json.dumps({'pid': int(sys.argv[1]), 'started_at': time.time(), 'version': 'ea-uas', 'git_sha': None,
                  'kind': 'ea-uas', 'started': json.loads(next(l for l in open('/var/lib/rvoip-bench/run/stats.jsonl') if 'ea_uas_started' in l))}))
PY
    fi
    if [ -s "$WORK/run/ready.json" ]; then cat "$WORK/run/ready.json"; exit 0; fi
    if ! kill -0 "$pid" 2>/dev/null; then log "DUT exited before ready"; tail -50 "$WORK/run/dut.log" >&2; exit 70; fi
    sleep 0.5
  done
  log "DUT not ready after 60 s"; tail -50 "$WORK/run/dut.log" >&2; exit 71
  ;;
dut-finish)
  trial=$1 bin=$2
  cd "$WORK/run"
  pid=$(cat pid)
  curl -s -m 10 "http://127.0.0.1:${METRICS_PORT:-9464}/metrics" > metrics-before-drain.prom || true
  if [ "$bin" = ea-uas ]; then kill -TERM "$pid" 2>/dev/null || true; else touch finish; fi
  rc=
  for _ in $(seq 1 600); do
    if ! kill -0 "$pid" 2>/dev/null; then break; fi
    sleep 0.5
  done
  if kill -0 "$pid" 2>/dev/null; then
    log "DUT still running 300 s after finish: SIGKILL"; kill -KILL "$pid" || true; rc=137
  fi
  # setsid'd child: its exit status is recorded by the DUT itself in summary.json;
  # the agent records whether it had to be killed.
  echo "{\"killed\": $([ -n "$rc" ] && echo true || echo false)}" > exit.json
  sleep 2; pkill -f host_sampler.py 2>/dev/null || true
  net_snapshot "$WORK/run/net-after.txt"
  cp /var/lib/rvoip-bench/tuning.json tuning.json 2>/dev/null || true
  facts > host-facts.json || true
  tar -C "$WORK/run" -czf "$WORK/dut.tar.gz" .
  upload_sha "$WORK/dut.tar.gz" "$trial"
  rm -f "$WORK/dut.tar.gz" "$WORK/dut.tar.gz.sha256"
  ;;
caller-run)
  trial=$1 index=$2
  disk_guard
  stop_all
  out="$WORK/caller-$index"
  rm -rf "$out"; mkdir -p "$out"
  aws s3 cp --only-show-errors "${trial}caller-$index.json" "$out/caller.json"
  aws s3 cp --only-show-errors "${trial}caller-env.sh" "$out/caller-env.sh"
  net_snapshot "$out/net-before.txt"   # before the barrier, so it never delays the start
  echo "{\"index\": $index, \"at\": $(date +%s.%N)}" > "$out/barrier-ready.json"
  aws s3 cp --only-show-errors "$out/barrier-ready.json" "${trial}barrier/ready-$index.json"
  go=
  for _ in $(seq 1 600); do
    if aws s3 cp --only-show-errors "${trial}barrier/go.json" "$out/go.json" 2>/dev/null; then go=1; break; fi
    sleep 0.5
  done
  [ -n "$go" ] || { log "no go.json after 300 s"; exit 72; }
  start_at=$(python3 -c "import json;print(json.load(open('$out/go.json'))['start_at'])")
  python3 -c "import time;d=$start_at-time.time();time.sleep(max(0,d))"
  echo "{\"started\": $(date +%s.%N)}" > "$out/start.json"
  cd "$out"
  ulimit -n 4194304
  set +e
  ( . "$out/caller-env.sh"; exec "$BIN/rvoip-loadgen" "$out/caller.json" ) > "$out/caller.log" 2>&1 &
  pid=$!
  python3 "$ROOT/py/host_sampler.py" --pid "$pid" --out "$out/resources.jsonl" > "$out/sampler.log" 2>&1 &
  wait "$pid"; rc=$?
  set -e
  echo "{\"code\": $rc, \"ended\": $(date +%s.%N)}" > "$out/exit.json"
  net_snapshot "$out/net-after.txt"
  sleep 3
  cp /var/lib/rvoip-bench/tuning.json "$out/tuning.json" 2>/dev/null || true
  facts > "$out/host-facts.json" || true
  tar -C "$out" -czf "$WORK/caller-$index.tar.gz" .
  upload_sha "$WORK/caller-$index.tar.gz" "$trial"
  rm -rf "$WORK/caller-$index.tar.gz"* "$out"
  exit "$rc"
  ;;
summarize)
  # summarize <trial prefix> <callers>: merge + summarize on this (caller-class) host, upload merged/summary.json.
  trial=$1 callers=$2
  T="$WORK/summarize"; rm -rf "$T"; mkdir -p "$T/dut"
  aws s3 cp --only-show-errors "${trial}plan.json" "$T/plan.json"
  names="dut"; for i in $(seq 1 "$callers"); do names="$names caller-$i"; done
  : > "$T/evidence-manifest.txt"
  for n in $names; do
    aws s3 cp --only-show-errors "${trial}$n.tar.gz" "$T/$n.tar.gz"
    aws s3 cp --only-show-errors "${trial}$n.tar.gz.sha256" "$T/$n.tar.gz.sha256"
    (cd "$T" && sha256sum -c --quiet "$n.tar.gz.sha256")
    cat "$T/$n.tar.gz.sha256" >> "$T/evidence-manifest.txt"
    mkdir -p "$T/$n"; tar -C "$T/$n" -xzf "$T/$n.tar.gz"; rm -f "$T/$n.tar.gz"
  done
  python3 -c "import json,sys;print(json.dumps({l.split()[1]: l.split()[0] for l in open(sys.argv[1])}, indent=1, sort_keys=True))" \
    "$T/evidence-manifest.txt" > "$T/evidence-manifest.json"
  python3 "$ROOT/py/summarize_trial.py" "$T" --criterion rfc > "$T/summarize.log" 2>&1 || { tail -40 "$T/summarize.log" >&2; exit 73; }
  aws s3 cp --only-show-errors "$T/merged/summary.json" "${trial}merged/summary.json"
  aws s3 cp --only-show-errors "$T/evidence-manifest.json" "${trial}evidence-manifest.json"
  rm -rf "$T"
  ;;
host-tweak)
  # host-tweak none|notrack : diagnostic-only caller tweak (never in a gate run). notrack exempts every
  # UDP packet from conntrack, to measure conntrack's share of the receive-path cost.
  mode=$1
  if command -v iptables >/dev/null; then
    iptables -t raw -D PREROUTING -p udp -j NOTRACK 2>/dev/null || true
    iptables -t raw -D OUTPUT -p udp -j NOTRACK 2>/dev/null || true
    if [ "$mode" = notrack ]; then
      iptables -t raw -I PREROUTING -p udp -j NOTRACK
      iptables -t raw -I OUTPUT -p udp -j NOTRACK
    fi
  elif [ "$mode" = notrack ]; then
    log "no iptables on this host"; exit 74
  fi
  python3 -c "import json,os;p='/proc/sys/net/netfilter/nf_conntrack_count';print(json.dumps({'host_tweak':'$mode','conntrack_count':int(open(p).read()) if os.path.exists(p) else None}))"
  ;;
cleanup)
  stop_all
  ;;
*)
  echo "unknown command $cmd" >&2; exit 64 ;;
esac
