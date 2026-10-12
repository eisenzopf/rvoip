//! Behavior pins for `ConfigLoader`: file loading, the base → environment
//! file → environment variable precedence, prefix/separator handling, and
//! which stage an error is attributed to.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rvoip_infra_common::config::ConfigLoader;
use rvoip_infra_common::errors::types::Error;
use serde::Deserialize;

// The loader reads the process environment, so every test that touches it
// runs under this lock.
static ENV_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Deserialize, PartialEq)]
struct Server {
    host: String,
    port: u16,
    #[serde(default)]
    tls: bool,
}

#[derive(Debug, Deserialize, PartialEq)]
struct AppConfig {
    name: String,
    server: Server,
    #[serde(default)]
    tags: Vec<String>,
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(test: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "rvoip-infra-common-config-loader-{}-{}",
            test,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }

    fn write(&self, name: &str, contents: &str) {
        std::fs::write(self.0.join(name), contents).expect("write config file");
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct EnvVars(Vec<&'static str>);

impl EnvVars {
    fn set(vars: &[(&'static str, &str)]) -> Self {
        for (key, value) in vars {
            std::env::set_var(key, value);
        }
        EnvVars(vars.iter().map(|(key, _)| *key).collect())
    }
}

impl Drop for EnvVars {
    fn drop(&mut self) {
        for key in &self.0 {
            std::env::remove_var(key);
        }
    }
}

fn config_message(error: Error) -> String {
    match error {
        Error::Config(message) => message,
        other => panic!("expected Error::Config, got {other:?}"),
    }
}

const BASE: &str = r#"
name = "base"
tags = ["a", "b"]

[server]
host = "0.0.0.0"
port = 5060
"#;

#[test]
fn load_from_file_reads_nested_toml() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new("from-file");
    dir.write("app.toml", BASE);

    let loader = ConfigLoader::new(dir.path(), "dev");
    let provider = loader.load_from_file::<AppConfig, _>("app.toml").unwrap();

    assert_eq!(
        *provider.config(),
        AppConfig {
            name: "base".into(),
            server: Server {
                host: "0.0.0.0".into(),
                port: 5060,
                tls: false,
            },
            tags: vec!["a".into(), "b".into()],
        }
    );
}

#[test]
fn load_from_file_ignores_environment_variables() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new("from-file-no-env");
    dir.write("app.toml", BASE);
    let _env = EnvVars::set(&[("APP__SERVER__PORT", "9999")]);

    let loader = ConfigLoader::new(dir.path(), "dev");
    let provider = loader.load_from_file::<AppConfig, _>("app.toml").unwrap();
    assert_eq!(provider.config().server.port, 5060);
}

#[test]
fn load_from_file_reports_missing_and_malformed_files() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new("from-file-errors");
    dir.write("broken.toml", "name = \"unterminated\n[server\n");
    let loader = ConfigLoader::new(dir.path(), "dev");

    let missing = config_message(
        loader
            .load_from_file::<AppConfig, _>("absent.toml")
            .unwrap_err(),
    );
    assert!(missing.starts_with("Config file not found"), "{missing}");

    let malformed = config_message(
        loader
            .load_from_file::<AppConfig, _>("broken.toml")
            .unwrap_err(),
    );
    assert!(
        malformed.starts_with("Failed to load config file"),
        "{malformed}"
    );
}

#[test]
fn load_with_env_layers_base_env_file_and_env_vars() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new("with-env-layers");
    dir.write("layered.toml", BASE);
    dir.write(
        "layered.prod.toml",
        r#"
name = "prod"

[server]
port = 5061
"#,
    );
    // The prefix is the upper-cased name; the prefix separator follows the
    // `__` key separator.
    let _env = EnvVars::set(&[
        ("LAYERED__SERVER__HOST", "10.0.0.1"),
        ("LAYERED__SERVER__TLS", "true"),
        ("LAYERED_SERVER__PORT", "1"),
    ]);

    let loader = ConfigLoader::new(dir.path(), "prod");
    let provider = loader.load_with_env::<AppConfig>("layered").unwrap();

    assert_eq!(
        *provider.config(),
        AppConfig {
            // env file overrides base
            name: "prod".into(),
            server: Server {
                // env var overrides both files
                host: "10.0.0.1".into(),
                // env file overrides base; the single-underscore variable
                // does not match the `LAYERED__` prefix
                port: 5061,
                tls: true,
            },
            // untouched base value survives the overlays
            tags: vec!["a".into(), "b".into()],
        }
    );
}

#[test]
fn load_with_env_env_vars_override_env_file() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new("with-env-precedence");
    dir.write("prec.toml", BASE);
    dir.write("prec.staging.toml", "[server]\nport = 5061\n");
    let _env = EnvVars::set(&[("PREC__SERVER__PORT", "7000")]);

