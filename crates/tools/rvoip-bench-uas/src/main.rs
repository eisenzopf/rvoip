//! `rvoip-bench-uas --config <dut.json>`

use std::path::PathBuf;
use std::process::ExitCode;

use rvoip_bench_uas::config::DutConfig;
use rvoip_bench_uas::dut::{self, EXIT_CONFIG, EXIT_ERROR};

const USAGE: &str = "usage: rvoip-bench-uas --config <dut.json>";

fn config_path() -> Result<PathBuf, String> {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" | "-c" => {
                path = Some(PathBuf::from(
                    args.next().ok_or("--config needs a path".to_string())?,
                ));
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("rvoip-bench-uas {} ({})", dut::VERSION, dut::GIT_SHA);
                std::process::exit(0);
            }
            other => match other.strip_prefix("--config=") {
                Some(value) => path = Some(PathBuf::from(value)),
                None => return Err(format!("unexpected argument {other:?}")),
            },
        }
    }
    path.ok_or_else(|| "--config is required".to_string())
}

fn exit(code: i32) -> ExitCode {
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn main() -> ExitCode {
    let path = match config_path() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("rvoip-bench-uas: {error}\n{USAGE}");
            return exit(EXIT_CONFIG);
        }
    };
    let loaded = match DutConfig::load(&path) {
        Ok(loaded) => loaded,
        Err(error) => {
            eprintln!("rvoip-bench-uas: {error}");
            return exit(EXIT_CONFIG);
        }
    };
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,rvoip_bench_uas=info".into()),
        )
        .with_writer(std::io::stderr)
        .try_init();

    let cpus = std::thread::available_parallelism().map_or(1, usize::from);
    let (media_lanes, workers) = loaded.config.thread_plan(cpus);
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .thread_name("bench-uas-sip")
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("rvoip-bench-uas: cannot start the runtime: {error}");
            return exit(EXIT_ERROR);
        }
    };
    let code = match runtime.block_on(dut::run(loaded, media_lanes)) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("rvoip-bench-uas: {error}");
            EXIT_ERROR
        }
    };
    // Media lanes and the SIP stack's background tasks are not joined: the
    // run files are written and flushed, so exit at once.
    std::process::exit(code)
}
