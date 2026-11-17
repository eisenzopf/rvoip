#!/usr/bin/env python3
"""Sample host CPU and one process's per-thread CPU and RSS until it exits.

Ported from the E74 harness (host-cpu-sampler.py for the host counters, and
the per-thread CPU sampling of its resource observer). Run on each bench host
beside the process under test (rvoip-bench-uas on the DUT, rvoip-loadgen on a
caller):

    host_sampler.py --pid <pid> --out resources.jsonl [--interval 2] [--iface <nic>]

One JSON line per interval until <pid> exits (or is replaced by a new process
with the same pid):

  at                  wall clock (epoch s)
  pid, logical_cpus, physical_cores (distinct core ids per package)
  host_cpu_pct        host busy %% over the interval: wall minus cpuidle
                      residency when the cpuidle files exist (exact, includes
                      irq/softirq on otherwise idle CPUs), else /proc/stat ticks
  host_cpu_pct_ticks  host busy %% from /proc/stat ticks
  host                raw counters: run_ns (per-CPU /proc/schedstat run time),
                      idle_us (per-CPU cpuidle residency), ksoftirqd_ns,
                      net_rx / net_tx (/proc/softirqs), stat (per-CPU ticks:
                      user nice system idle iowait irq softirq steal, USER_HZ;
                      caller_policy.softirq_saturation reads softirq and idle
                      per CPU from these)
  irq_iface, irq_cpus the primary NIC (--iface, else the default route's) and
                      the CPUs its queue IRQs are affined to (/proc/interrupts
                      names <iface>-*, /proc/irq/<n>/effective_affinity_list);
                      read once at start. Older files lack them: the reader
                      falls back to the host's tuning.json, then every CPU.
  cpu_cores_pct       process CPU over the interval (100 = one core); absent
                      on the first line
  rss_bytes, threads
  thread_cpu_times    [{id, user_time, system_time}] (seconds, cumulative)
  thread_names        {tid: comm}

The thread rows are what caller_policy.thread_peaks reads (no thread above 0.8
core). Linux only; standard library only.
"""
import argparse
import glob
import json
import os
from pathlib import Path
import sys
import time

HZ = os.sysconf('SC_CLK_TCK') if hasattr(os, 'sysconf') else 100
PAGE = os.sysconf('SC_PAGE_SIZE') if hasattr(os, 'sysconf') else 4096


def parse_proc_stat(text):
    """/proc/<pid>/stat (or task stat) -> utime, stime (ticks), starttime, threads, rss (pages)."""
    end = text.rindex(')')
    fields = text[end + 2:].split()
    return {'state': fields[0], 'utime': int(fields[11]), 'stime': int(fields[12]), 'threads': int(fields[17]),
            'starttime': int(fields[19]), 'rss_pages': int(fields[21])}


def physical_cores():
    """Distinct (package, core) pairs of the online CPUs, or None."""
    pairs = set()
    for path in glob.glob('/sys/devices/system/cpu/cpu[0-9]*/topology/core_id'):
        base = Path(path).parent
        try:
            pairs.add((base.joinpath('physical_package_id').read_text().strip(), Path(path).read_text().strip()))
        except OSError:
            continue
    return len(pairs) or None


def idle_paths():
    paths = {}
    for path in glob.glob('/sys/devices/system/cpu/cpu[0-9]*/cpuidle/state[0-9]*/time'):
        cpu = path.split('/')[5][3:]
        paths.setdefault(cpu, []).append(path)
    return paths


def ksoftirqd_schedstats():
    paths = []
    for path in glob.glob('/proc/[0-9]*/comm'):
        try:
            if Path(path).read_text().startswith('ksoftirqd/'):
                paths.append(path.rsplit('/', 1)[0] + '/schedstat')
        except OSError:
            pass
    return paths


def primary_iface():
    """The interface of the IPv4 default route (/proc/net/route), or None."""
    try:
        with open('/proc/net/route') as stream:
            next(stream, None)
            for line in stream:
                parts = line.split()
                if len(parts) > 2 and parts[1] == '00000000':
                    return parts[0]
    except OSError:
        pass
    return None


def parse_cpu_list(text):
    """'48-51,60' -> [48, 49, 50, 51, 60]."""
    cpus = set()
    for part in (text or '').strip().split(','):
        part = part.strip()
        if not part:
            continue
        low, _, high = part.partition('-')
        try:
            cpus.update(range(int(low), int(high or low) + 1))
        except ValueError:
            continue
    return sorted(cpus)


def nic_irq_cpus(iface, interrupts='/proc/interrupts', irq_root='/proc/irq'):
    """CPUs that the queue IRQs of `iface` (interrupt names starting with '<iface>-') are affined to, or None."""
    if not iface:
        return None
    irqs = []
    try:
        with open(interrupts) as stream:
            for line in stream:
                number, _, rest = line.partition(':')
                if number.strip().isdigit() and any(name.startswith(iface + '-') for name in rest.split()):
                    irqs.append(number.strip())
    except OSError:
        return None
    cpus = set()
    for irq in irqs:
        for name in ('effective_affinity_list', 'smp_affinity_list'):
            try:
                found = parse_cpu_list(Path(irq_root, irq, name).read_text())
            except OSError:
                continue
            if found:
                cpus.update(found)
                break
    return sorted(cpus) or None


