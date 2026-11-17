#!/usr/bin/env python3
"""Host tuning for rvoip bench lane hosts, applied at every boot.

Ported from the E74 harness (e48_nic.py and mem.py): the receiver NIC
placement, 500 us coalescing, conntrack sizing, port reservations and THP.
Installed into the bench AMI as /opt/rvoip-bench/bench_tune.py and run by
rvoip-bench-tune.service. The role (dut | caller) comes from the instance tag
`rvoip-role` (instance metadata tags are enabled by the launch templates) or
from --role.

  XPS       CPU i of the online list -> TX queue i mod Q
  IRQ       TX/RX queue q -> the second SMT sibling of the first CPU that sends on it
  coalesce  adaptive RX off, rx-usecs 500, tx-usecs 500
  ring      RX ring 8,192 (ENA maximum)
  conntrack nf_conntrack_max 4,194,304, hashsize 1,048,576 (only if loaded)
  ports     DUT: RTP 10000-64999 reserved, ephemeral 65000-65535
            caller: media 25000-64999 reserved, ephemeral 10240-24999
  memory    THP always, defrag madvise
  limits    socket buffers, backlog, somaxconn

Writes /var/lib/rvoip-bench/tuning.json (settings and read-back). Standard
library only.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import time
import urllib.request

OUT = Path('/var/lib/rvoip-bench/tuning.json')
COALESCE_USECS = 500
RX_RING = 8192
CONNTRACK_MAX = 4194304
CONNTRACK_HASH = 1048576
PORTS = {
    'dut': {'net.ipv4.ip_local_port_range': '65000 65535', 'net.ipv4.ip_local_reserved_ports': '10000-64999'},
    'caller': {'net.ipv4.ip_local_port_range': '10240 24999', 'net.ipv4.ip_local_reserved_ports': '25000-64999'},
}
SYSCTL = {
    'net.core.rmem_max': '33554432',
    'net.core.wmem_max': '33554432',
    'net.core.rmem_default': '4194304',
    'net.core.wmem_default': '4194304',
    'net.core.netdev_max_backlog': '250000',
    'net.core.somaxconn': '65535',
    'net.ipv4.tcp_max_syn_backlog': '65535',
    'net.ipv4.udp_mem': '8388608 12582912 16777216',
    'fs.file-max': '8388608',
    'fs.nr_open': '8388608',
    'vm.max_map_count': '1048576',
}


def sh(*argv, check=False):
    proc = subprocess.run(argv, capture_output=True, text=True)
    if check and proc.returncode:
        raise RuntimeError('%s: %s' % (' '.join(argv), proc.stderr.strip()))
    return proc


def cpus(text):
    out = []
    for part in text.strip().split(','):
        if part:
            lo, _, hi = part.partition('-')
            out.extend(range(int(lo), int(hi or lo) + 1))
    return out


def imds_tag(key):
    try:
        token = urllib.request.urlopen(urllib.request.Request(
            'http://169.254.169.254/latest/api/token', method='PUT',
            headers={'X-aws-ec2-metadata-token-ttl-seconds': '60'}), timeout=2).read().decode()
        return urllib.request.urlopen(urllib.request.Request(
            'http://169.254.169.254/latest/meta-data/tags/instance/' + key,
            headers={'X-aws-ec2-metadata-token': token}), timeout=2).read().decode().strip()
    except OSError:
        return None


def primary_interface():
    route = sh('ip', '-o', 'route', 'show', 'default').stdout.split()
    return route[route.index('dev') + 1] if 'dev' in route else None


def topology(iface):
    online = cpus(Path('/sys/devices/system/cpu/online').read_text())
    siblings = {cpu: cpus(Path('/sys/devices/system/cpu/cpu%d/topology/thread_siblings_list' % cpu).read_text())
                for cpu in online}
    queues = Path('/sys/class/net') / iface / 'queues'
    tx = sorted(int(p.name.split('-')[1]) for p in queues.glob('tx-*'))
    rx = sorted(int(p.name.split('-')[1]) for p in queues.glob('rx-*'))
    return {'interface': iface, 'online': online, 'siblings': siblings, 'tx_queues': tx, 'rx_queues': rx}


def nic_plan(facts):
    """e48_nic.plan: XPS round-robin, IRQ on the second SMT sibling of the first sender."""
    queues, online = facts['tx_queues'], sorted(facts['online'])
    xps = [[cpu for i, cpu in enumerate(online) if i % len(queues) == q] for q in range(len(queues))]
    cores = sorted({tuple(sorted(facts['siblings'][cpu])) for cpu in online})
    second = {core[0]: core[-1] for core in cores}
    irq = [second.get(group[0], group[0]) for group in xps]
    return {'xps': xps, 'irq_cpus': irq, 'smt': all(len(core) == 2 for core in cores)}


def mask(cpu_list):
    value = 0
    for cpu in cpu_list:
        value |= 1 << cpu
    text = '%x' % value
    groups = [text[max(0, i - 8):i] for i in range(len(text), 0, -8)][::-1]
    return ','.join(groups)


def queue_irqs(iface):
    """IRQ numbers of the interface's queues, in queue order (ENA: <iface>-Tx-Rx-<q>)."""
    found = {}
    for line in Path('/proc/interrupts').read_text().splitlines():
        match = re.search(r'^\s*(\d+):.*\b%s-Tx-Rx-(\d+)\s*$' % re.escape(iface), line)
        if match:
            found[int(match[2])] = int(match[1])
    if not found:
        msix = Path('/sys/class/net') / iface / 'device/msi_irqs'
        irqs = sorted(int(p.name) for p in msix.iterdir()) if msix.exists() else []
        found = {q: irq for q, irq in enumerate(irqs[1:])}   # ENA: the first vector is management
    return [found[q] for q in sorted(found)]


