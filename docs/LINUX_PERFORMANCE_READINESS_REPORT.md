# rvoip Linux Performance Readiness and Optimization Report

Status: Review complete; implementation recommendations proposed  
Prepared: 2026-08-02  
Reviewed checkout: `ae2d7e98`  
Fetched `origin/main`: `e987ed1b` (14 commits ahead at review time)  
Scope: Linux performance-test hosts, Rust build/runtime configuration, SIP/RTP
socket capacity, platform-specific code, and performance evidence

## Executive conclusion

rvoip does need explicit Linux host preparation for high-density performance
tests, especially for open-file limits, UDP port ownership, socket buffers, and
kernel drop telemetry. The open-file requirement is already substantially
implemented: the current GCP worker sets `nofile` to 262,144 and the
infrastructure preflight opens and binds 4,096 UDP sockets before an expensive
performance gate is allowed to run.

The more important unresolved Linux risks are:

1. Several same-host RTP test ranges overlap Linux's default automatic port
   range. This can create intermittent bind conflicts or consume ports needed
   by unrelated outbound connections.
2. Performance profiles request 8 MiB SIP receive and send buffers, but the
   code records the requested values rather than reading back the values the
   Linux kernel actually granted.
3. The burst harness parses BSD/macOS-style `netstat` output. A normal Linux
   worker may therefore report no usable UDP drop counters while still running
   the test.
4. The macOS and Linux results are not yet a controlled CPU comparison. An
   Apple M3 Max and an N2 x86 virtual machine differ in architecture, core
   strength, SMT behavior, enabled instruction sets, and virtualization noise.

There is no evidence of a macOS-only optimized rvoip implementation that Linux
is missing. The workspace's normal release profile is already configured for
maximum runtime optimization. Adding generic Linux-specific Rust files is not
the recommended first response.

There are legitimate Rust optimization opportunities, including incomplete
SIMD dispatch and an avoidable audio-frame allocation/copy path, but those are
cross-platform improvements. They should be addressed only after Linux host
capacity and measurement validity are controlled.

## Decisions

| Question | Decision |
| --- | --- |
| Should Linux performance workers raise `nofile`? | Yes. Keep the existing 262,144 requirement and verify it inside each measured process. |
| Does Linux need ports to be "opened" globally? | No. UDP ports are bound per socket. The requirement is a collision-free port plan, enough local addresses, and sufficient descriptors. |
| Should the full RTP range simply be reserved in `ip_local_reserved_ports`? | No. Reserving 16,384-65,535 without moving the automatic port range would remove nearly all normal ephemeral ports. Partition the ranges deliberately or use multiple addresses/namespaces/hosts. |
| Should Linux socket-buffer ceilings be raised? | Yes, to at least the application's requested values, followed by `getsockopt` readback and drop-counter validation. |
| Should rvoip add Linux-only optimized Rust source files now? | No. First establish comparable hardware and valid Linux evidence. Optimize measured hot paths afterward. |
| Should the build use `target-cpu=native`? | Only for an experiment whose build and run CPU are guaranteed compatible. Do not make it the portable release default. |
| Will `lld`, sccache, or prebuilt bundles improve call throughput? | No. They improve compile/link/setup duration. They are still useful because they shorten total gate time. |
| Is profile-guided optimization worth evaluating? | Yes, after the environment is stable and representative profiles are available. |

## Review scope and limitations

This report is a static review of the repository and fetched release-runner
state. It does not claim a measured root cause because no matched-hardware
Linux/macOS benchmark or Linux profile was supplied with the request.

The review covered:

- Cargo release profiles and target-specific compiler configuration;
- platform-specific Rust source and socket behavior;
- RTP port allocation and same-host performance layouts;
- SIP and RTP UDP buffer requests;
- Linux worker startup and infrastructure preflight logic;
- performance scripts and resource sampling;
- CPU architecture and GCP machine assumptions; and
- likely Linux kernel and service limits at high socket counts.

The working tree already contained unrelated untracked content under `docs/`
and `outputs/`. It was not modified as part of this review.

## Separate the three meanings of "Linux is slower"

Before changing code, every comparison should identify which duration or rate
is slower.

### 1. Build and link duration

This includes dependency compilation, code generation, linking, and test
binary discovery. It is affected by:

- compiler cache hits;
- linker choice;
- number of Cargo codegen units;
- LTO mode;
- disk speed;
- number of parallel builds; and
- whether exact performance executables are built once or once per worker.

