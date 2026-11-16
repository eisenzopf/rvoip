#!/bin/bash
# Runs on the AMI builder instance (Ubuntu 24.04) over SSM; see bake.py.
# Expects bench_tune.py and rvoip-bench-tune.service in the current directory.
set -euxo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -q
apt-get -y -q -o Dpkg::Options::=--force-confold upgrade
apt-get -y -q install ethtool jq zstd sysstat numactl chrony python3 \
  linux-tools-common "linux-tools-$(uname -r)" linux-tools-aws iproute2 tcpdump
snap list aws-cli >/dev/null 2>&1 || snap install aws-cli --classic
# Sub-millisecond clocks across the lane: the start barrier and the RFC 6076
# cross-host timings rely on them. Amazon Time Sync first.
if ! grep -q 169.254.169.123 /etc/chrony/chrony.conf; then
  sed -i '1i server 169.254.169.123 prefer iburst minpoll 4 maxpoll 4' /etc/chrony/chrony.conf
fi
systemctl enable chrony
install -d -m 0755 /opt/rvoip-bench /var/lib/rvoip-bench
install -m 0755 bench_tune.py /opt/rvoip-bench/bench_tune.py
install -m 0644 rvoip-bench-tune.service /etc/systemd/system/rvoip-bench-tune.service
systemctl daemon-reload
systemctl enable rvoip-bench-tune.service
systemctl disable irqbalance || true
cat > /etc/security/limits.d/90-rvoip-bench.conf <<'LIMITS'
*    soft nofile 4194304
*    hard nofile 4194304
root soft nofile 4194304
root hard nofile 4194304
LIMITS
mkdir -p /etc/systemd/system.conf.d
printf '[Manager]\nDefaultLimitNOFILE=4194304\n' > /etc/systemd/system.conf.d/90-rvoip-bench.conf
# The AMI records what it is.
python3 - <<'PY'
import json, os, subprocess, time
def out(*a):
    return subprocess.run(a, capture_output=True, text=True).stdout.strip()
json.dump({'baked_at': time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime()), 'kernel': os.uname().release,
           'os': out('lsb_release', '-ds'), 'tune_sha256': out('sha256sum', '/opt/rvoip-bench/bench_tune.py').split()[0],
           'aws_cli': out('/snap/bin/aws', '--version')},
          open('/opt/rvoip-bench/ami.json', 'w'), indent=1)
PY
cat /opt/rvoip-bench/ami.json
apt-get -y -q autoremove
apt-get clean
cloud-init clean --logs || true
