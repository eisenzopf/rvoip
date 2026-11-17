//! Process and socket figures read from `/proc` (Linux). On other platforms
//! every figure reads as zero.

/// Process CPU, memory and thread figures.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ProcessStats {
    /// User plus system CPU time, seconds.
    pub cpu_seconds: f64,
    /// Resident set size, bytes.
    pub resident_bytes: u64,
    /// Threads.
    pub threads: u64,
}

/// `USER_HZ`: the unit of the CPU times in `/proc/<pid>/stat`. Linux fixes it
/// at 100 on every architecture rvoip targets (it is the ABI value, not the
/// kernel tick rate).
const USER_HZ: f64 = 100.0;

/// Read this process's figures.
pub fn process_stats() -> ProcessStats {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    ProcessStats {
        cpu_seconds: parse_cpu_seconds(&stat).unwrap_or(0.0),
        resident_bytes: status_kib(&status, "VmRSS:").map_or(0, |kib| kib * 1024),
        threads: status_kib(&status, "Threads:").unwrap_or(0),
    }
}

/// `utime + stime` from `/proc/<pid>/stat`, in seconds. The command name
/// (field 2) may contain spaces, so fields are counted after its closing
/// parenthesis: utime and stime are fields 14 and 15.
fn parse_cpu_seconds(stat: &str) -> Option<f64> {
    let after_comm = &stat[stat.rfind(')')? + 1..];
    let mut fields = after_comm.split_whitespace();
    // Fields 3..=13 precede utime.
    let utime: u64 = fields.nth(11)?.parse().ok()?;
    let stime: u64 = fields.next()?.parse().ok()?;
    Some((utime + stime) as f64 / USER_HZ)
}

/// The first number after `key` in `/proc/<pid>/status`.
fn status_kib(status: &str, key: &str) -> Option<u64> {
    status
        .lines()
        .find_map(|line| line.strip_prefix(key))?
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

/// Established TCP connections whose local port is `port`, over IPv4 and
/// IPv6 (the kernel's own socket table, so a connection rvoip leaked or
/// closed is reflected exactly).
pub fn established_tcp_connections(port: u16) -> u64 {
    ["/proc/net/tcp", "/proc/net/tcp6"]
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .map(|table| count_established(&table, port))
        .sum()
}

/// Whether a TCP socket listens on `port` (IPv4 or IPv6). `None` when the
/// kernel tables cannot be read (not Linux).
pub fn tcp_listening(port: u16) -> Option<bool> {
    let tables: Vec<String> = ["/proc/net/tcp", "/proc/net/tcp6"]
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .collect();
    (!tables.is_empty()).then(|| {
        tables
            .iter()
            .any(|table| count_in_state(table, port, "0A") > 0)
    })
}

/// Count `/proc/net/tcp`-format rows in state `01` (ESTABLISHED) whose local
/// address has port `port`.
fn count_established(table: &str, port: u16) -> u64 {
    count_in_state(table, port, "01")
}

fn count_in_state(table: &str, port: u16, wanted: &str) -> u64 {
    table
        .lines()
        .skip(1)
        .filter(|line| {
            let mut fields = line.split_whitespace();
            let local = fields.nth(1);
            let state = fields.nth(1);
            matches!((local, state), (Some(local), Some(state))
                if state == wanted && local
                    .rsplit_once(':')
                    .and_then(|(_, hex)| u16::from_str_radix(hex, 16).ok())
                    == Some(port))
        })
        .count() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_seconds_skip_a_command_name_with_spaces_and_parentheses() {
        let stat = "4242 (rvoip (bench) uas) S 1 4242 4242 0 -1 4194560 1000 0 0 0 \
                    250 125 0 0 20 0 24 0 100 1000000 500 18446744073709551615";
        assert_eq!(parse_cpu_seconds(stat), Some(3.75));
        assert_eq!(parse_cpu_seconds("garbage"), None);
    }

    #[test]
    fn status_fields_parse() {
        let status = "Name:\trvoip-bench-uas\nVmRSS:\t  20480 kB\nThreads:\t17\n";
        assert_eq!(status_kib(status, "VmRSS:"), Some(20_480));
        assert_eq!(status_kib(status, "Threads:"), Some(17));
        assert_eq!(status_kib(status, "VmSwap:"), None);
    }

    #[test]
    fn established_rows_on_the_port_are_counted() {
        // 0x13C5 = 5061; state 01 established, 0A listen, 06 time-wait.
        let table = "  sl  local_address rem_address   st tx_queue rx_queue\n\
           0: 0A00000A:13C5 00000000:0000 0A 00000000:00000000 00:00000000\n\
           1: 0A00000A:13C5 0B00000A:D431 01 00000000:00000000 00:00000000\n\
           2: 0A00000A:13C5 0C00000A:D432 01 00000000:00000000 00:00000000\n\
           3: 0A00000A:13C5 0C00000A:D433 06 00000000:00000000 00:00000000\n\
           4: 0A00000A:13C4 0C00000A:D434 01 00000000:00000000 00:00000000\n";
        assert_eq!(count_established(table, 5061), 2);
        assert_eq!(count_established(table, 5060), 1);
        let v6 = "  sl  local_address remote_address st\n\
           0: 00000000000000000000000001000000:13C5 00000000000000000000000001000000:9C40 01 0\n";
        assert_eq!(count_established(v6, 5061), 1);
        assert_eq!(count_in_state(table, 5061, "0A"), 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn this_process_reports_itself() {
        let stats = process_stats();
        assert!(stats.resident_bytes > 0);
        assert!(stats.threads >= 1);
    }
}