The 14 fetched commits after the reviewed checkout add `lld`, exact prebuilt
performance bundles, and additional build caching/orchestration. These should
substantially reduce GCP gate setup time, but they do not make a running SIP or
media loop process more calls per second.

### 2. Test-suite wall-clock duration

This includes how many independent test binaries and tests run concurrently.
The local ignored `.cargo/config.toml` sets `RUST_TEST_THREADS=1`. If that file
or equivalent environment setting is copied to Linux, tests inside each test
binary run serially. This can make the test suite appear slow without changing
library runtime performance.

Each performance result should record:

- `RUST_TEST_THREADS` and Cargo/nextest concurrency;
- time spent compiling and linking;
- time spent starting infrastructure;
- time spent in warm-up; and
- time spent in the measured interval.

### 3. Runtime throughput and latency

This is the relevant category for calls per second, packet processing,
post-dial delay, tail latency, active calls, or CPU per call. It is affected by
CPU capability, scheduler and virtualization behavior, socket limits, port
collisions, kernel drops, queue depths, memory pressure, and application hot
paths.

Build-system improvements must not be reported as runtime throughput
improvements, and total gate duration must not be used as a proxy for calls per
second.

## Current Rust optimization state

### Normal release profile is already runtime-oriented

The root `Cargo.toml` defines the normal release profile as:

```toml
[profile.release]
opt-level = 3
lto = true
codegen-units = 1
panic = "abort"
debug = true
strip = false
```

This is already an aggressive runtime configuration:

- `opt-level = 3` enables the optimizer's speed-oriented transformations;
- `lto = true` enables cross-crate link-time optimization;
- `codegen-units = 1` maximizes the optimizer's visibility at the expense of
  compile time; and
- retaining debug symbols supports meaningful Linux `perf` profiles without
  disabling release optimization.

The profile named `release-fast` is faster to compile, not necessarily faster
at runtime. It changes to thin LTO, 16 codegen units, and panic unwinding.
Cargo's documentation notes that more codegen units can improve compilation
time while potentially producing slower code.

Reference:
[Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)

### There is no repository-wide CPU target contract

The reviewed configuration does not set `target-cpu` or a repository-wide
target feature baseline. A generic `x86_64-unknown-linux-gnu` build therefore
uses a conservative x86-64 instruction baseline. An Apple AArch64 target has a
different baseline, and Apple Silicon also has substantially different
single-core performance and memory behavior.

An x86-specific build can be evaluated after the runner CPU contract is fixed:

- `-C target-cpu=x86-64-v3` is a reasonable portable experiment for a fleet
  whose minimum CPU explicitly supports that level;
- `-C target-cpu=cascadelake` is narrower and should be used only when that CPU
  contract is guaranteed; and
- `-C target-cpu=native` is appropriate for a diagnostic build that runs on
  the same machine, but it is unsafe as a general artifact policy when a
  builder and worker can expose different feature sets.

This matters because fetched `origin/main` builds performance executables on a
dedicated N2 builder and distributes them to separate N2 workers. The minimum
CPU platform should be pinned before enabling a machine-specific target.