    let loader = ConfigLoader::new(dir.path(), "staging");
    let provider = loader.load_with_env::<AppConfig>("prec").unwrap();
    assert_eq!(provider.config().server.port, 7000);
    assert_eq!(provider.config().name, "base");
}

#[test]
fn load_with_env_skips_absent_files() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new("with-env-no-files");
    let loader = ConfigLoader::new(dir.path(), "dev");

    // Env vars alone can satisfy the struct.
    {
        let _env = EnvVars::set(&[
            ("ENVONLY__NAME", "from-env"),
            ("ENVONLY__SERVER__HOST", "127.0.0.1"),
            ("ENVONLY__SERVER__PORT", "5080"),
        ]);
        let provider = loader.load_with_env::<AppConfig>("envonly").unwrap();
        assert_eq!(provider.config().name, "from-env");
        assert_eq!(provider.config().server.port, 5080);
        assert!(provider.config().tags.is_empty());
    }

    // Nothing at all is a deserialize error, not a load error.
    let message = config_message(loader.load_with_env::<AppConfig>("envonly").unwrap_err());
    assert!(
        message.starts_with("Failed to deserialize config"),
        "{message}"
    );
}

#[test]
fn load_with_env_attributes_errors_to_the_failing_stage() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let broken = "name = \"unterminated\n[server\n";

    let dir = TempDir::new("with-env-bad-base");
    dir.write("svc.toml", broken);
    dir.write("svc.dev.toml", "name = \"dev\"\n");
    let message = config_message(
        ConfigLoader::new(dir.path(), "dev")
            .load_with_env::<AppConfig>("svc")
            .unwrap_err(),
    );
    assert!(
        message.starts_with("Failed to load base config"),
        "{message}"
    );

    let dir = TempDir::new("with-env-bad-env");
    dir.write("svc.toml", BASE);
    dir.write("svc.dev.toml", broken);
    let message = config_message(
        ConfigLoader::new(dir.path(), "dev")
            .load_with_env::<AppConfig>("svc")
            .unwrap_err(),
    );
    assert!(
        message.starts_with("Failed to load env config"),
        "{message}"
    );
}

#[test]
fn load_from_env_uses_uppercased_prefix_and_parses_scalars() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = TempDir::new("from-env");
    let _env = EnvVars::set(&[
        ("FROMENV__NAME", "env-app"),
        ("FROMENV__SERVER__HOST", "::1"),
        ("FROMENV__SERVER__PORT", "5090"),
        ("FROMENV__SERVER__TLS", "TRUE"),
    ]);

    let loader = ConfigLoader::new(dir.path(), "dev");
    let provider = loader.load_from_env::<AppConfig>("fromenv").unwrap();
    assert_eq!(
        *provider.config(),
        AppConfig {
            name: "env-app".into(),
            server: Server {
                host: "::1".into(),
                port: 5090,
                tls: true,
            },
            tags: vec![],
        }
    );

    let message = config_message(
        loader
            .load_from_env::<AppConfig>("unsetprefix")
            .unwrap_err(),
    );
    assert!(
        message.starts_with("Failed to deserialize env config"),
        "{message}"
    );
}