def host_counters(idle, ksoft):
    row = {}
    residency = {}
    for cpu, paths in idle.items():
        total = 0
        for path in paths:
            try:
                total += int(Path(path).read_text())
            except (OSError, ValueError):
                pass
        residency[cpu] = total
    row['idle_us'] = residency
    run, stat = {}, {}
    try:
        with open('/proc/schedstat') as stream:
            for line in stream:
                if line.startswith('cpu'):
                    parts = line.split()
                    run[parts[0][3:]] = int(parts[7])
    except (OSError, IndexError, ValueError):
        pass
    row['run_ns'] = run
    with open('/proc/stat') as stream:
        for line in stream:
            if line.startswith('cpu'):
                parts = line.split()
                stat[parts[0]] = [int(x) for x in parts[1:9]]
    row['stat'] = stat
    total = 0
    for path in ksoft:
        try:
            total += int(Path(path).read_text().split()[0])
        except (OSError, ValueError, IndexError):
            pass
    row['ksoftirqd_ns'] = total
    try:
        with open('/proc/softirqs') as stream:
            for line in stream:
                name, _, values = line.partition(':')
                if name.strip() in ('NET_RX', 'NET_TX'):
                    row[name.strip().lower()] = sum(int(v) for v in values.split())
    except OSError:
        pass
    return row


def busy_pct_ticks(before, after):
    """Host busy % between two /proc/stat 'cpu' rows (user nice system idle iowait irq softirq steal)."""
    if not before or not after:
        return None
    delta = [b - a for a, b in zip(before, after)]
    total = sum(delta)
    idle = delta[3] + delta[4]
    return 100.0 * (total - idle) / total if total > 0 else None


def busy_pct_idle(before, after, elapsed, cpus):
    """Host busy % from cpuidle residency (us) over `elapsed` seconds on `cpus` CPUs."""
    if not before or not after or elapsed <= 0 or not cpus or set(before) != set(after):
        return None
    idle = sum(after[cpu] - before[cpu] for cpu in after) / 1e6
    return max(0.0, min(100.0, 100.0 * (1 - idle / (elapsed * cpus))))


def process_sample(pid):
    """Process and per-thread CPU of `pid` (None when it is gone)."""
    base = Path('/proc', str(pid))
    try:
        process = parse_proc_stat(base.joinpath('stat').read_text())
    except (OSError, ValueError, IndexError):
        return None
    if process['state'] in ('Z', 'X'):
        return None   # exited (a zombie waiting for its parent's wait())
    times, names = [], {}
    for task in base.joinpath('task').glob('[0-9]*'):
        try:
            stat = parse_proc_stat(task.joinpath('stat').read_text())
            name = task.joinpath('comm').read_text().strip()
        except (OSError, ValueError, IndexError):
            continue
        times.append({'id': int(task.name), 'user_time': stat['utime'] / HZ, 'system_time': stat['stime'] / HZ})
        names[task.name] = name
    return {'process': process, 'thread_cpu_times': times, 'thread_names': names}


def run(pid, out, interval=2.0, max_samples=None, clock=time, iface=None):
    """Sample until `pid` exits; returns the number of lines written."""
    iface = iface or primary_iface()
    irq_cpus = nic_irq_cpus(iface)
    idle = idle_paths()
    ksoft = ksoftirqd_schedstats()
    cpus = os.cpu_count()
    cores = physical_cores()
    first = process_sample(pid)
    if first is None:
        return 0
    identity = first['process']['starttime']
    previous = None
    written = 0
    with open(out, 'a') as stream:
        while max_samples is None or written < max_samples:
            sample = process_sample(pid)
            if sample is None or sample['process']['starttime'] != identity:
                break
            at = clock.time()
            host = host_counters(idle, ksoft)
            process = sample['process']
            row = {'at': at, 'pid': pid, 'logical_cpus': cpus, 'physical_cores': cores,
                   'rss_bytes': process['rss_pages'] * PAGE, 'threads': process['threads'],
                   'thread_cpu_times': sample['thread_cpu_times'], 'thread_names': sample['thread_names'], 'host': host,
                   'irq_iface': iface, 'irq_cpus': irq_cpus}
            cpu_seconds = (process['utime'] + process['stime']) / HZ
            if previous is not None:
                elapsed = at - previous['at']
                if elapsed > 0:
                    row['cpu_cores_pct'] = 100.0 * (cpu_seconds - previous['cpu_seconds']) / elapsed
                ticks = busy_pct_ticks(previous['host']['stat'].get('cpu'), host['stat'].get('cpu'))
                row['host_cpu_pct_ticks'] = ticks
                exact = busy_pct_idle(previous['host']['idle_us'], host['idle_us'], elapsed, cpus) if idle else None
                row['host_cpu_pct'] = exact if exact is not None else ticks
                row['host_cpu_basis'] = 'cpuidle residency' if exact is not None else '/proc/stat ticks'
            stream.write(json.dumps(row, separators=(',', ':')) + '\n')
            stream.flush()
            written += 1
            previous = {'at': at, 'cpu_seconds': cpu_seconds, 'host': host}
            time.sleep(interval)
    return written


def main(argv=None):
    parser = argparse.ArgumentParser(description='Sample host CPU and a process\'s per-thread CPU until it exits.')
    parser.add_argument('--pid', type=int, required=True)
    parser.add_argument('--out', required=True)
    parser.add_argument('--interval', type=float, default=2.0)
    parser.add_argument('--max-samples', type=int, default=None)
    parser.add_argument('--iface', default=None, help='primary NIC (default: the default route\'s interface)')
    args = parser.parse_args(argv)
    if args.pid <= 1:
        parser.error('--pid must name the process under test')
    written = run(args.pid, args.out, args.interval, args.max_samples, iface=args.iface)
    print(json.dumps({'pid': args.pid, 'out': args.out, 'samples': written}))
    return 0 if written else 1


if __name__ == '__main__':
    sys.exit(main())