References:
[rustc code-generation options](https://doc.rust-lang.org/rustc/codegen-options/index.html),
[Google Cloud general-purpose N2 machines](https://cloud.google.com/compute/docs/general-purpose-machines)

### Linker and cache changes are build optimizations

Fetched `origin/main` installs `lld` and sets:

```text
RUSTFLAGS=-C link-arg=-fuse-ld=lld
```

It also prebuilds exact performance executables once and distributes a
content-addressed bundle. These changes should be adopted before comparing
end-to-end gate duration. They should not be expected to change the throughput
of an already-running executable.

### Profile-guided optimization is a later-stage option

PGO can improve hot-path layout, inlining, and branch decisions using a
representative workload. It is worth testing only after the runner and
workload are stable. A profile produced from an unrepresentative smoke test can
make the real carrier workload worse.

Any PGO experiment should:

1. instrument the same release candidate and CPU target used by the test;
2. train on the expected mix of signaling, media, setup, teardown, and steady
   state;
3. merge profiles from multiple representative scenarios;
4. rebuild with the merged profile;
5. compare against an otherwise identical non-PGO binary; and
6. retain the compiler version, flags, profile digest, and binary digest in
   the evidence.

Reference:
[Rust profile-guided optimization](https://doc.rust-lang.org/nightly/rustc/profile-guided-optimization.html)

## Platform-specific code review

### No macOS-only fast path was found

The meaningful operating-system-specific runtime behavior is primarily socket
configuration. In `crates/media/rtp-core/src/transport/validation.rs`, macOS
and Linux both use:

- `SO_REUSEADDR=true`;
- `SO_REUSEPORT=false`;
- separate IPv4/IPv6 behavior; and
- 131,072-byte RTP send and receive buffer requests.

The only obvious difference in those strategies is a 500 ms macOS port-rebind
wait versus 250 ms on Linux. That difference favors Linux and cannot explain a
general macOS throughput advantage.

### SIMD support is incomplete on both architectures

`crates/media/codec-core/src/utils/simd.rs` detects SSE2 and AVX2 on x86-64 and
NEON on AArch64, but:

- AVX2 is detected and never used;
- the SSE2 G.711 path loads eight samples and then extracts every lane to call
  scalar conversion functions;
- the AArch64 NEON encoding functions explicitly fall back to the scalar
  implementation; and
- the public `AudioCodecExt::encode_to_buffer` and `decode_to_buffer` G.711
  methods use scalar loops rather than the SIMD dispatcher.

Separately, `crates/media/media-core/src/performance/simd.rs` reports SIMD as
unavailable and always uses scalar gain and RMS implementations. This may be a
sound choice for 160-sample G.711 frames, but the names and metrics imply an
optimization that is not active.

This is not evidence that macOS has a hidden optimized path. It is evidence
that any codec optimization should begin with a microbenchmark of the actual
public buffer APIs on representative x86 and AArch64 processors. A lookup-table
or compiler-autovectorized scalar implementation may outperform hand-written
intrinsics for small frames.

### The advertised zero-copy audio path still copies

In `crates/media/media-core/src/processing/audio/processor.rs`, the v2 capture
path obtains a pooled frame, may allocate a new output vector and apply unity
gain, discards that output, and then clones the input frame. It records
`zero_copy_used=true` even though the normal frame is cloned.

This is a real cross-platform hot-path candidate. It should be fixed only after
a profile confirms that this code participates in the measured workload. The
correct outcome is to process a pooled frame through the pipeline rather than
allocating work that is discarded.

## Linux file-descriptor readiness

### Existing implementation

The current `infra/release-runners/gcp-release-startup.sh` already performs:

```sh
ulimit -n 262144
test "$(ulimit -n)" -ge 262144
```

The current infrastructure preflight also verifies the limit and opens 4,096
simultaneously bound UDP sockets. This is a strong early failure check and
should remain.

### Capacity model

Every UDP socket consumes a file descriptor. rvoip's default RTP allocator
uses ports 16,384-32,767 and defaults to RTP/RTCP multiplexing. A useful
planning equation is:

```text
required nofile >= fixed process overhead
                 + active media sessions * sockets per media session
                 + SIP/listener sockets
                 + harness/capture sockets
                 + 25%-50% headroom
```

For a same-host test in which both call endpoints are local:

- with RTCP multiplexing, full media is approximately two media sockets per
  active call across the two endpoint processes;
- without RTCP multiplexing, it is approximately four; and
- packet capture, SIP shards, metrics, log files, pipes, and loaded libraries
  add overhead.

At 10,000 active same-host calls, approximately 20,000 media descriptors with
RTCP multiplexing is a reasonable order-of-magnitude estimate. A 262,144 limit
therefore provides substantial headroom.

The exact count must be measured, not assumed. Record these values at baseline,
peak load, end of load, and after cleanup:

```sh
cat /proc/$PID/limits
find /proc/$PID/fd -mindepth 1 -maxdepth 1 | wc -l
cat /proc/sys/fs/nr_open
cat /proc/sys/fs/file-max
cat /proc/sys/fs/file-nr
```

### Service-level limits still matter

A shell `ulimit` affects that shell and its descendants. A production systemd
service needs an explicit `LimitNOFILE=` contract, and the running process's
`/proc/$PID/limits` is authoritative. PAM limits or an interactive shell are
not proof that a service received the same limit.

`fs.nr_open` is the kernel ceiling for a process's file descriptors, while
`fs.file-max` controls the system-wide file table. Do not increase
`fs.file-max` by default; record `file-nr` and change the system-wide ceiling
only if calculated demand and observed use require it.

Reference:
[Linux filesystem sysctls](https://docs.kernel.org/admin-guide/sysctl/fs.html)

## Linux UDP port readiness

### Ports are not a tunable count like file descriptors

Linux does not require a setting that globally "opens more UDP ports." A
process binds a UDP socket to an address and a 16-bit port. Capacity is bounded
by available address/port combinations, descriptor limits, memory, and
conflicts.

Using more local IP addresses can multiply the usable address/port tuples,
provided the applications bind those specific addresses rather than a wildcard
address. Separate network namespaces or separate hosts provide stronger
isolation for very large two-ended tests.

### Current rvoip ranges overlap Linux automatic allocation

Linux commonly defaults `net.ipv4.ip_local_port_range` to 32,768-60,999. That
range controls automatic selection for port-zero binds and outbound
connections. Explicit RTP binds are allowed outside or inside it, but an
automatic allocation can occupy a port immediately before rvoip tries to bind
the same address and port.

Current examples of overlap include:

| Test/configuration | Explicit media pools | Linux default overlap |
| --- | --- | --- |
| Default RTP allocator | 16,384-32,767 | None, except the 32,767/32,768 boundary is adjacent |
| PBX/carrier recipe capacity | 16,384-65,535 | 32,768-60,999 |
| Same-host burst Bob | 4,000-25,999 | None |
| Same-host burst Alice | 27,000-49,151 | 32,768-49,151 |
| Call-setup Bob | 16,384-40,999 | 32,768-40,999 |
| Call-setup Alice | 51,000-65,535 | 51,000-60,999 |

The comment in `crates/sip/rvoip-sip/tests/perf/support/burst.rs` says that
keeping the upper pool below 49,152 avoids the default dynamic/private range.
That reflects the IANA dynamic/private range, not the common Linux automatic
allocation default, so it does not establish collision safety on Linux.

Reference:
[Linux `ip_local_port_range` and `ip_local_reserved_ports`](https://docs.kernel.org/networking/ip-sysctl.html)

### Recommended port policy

For every performance topology:

1. Read and record `ip_local_port_range` and `ip_local_reserved_ports`.
2. Declare non-overlapping SIP, RTP/RTCP, harness, and automatic client ranges.
3. Reject a run when the declared capacity does not fit the topology.
4. Reserve explicit RTP ranges from automatic allocation when the reservation
   leaves an adequate automatic range.
5. Merge with existing `ip_local_reserved_ports`; writing this sysctl replaces
   the previous list.
6. Use separate local IPs, namespaces, or hosts when one IP cannot provide both
   the required media tuples and a healthy automatic port pool.

Do not reserve the full 16,384-65,535 recipe range while leaving the default
automatic range at 32,768-60,999. That would reserve the entire default
automatic range and could break port-zero binds, package downloads, control
traffic, and the existing 4,096-socket preflight.

Two viable large-test designs are:

- run caller and receiver on separate VMs, each with its own complete port
  namespace; or
- bind caller and receiver to distinct loopback addresses or network
  namespaces and validate that no wildcard bind shadows the specific
  addresses.

The separate-VM design is a better production analogue and avoids charging
both endpoints to one VM's CPU, scheduler, descriptor table, and UDP memory.

## Linux UDP buffer readiness

### Application requests

The PBX and carrier performance recipes request:

```text
SO_RCVBUF = 8,388,608 bytes
SO_SNDBUF = 8,388,608 bytes
```

`crates/sip/sip-transport/src/transport/udp/socket.rs` applies both requests
before binding. It does not call `getsockopt` afterward. On Linux, a successful
request does not prove the requested effective capacity was granted; kernel
ceilings and Linux's socket-buffer accounting affect the returned value.

The RTP strategy separately requests 131,072-byte send and receive buffers for
each RTP socket on both macOS and Linux. At high socket counts, changing the
per-RTP-socket value has a much larger aggregate-memory consequence than
changing the small number of SIP listener sockets.

### Runner state

The reviewed checkout does not set network socket-buffer sysctls in the GCP
startup script. Fetched `origin/main` adds:

```text
net.core.rmem_max = 67,108,864
```

That change is documented as protection for a 32 MiB packet-capture buffer. It
does not set `net.core.wmem_max`, so it does not establish that the requested
8 MiB SIP send buffer is available.

### Recommendation

The performance runner should:

1. require `net.core.rmem_max` to be at least the largest requested receive
   buffer;
2. require `net.core.wmem_max` to be at least the largest requested send
   buffer;
3. read back `SO_RCVBUF` and `SO_SNDBUF` from every important SIP listener and
   record both requested and effective values;
4. document Linux's returned-value accounting so evidence does not mistake the
   reported doubled value for an additional application request;
5. monitor aggregate UDP memory while under load; and
6. tune per-RTP-socket buffers only in response to observed queue drops.

Useful host evidence includes:

```sh
sysctl net.core.rmem_default net.core.rmem_max
sysctl net.core.wmem_default net.core.wmem_max
sysctl net.ipv4.udp_mem
cat /proc/net/sockstat
```

Using 64 MiB for both maxima is a simple symmetric runner ceiling if the host
has enough RAM, but rvoip only requires a send ceiling of at least 8 MiB for
the current SIP recipe. The ceiling should not be confused with memory that
must be committed to every socket.

Reference:
[Linux network core sysctls](https://docs.kernel.org/admin-guide/sysctl/net.html)

## Linux UDP drop telemetry gap

The current burst script invokes `netstat -s -p udp` and looks for exact labels
such as:

- `dropped due to full socket buffers`;
- `dropped due to no socket`; and
- `open UDP sockets`.

These are BSD/macOS-style labels. Linux commonly exposes different `netstat`
labels, and the GCP startup package list does not install the legacy
`net-tools` package that supplies `netstat`. When the command is missing, the
script writes `available=false` and continues. As a result, a Linux performance
artifact can contain no authoritative full-buffer-drop delta.

This is a P0 evidence problem. Throughput and application success counts alone
cannot distinguish a healthy run from a kernel queue that dropped packets and
was masked by retransmission or test behavior.

The Linux collector should read stable kernel sources that do not depend on
localized command output:

- `/proc/net/snmp`, Udp: `InDatagrams`, `NoPorts`, `InErrors`, `OutDatagrams`,
  `RcvbufErrors`, and `SndbufErrors`;
- `/proc/net/sockstat`, especially UDP socket count and memory;
- `/proc/net/softnet_stat` for network-stack backlog drops;
- `/proc/$PID/fd` for process descriptor count; and
- interface counters for real-NIC tests.

Collect snapshots before warm-up, immediately before the measured window,
after the measured window, and after cleanup. Calculate deltas only across the
measured window. The gate should fail closed when mandatory Linux counters are
missing or unparsable.

For a host shared by multiple concurrent gates, system-wide UDP deltas are not
fully attributable. Performance workers should remain exclusive, or the test
should supplement system counters with per-process/eBPF evidence.

## CPU and VM comparison readiness

### Why the current macOS/Linux comparison is not conclusive

The reviewed macOS host was an Apple M3 Max. The fetched release design uses a
mix of N2 worker sizes for performance and soak gates and a larger N2 builder
for exact binaries. An N2 vCPU is not equivalent to one Apple performance
core, and VM results may include SMT contention, host scheduling, and steal
time.

The comparison must control or record:

- CPU vendor, family, model, stepping, and exposed features;
- VM machine type and minimum CPU platform;
- vCPU count and whether sibling threads share a physical core;
- kernel, libc, rustc, Cargo profile, feature flags, and binary digest;
- CPU utilization normalized to available logical CPUs;
- steal time and other tenants/noisy-neighbor effects;
- memory capacity and pressure;
- thermal/power state where visible;
- workload topology, including whether both call ends share one host; and
- at least five independent repetitions.

Use a Linux machine as the release-performance baseline if Linux is the
deployment target. Keep macOS results as developer-regression information, not
as the pass/fail threshold for Linux.

### Required machine attestation

At minimum, save:

```sh
uname -a
rustc -vV
cargo -V
lscpu
cat /proc/cpuinfo
cat /proc/meminfo
cat /proc/loadavg
cat /proc/pressure/cpu
cat /proc/pressure/memory
cat /proc/pressure/io
cat /proc/stat
```

The harness should derive steal-time deltas from `/proc/stat` during the
measured window. A run with material steal time should be rejected or marked
invalid rather than treated as a library regression.

## Other Linux-specific readiness requirements

### Memory and swap

Record and gate on:

- swap configured and swap activity during the test;
- major page faults;
- memory pressure-stall information;
- reclaim activity;
- OOM kills; and
- process RSS at baseline, peak, end of load, and post-cleanup.

Latency tests should normally run without active swapping. Do not disable
transparent huge pages or change allocator policy without a profile and an A/B
result. The fetched branch already improves file-backed resource sampling so
that the sampler does not manufacture an RSS slope by growing its own in-memory
history during the measurement.

### Scheduling, affinity, and NUMA

Record task migrations, context switches, and per-CPU saturation. CPU affinity
can help only when a profile shows migration or contention; it can also make
results worse by pinning application tasks and kernel network work to the same
CPU. NUMA binding is relevant on larger multi-node machines, not a default
requirement for every N2 worker.

Do not introduce real-time scheduling as a general performance fix. It changes
system fairness and can starve control, logging, or kernel work.

### Loopback versus real network interfaces

Most same-host tests exercise Linux loopback. NIC queue, RSS, RPS, RFS, XPS,
interrupt affinity, and `net.core.netdev_max_backlog` tuning are not valid
first-line fixes for a loopback-only bottleneck.

For cross-host or physical-NIC tests, capture interface drops and queue
statistics, then evaluate RSS/RPS and interrupt distribution only when a queue
or CPU is demonstrably saturated.

Reference:
[Linux networking scaling](https://docs.kernel.org/networking/scaling.html)

### Firewall, containers, and connection tracking

Docker, NAT, firewall rules, or network namespaces can place traffic through
netfilter and connection tracking. When those components are part of the
tested topology, record:

- `nf_conntrack_count` and `nf_conntrack_max` when available;
- conntrack insertion/drop statistics;
- namespace and firewall ruleset identity; and
- container CPU/memory limits and throttling.

Do not disable connection tracking globally merely to improve a benchmark; a
production-representative topology may depend on it.

### TCP-specific sysctls are not general SIP/UDP fixes

The primary performance profiles use UDP. The following are not first-line
controls for those tests:

- `net.core.somaxconn`;
- TCP SYN backlog;
- TIME_WAIT reuse;
- TCP keepalive; and
- TCP congestion-control selection.

They should be considered only for dedicated SIP-over-TCP, TLS, WebSocket, or
WebRTC transport tests, with separate evidence and thresholds.

## Recommended implementation backlog

### P0: required before trusting Linux performance results

#### LNX-PERF-001: Preserve and extend runner preflight

Keep the existing `nofile=262144` and 4,096 UDP socket probe. Add evidence for:

- soft and hard `RLIMIT_NOFILE`;
- `/proc/self/limits` in the actual gate process;
- `fs.nr_open`, `fs.file-max`, and `fs.file-nr`;
- `ip_local_port_range` and `ip_local_reserved_ports`;
- `rmem_max`, `wmem_max`, and `udp_mem`;
- kernel, CPU, memory, pressure, and swap state; and
- availability of all mandatory telemetry inputs.

Acceptance criteria:

- the runner fails before compilation when a required limit is insufficient;
- the attestation is included in the performance artifact; and
- the process executing the workload proves it inherited the expected limit.

#### LNX-PERF-002: Make the port topology Linux-aware

Add a single port-layout validator that understands:

- all SIP listener/shard ports;
- all caller and receiver RTP/RTCP pools;
- the host automatic port range;
- reserved ports;
- bind addresses and wildcard conflicts; and
- required active-session capacity.

Correct the claim that an upper port below 49,152 avoids Linux's automatic
range. For the largest same-host tests, select separate IPs/namespaces or move
the two call sides to separate workers.

Acceptance criteria:

- no explicit media pool overlaps automatic allocation unless it is safely
  reserved;
- the automatic range retains sufficient capacity for the runner and control
  plane; and
- the harness refuses an impossible or conflicting topology before load.

#### LNX-PERF-003: Add Linux-native UDP evidence

Replace or supplement the BSD `netstat` parser with a `/proc/net/snmp` and
`/proc/net/sockstat` collector. Add softnet and relevant interface counters.

Acceptance criteria:

- `RcvbufErrors`, `SndbufErrors`, `InErrors`, and `NoPorts` deltas are numeric;
- mandatory counters cannot silently become `n/a` on a release gate;
- snapshots bracket the measured interval; and
- a nonzero unexpected drop delta fails or explicitly invalidates the run.

#### LNX-PERF-004: Verify effective socket buffers

Add `getsockopt` readback for the SIP UDP socket and expose requested/effective
receive and send sizes in the performance report. Set and verify both host
ceilings.

Acceptance criteria:

- every performance artifact includes requested and effective buffer sizes;
- Linux accounting is normalized/documented;
- an effective size below the scenario requirement fails preflight or startup;
  and
- UDP drop counters remain zero during the accepted measurement.

### P1: required for a defensible Linux baseline

#### LNX-PERF-005: Pin and attest the CPU contract

Pin the Linux release baseline to a machine family, size, and minimum CPU
platform. Save CPU features and the exact binary digest. Reject runs with
material steal time, active swap, or unexpected resource limits.

#### LNX-PERF-006: Separate build and runtime reporting

Report provisioning, compilation, linking, warm-up, measurement, and artifact
upload durations independently. Adopt fetched `origin/main`'s `lld` and exact
prebuilt-bundle work before using end-to-end gate duration as a trend metric.

#### LNX-PERF-007: Track descriptor and socket lifetime

Sample open descriptors and UDP sockets at baseline, peak, end of load, and
after cleanup. Establish distinct thresholds for capacity exhaustion and leaks.

#### LNX-PERF-008: Establish repeatability rules

Run at least five repetitions per candidate configuration. Report median and
dispersion, not only the best run. Invalidate outliers caused by steal time,
kernel drops, swapping, or a changed environment rather than deleting them
without evidence.

### P2: measured Rust optimization work

#### LNX-PERF-009: Benchmark CPU target variants

On the pinned Linux baseline, compare otherwise identical binaries:

1. current generic release;
2. `x86-64-v3`;
3. a pinned microarchitecture target; and
4. `native` as a same-machine diagnostic only.

Compare call throughput, setup/tail latency, CPU seconds per completed call,
and binary portability. Promote a target only when it has a meaningful,
repeatable win and a documented deployment CPU contract.

#### LNX-PERF-010: Repair or remove misleading SIMD paths

Benchmark the real public G.711 APIs at normal frame sizes. Then either:

- implement a genuinely vectorized AVX2/AArch64 path with function-level
  runtime dispatch;
- use a faster scalar or lookup-table path; or
- remove dispatch that adds overhead without doing vector work.

Do not select an implementation from instruction counts alone; measure the
complete encode/decode call on supported CPUs.

#### LNX-PERF-011: Complete the zero-copy audio pipeline

Remove the allocated-and-discarded unity-gain output and carry the pooled frame
through the actual processing pipeline. Correct the metric so `zero_copy_used`
means that the frame avoided a copy.

#### LNX-PERF-012: Evaluate PGO

Train on representative carrier signaling and media scenarios and compare with
the generic optimized release. Treat the profile and compiler version as
versioned build inputs.

## Proposed benchmark sequence

### Stage 1: Environment qualification

Run no application load. Produce one immutable environment attestation and
verify:

- resource limits;
- port layout;
- socket-buffer ceilings;
- telemetry availability;
- CPU/memory/pressure state; and
- binary/toolchain identity.

### Stage 2: Capacity probes

Before a long benchmark:

1. retain the 4,096 ephemeral UDP bind probe;
2. bind representative sockets from every declared media pool;
3. probe the scenario's projected peak descriptor count or a safe sampled
   equivalent;
4. set and read back the requested SIP buffers; and
5. verify that counters change when a controlled diagnostic condition is
   introduced in a non-release test, proving the collector works.

### Stage 3: Linux baseline

Use the current generic release profile on the pinned Linux worker. Run:

- signaling-only setup load;
- full-media setup load;
- burst/microburst load;
- sustained steady state; and
- teardown/cleanup.

Require zero unexplained kernel drops and preserve peak/post-cleanup descriptor
and memory evidence.

### Stage 4: Compiler A/B matrix

Reuse the same environment and workload while changing only the compiler CPU
target. Randomize or alternate run order to reduce host-time bias. Rebuild exact
binaries and preserve their digests.

### Stage 5: Application profiles

Collect Linux `perf` samples from the optimized release with symbols. Attribute
CPU to:

- SIP parsing and serialization;
- transaction/dialog dispatch;
- allocator and lock contention;
- RTP send/receive loops;
- codec/audio processing;
- memory allocation and copying;
- Tokio scheduling; and
- kernel/network system calls.

Only after this profile should a Rust hot path be selected for implementation.

### Stage 6: Soak validation

Promote a change only after a sustained run demonstrates:

- stable throughput and tail latency;
- no descriptor or socket leak;
- acceptable RSS slope;
- no active swapping or pressure stalls;
- zero unexplained UDP buffer errors; and
- successful cleanup back to the defined steady baseline.

## Minimum artifact schema

Every Linux performance result should include at least:

```text
candidate SHA
binary SHA-256
rustc version and full RUSTFLAGS
Cargo profile and feature set
kernel/libc identity
cloud machine type and CPU model/features
vCPU and memory capacity
soft/hard nofile and process /proc limits
fs.nr_open, fs.file-max, fs.file-nr
declared SIP and media port pools
ip_local_port_range and ip_local_reserved_ports
requested/effective SO_RCVBUF and SO_SNDBUF
rmem_max, wmem_max, udp_mem
UDP/softnet/interface counters before and after measurement
open descriptors and socket count at lifecycle checkpoints
CPU, steal, RSS, faults, swap, and PSI samples
warm-up and measured-window timestamps
throughput, success/failure counts, and latency histogram
topology: same host, namespaces, or separate hosts
```

Without this context, a Linux/macOS number is useful for investigation but not
for a release threshold or optimization decision.

## Settings that should not be changed without evidence

Avoid a generic "performance sysctl" bundle. In particular, do not change the
following merely because the host runs many UDP calls:

- `fs.file-max`, unless system-wide file-table use approaches the limit;
- TCP backlog, TIME_WAIT, or congestion settings for a UDP benchmark;
- `net.core.netdev_max_backlog` for a loopback-only benchmark without observed
  softnet/interface drops;
- RSS/RPS/RFS/XPS or IRQ affinity without a real-NIC queue/CPU bottleneck;
- conntrack policy unless the benchmark traverses netfilter/NAT and its counters
  show pressure;
- transparent huge pages without memory/profile evidence;
- real-time priority as a substitute for scheduler diagnosis; or
- `SO_REUSEPORT` for media sockets, because duplicate binds can distribute
  packets to the wrong session and compromise correctness.

The performance runner should be explicit, minimal, attested, and tied to
observed failure modes.

## Final recommendation

The immediate work should focus on Linux test validity, not Linux-only Rust
code:

1. keep the existing 262,144 `nofile` policy and extend descriptor lifecycle
   evidence;
2. redesign or validate the same-host port ranges against Linux automatic port
   allocation;
3. verify both receive and send socket-buffer ceilings and read back effective
   buffer sizes;
4. replace the BSD/macOS UDP counter parser with Linux-native, fail-closed
   telemetry;
5. adopt the fetched build/link/prebuilt improvements and report build time
   separately from runtime;
6. pin the Linux CPU baseline and collect repeated results; and
7. profile the matched Linux binary before implementing SIMD, zero-copy,
   `target-cpu`, or PGO changes.

Once those controls are in place, a remaining Linux runtime gap will be a
credible application optimization signal. Until then, the gap can be explained
by hardware asymmetry, build/test accounting, port contention, kernel buffer
limits, or missing drop evidence just as plausibly as by Rust-generated code.

## Reviewed repository locations

- `Cargo.toml` (`[profile.release]`, `[profile.release-fast]`)
- `.cargo/config.toml` (local ignored test-thread setting)
- `infra/release-runners/gcp-release-startup.sh`
- `infra/release-runners/release-infrastructure-preflight.sh`
- `crates/media/rtp-core/src/transport/allocator.rs`
- `crates/media/rtp-core/src/transport/validation.rs`
- `crates/media/rtp-core/src/transport/udp.rs`
- `crates/media/codec-core/src/utils/simd.rs`
- `crates/media/codec-core/src/codecs/g711/mod.rs`
- `crates/media/media-core/src/performance/simd.rs`
- `crates/media/media-core/src/processing/audio/processor.rs`
- `crates/sip/sip-transport/src/transport/udp/socket.rs`
- `crates/sip/rvoip-sip/config/performance-recipes.yaml`
- `crates/sip/rvoip-sip/tests/perf/support/burst.rs`
- `crates/sip/rvoip-sip/tests/perf/perf_call_setup_cps.rs`
- `crates/sip/rvoip-sip/tests/perf/support/sampler.rs`
- `crates/sip/rvoip-sip/scripts/perf_burst_matrix.sh`
- fetched `origin/main` release-runner and prebuilt-performance changes through
  `e987ed1b`

## External references

- [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html)
- [rustc code-generation options](https://doc.rust-lang.org/rustc/codegen-options/index.html)
- [Rust profile-guided optimization](https://doc.rust-lang.org/nightly/rustc/profile-guided-optimization.html)
- [Linux filesystem sysctls](https://docs.kernel.org/admin-guide/sysctl/fs.html)
- [Linux IP sysctls](https://docs.kernel.org/networking/ip-sysctl.html)
- [Linux network core sysctls](https://docs.kernel.org/admin-guide/sysctl/net.html)
- [Linux networking scaling](https://docs.kernel.org/networking/scaling.html)
- [Google Cloud general-purpose N2 machines](https://cloud.google.com/compute/docs/general-purpose-machines)
