use std::fmt::Debug;
use std::path::{Path, PathBuf};

use crate::config::provider::{BasicConfigProvider, ConfigSource};
use crate::errors::types::{Error, Result};
use config::builder::DefaultState;
use config::{Config, ConfigBuilder, Environment, File, Source};
use serde::de::DeserializeOwned;

/// Handles loading configuration from various sources
#[derive(Debug)]
pub struct ConfigLoader {
    base_path: PathBuf,
    environment: String,
}

impl ConfigLoader {
    /// Create a new configuration loader
    pub fn new<P: AsRef<Path>>(base_path: P, environment: &str) -> Self {
        ConfigLoader {
            base_path: base_path.as_ref().to_path_buf(),
            environment: environment.to_string(),
        }
    }

    /// Load configuration from a file
    pub fn load_from_file<T, P>(&self, file_path: P) -> Result<BasicConfigProvider<T>>
    where
        T: DeserializeOwned + Send + Sync + Debug + 'static,
        P: AsRef<Path>,
    {
        let full_path = self.resolve_path(file_path);

        if !full_path.exists() {
            return Err(Error::Config(format!(
                "Config file not found: {:?}",
                full_path
            )));
        }

        let config = self.build_config(Some(&full_path))?;
        let typed_config: T = config
            .try_deserialize()
            .map_err(|e| Error::Config(format!("Failed to deserialize config: {}", e)))?;

        Ok(BasicConfigProvider::new(
            full_path.to_string_lossy().to_string(),
            ConfigSource::File,
            typed_config,
        ))
    }

    /// Load configuration with environment overrides
    pub fn load_with_env<T>(&self, name: &str) -> Result<BasicConfigProvider<T>>
    where
        T: DeserializeOwned + Send + Sync + Debug + 'static,
    {
        let base_file = format!("{}.toml", name);
        let env_file = format!("{}.{}.toml", name, self.environment);

        let base_path = self.resolve_path(&base_file);
        let env_path = self.resolve_path(&env_file);

        let mut builder = Config::builder();

        // Load base config if it exists
        if base_path.exists() {
            builder = add_checked(builder, File::from(base_path), "Failed to load base config")?;
        }

        // Load environment-specific config if it exists
        if env_path.exists() {
            builder = add_checked(builder, File::from(env_path), "Failed to load env config")?;
        }

        // Add environment variable overrides. The file layers above were
        // already collected, so a failure here belongs to this stage.
        let env_prefix = name.to_uppercase();
        let config = builder
            .add_source(Environment::with_prefix(&env_prefix).separator("__"))
            .build()
            .map_err(|e| Error::Config(format!("Failed to add env overrides: {}", e)))?;

        let typed_config: T = config
            .try_deserialize()
            .map_err(|e| Error::Config(format!("Failed to deserialize config: {}", e)))?;

        Ok(BasicConfigProvider::new(
            name.to_string(),
            ConfigSource::File,
            typed_config,
        ))
    }

    /// Load configuration from environment variables only
    pub fn load_from_env<T>(&self, prefix: &str) -> Result<BasicConfigProvider<T>>
    where
        T: DeserializeOwned + Send + Sync + Debug + 'static,
    {
        // Add environment variable configuration
        let env_prefix = prefix.to_uppercase();
        let config = Config::builder()
            .add_source(Environment::with_prefix(&env_prefix).separator("__"))
            .build()
            .map_err(|e| Error::Config(format!("Failed to load env config: {}", e)))?;

        let typed_config: T = config
            .try_deserialize()
            .map_err(|e| Error::Config(format!("Failed to deserialize env config: {}", e)))?;

        Ok(BasicConfigProvider::new(
            format!("{}_env", prefix),
            ConfigSource::Environment,
            typed_config,
        ))
    }

    // Helper to build a Config object from a path
    fn build_config(&self, path: Option<&PathBuf>) -> Result<Config> {
        let mut builder = Config::builder();

        if let Some(p) = path {
            builder = builder.add_source(File::from(p.clone()));
        }

        builder
            .build()
            .map_err(|e| Error::Config(format!("Failed to load config file: {}", e)))
    }

    // Helper to resolve a path relative to the base path
    fn resolve_path<P: AsRef<Path>>(&self, path: P) -> PathBuf {
        self.base_path.join(path)
    }
}

/// Add `source` to `builder` and collect the layers so far, so a failure is
/// reported against the source that caused it.
///
/// This keeps the per-stage error attribution of the `Config::merge` calls
/// the loader used before config 0.15 removed them: each `merge` collected
/// every source added so far and failed on the first one that did not load.
fn add_checked<S>(
    builder: ConfigBuilder<DefaultState>,
    source: S,
    context: &str,
) -> Result<ConfigBuilder<DefaultState>>
where
    S: Source + Send + Sync + 'static,
{
    let builder = builder.add_source(source);
    builder
        .build_cloned()
        .map_err(|e| Error::Config(format!("{}: {}", context, e)))?;
    Ok(builder)
}