def apply(role):
    record = {'role': role, 'started_at': time.time(), 'errors': []}
    iface = primary_interface()
    record['interface'] = iface
    sh('systemctl', 'stop', 'irqbalance')
    sh('systemctl', 'disable', 'irqbalance')
    sysctl = dict(SYSCTL, **PORTS[role])
    if Path('/proc/sys/net/netfilter/nf_conntrack_max').exists():
        sysctl['net.netfilter.nf_conntrack_max'] = str(CONNTRACK_MAX)
        try:
            Path('/sys/module/nf_conntrack/parameters/hashsize').write_text(str(CONNTRACK_HASH))
        except OSError as error:
            record['errors'].append('conntrack hashsize: %s' % error)
        record['conntrack'] = 'loaded'
    else:
        record['conntrack'] = 'not loaded (no conntrack cost on this host)'
    for key, value in sysctl.items():
        proc = sh('sysctl', '-q', '-w', '%s=%s' % (key, value))
        if proc.returncode:
            record['errors'].append('sysctl %s: %s' % (key, proc.stderr.strip()))
    for name, value in (('enabled', 'always'), ('defrag', 'madvise')):
        try:
            Path('/sys/kernel/mm/transparent_hugepage/' + name).write_text(value)
        except OSError as error:
            record['errors'].append('thp %s: %s' % (name, error))
    if iface:
        ring = sh('ethtool', '-G', iface, 'rx', str(RX_RING))
        if ring.returncode and 'unmodified' not in ring.stderr:
            record['errors'].append('rx ring: %s' % ring.stderr.strip())
        coalesce = sh('ethtool', '-C', iface, 'adaptive-rx', 'off', 'rx-usecs', str(COALESCE_USECS),
                      'tx-usecs', str(COALESCE_USECS))
        if coalesce.returncode and 'unmodified' not in coalesce.stderr:
            record['errors'].append('coalesce: %s' % coalesce.stderr.strip())
        facts = topology(iface)
        plan = nic_plan(facts)
        record['plan'] = plan
        for q, group in enumerate(plan['xps']):
            path = Path('/sys/class/net') / iface / 'queues' / ('tx-%d' % q) / 'xps_cpus'
            try:
                path.write_text(mask(group))
            except OSError as error:
                record['errors'].append('xps %d: %s' % (q, error))
        irqs = queue_irqs(iface)
        record['queue_irqs'] = irqs
        for q, irq in enumerate(irqs):
            cpu = plan['irq_cpus'][q % len(plan['irq_cpus'])]
            try:
                Path('/proc/irq/%d/smp_affinity_list' % irq).write_text(str(cpu))
            except OSError as error:
                record['errors'].append('irq %d -> cpu %d: %s' % (irq, cpu, error))
        record['readback'] = {
            'ring': sh('ethtool', '-g', iface).stdout,
            'coalesce': sh('ethtool', '-c', iface).stdout,
            'channels': sh('ethtool', '-l', iface).stdout,
            'irq_affinity': {irq: Path('/proc/irq/%d/smp_affinity_list' % irq).read_text().strip() for irq in irqs},
        }
    record['readback_sysctl'] = {key: sh('sysctl', '-n', key).stdout.strip() for key in sysctl}
    record['thp'] = Path('/sys/kernel/mm/transparent_hugepage/enabled').read_text().strip()
    record['kernel'] = os.uname().release
    record['ended_at'] = time.time()
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(record, indent=1, sort_keys=True) + '\n')
    return record


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument('--role', choices=['dut', 'caller'])
    args = parser.parse_args()
    role = args.role or imds_tag('rvoip-role')
    if role not in PORTS:
        print('bench_tune: no rvoip-role tag (baking?); nothing to do', file=sys.stderr)
        return 0
    record = apply(role)
    print(json.dumps({'role': role, 'errors': record['errors']}))
    return 1 if record['errors'] else 0


if __name__ == '__main__':
    sys.exit(main())
