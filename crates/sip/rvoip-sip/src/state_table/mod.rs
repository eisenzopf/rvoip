pub mod builder;
pub mod types;
pub mod wiring_manifest;
pub mod yaml_loader;

pub use builder::StateTableBuilder;
pub use types::*;
pub use yaml_loader::YamlTableLoader;

use lazy_static::lazy_static;
use sha2::{Digest, Sha256};
use std::{fs, path::PathBuf, sync::Arc};

lazy_static! {
    /// The master state table - single source of truth for all transitions
    pub static ref MASTER_TABLE: Arc<MasterStateTable> = Arc::new(build_master_table());
}

/// Build the complete master state table
fn build_master_table() -> MasterStateTable {
    // Load embedded default YAML (this should always succeed)
    let table = YamlTableLoader::load_embedded_default()
        .expect("Embedded default state table must be valid");

    // Validate the table
    if let Err(errors) = table.validate() {
        panic!("Invalid default state table: {:?}", errors);
    }

    tracing::debug!("Using embedded default state table");
    table
}

/// Load state table with two-tier priority:
/// 1. Config path (if Some)
/// 2. Embedded default
pub fn load_state_table_with_config(config_path: Option<&str>) -> MasterStateTable {
    let StateTableSelection { table, metadata } =
        load_state_table_selection_with_config(config_path);
    drop(metadata);
    table
}

/// Bounded reason why a configured table did not become authoritative.
///
/// The category is safe to publish in diagnostics. Parser errors, validation
/// details, and configured paths can contain deployment information, so they
/// are deliberately not retained in the diagnostic projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StateTableFallbackReason {
    ReadFailed,
    DecodeFailed,
    LoadFailed,
    ValidationFailed,
}

impl StateTableFallbackReason {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::ReadFailed => "read-failed",
            Self::DecodeFailed => "decode-failed",
            Self::LoadFailed => "load-failed",
            Self::ValidationFailed => "validation-failed",
        }
    }
}

/// Internal provenance for the selected runtime state table.
///
/// Do not derive `Debug`: configured paths may contain tenant or credential
/// material. Use [`StateTableSourceMetadata::diagnostic`] for logging.
pub(crate) enum StateTableSelectedSource {
    EmbeddedDefault,
    ConfiguredPath {
        path: PathBuf,
    },
    ConfiguredPathFallback {
        configured_path: PathBuf,
        reason: StateTableFallbackReason,
    },
}

/// Exact selected YAML and its provenance. This remains crate-private so
/// `Config::state_table_path` and the public state-table API stay unchanged.
pub(crate) struct StateTableSourceMetadata {
    pub(crate) source: StateTableSelectedSource,
    pub(crate) selected_yaml: Arc<[u8]>,
    pub(crate) sha256: String,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) struct StateTableSelectionDiagnostic<'a> {
    pub(crate) selected_source: &'static str,
    pub(crate) fallback_reason: Option<&'static str>,
    pub(crate) sha256: &'a str,
}

impl StateTableSourceMetadata {
    /// Redacted fields used by the runtime diagnostic. The configured path and
    /// detailed loader error are intentionally absent.
    pub(crate) fn diagnostic(&self) -> StateTableSelectionDiagnostic<'_> {
        let (selected_source, fallback_reason) = match &self.source {
            StateTableSelectedSource::EmbeddedDefault => ("embedded-default", None),
            StateTableSelectedSource::ConfiguredPath { path } => {
                let _ = path;
                ("configured-path", None)
            }
            StateTableSelectedSource::ConfiguredPathFallback {
                configured_path,
                reason,
            } => {
                let _ = configured_path;
                ("configured-path-fallback", Some(reason.as_str()))
            }
        };
        let _ = &self.selected_yaml;
        StateTableSelectionDiagnostic {
            selected_source,
            fallback_reason,
            sha256: &self.sha256,
        }
    }
}

pub(crate) struct StateTableSelection {
    pub(crate) table: MasterStateTable,
    pub(crate) metadata: StateTableSourceMetadata,
}

fn yaml_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn configured_state_table(
    path: &str,
) -> Result<(MasterStateTable, Arc<[u8]>), StateTableFallbackReason> {
    let yaml = fs::read(path).map_err(|_| StateTableFallbackReason::ReadFailed)?;
    let yaml_text =
        std::str::from_utf8(&yaml).map_err(|_| StateTableFallbackReason::DecodeFailed)?;
    let mut loader = YamlTableLoader::new();
    loader
        .load_from_string(yaml_text)
        .map_err(|_| StateTableFallbackReason::LoadFailed)?;
    let table = loader
        .build()
        .map_err(|_| StateTableFallbackReason::LoadFailed)?;
    table
        .validate()
        .map_err(|_| StateTableFallbackReason::ValidationFailed)?;
    Ok((table, Arc::from(yaml.into_boxed_slice())))
}

fn embedded_state_table() -> (MasterStateTable, Arc<[u8]>) {
    let table = YamlTableLoader::load_embedded_default()
        .expect("Embedded default state table must be valid");
    if let Err(errors) = table.validate() {
        panic!("Invalid default state table: {:?}", errors);
    }
    (
        table,
        Arc::from(YamlTableLoader::embedded_default_yaml_bytes()),
    )
}

/// Load the table using the established two-tier policy while retaining exact
/// selected-source evidence for internal diagnostics and release reporting.
///
/// The compatibility contract is unchanged: every configured read, decode,
/// load, or table-validation failure falls back to the embedded default.
pub(crate) fn load_state_table_selection_with_config(
    config_path: Option<&str>,
) -> StateTableSelection {
    let (table, metadata) = match config_path {
        Some(path) => match configured_state_table(path) {
            Ok((table, selected_yaml)) => {
                let sha256 = yaml_sha256(&selected_yaml);
                (
                    table,
                    StateTableSourceMetadata {
                        source: StateTableSelectedSource::ConfiguredPath {
                            path: PathBuf::from(path),
                        },
                        selected_yaml,
                        sha256,
                    },
                )
            }
            Err(reason) => {
                let (table, selected_yaml) = embedded_state_table();
                let sha256 = yaml_sha256(&selected_yaml);
                (
                    table,
                    StateTableSourceMetadata {
                        source: StateTableSelectedSource::ConfiguredPathFallback {
                            configured_path: PathBuf::from(path),
                            reason,
                        },
                        selected_yaml,
                        sha256,
                    },
                )
            }
        },
        None => {
            let (table, selected_yaml) = embedded_state_table();
            let sha256 = yaml_sha256(&selected_yaml);
            (
                table,
                StateTableSourceMetadata {
                    source: StateTableSelectedSource::EmbeddedDefault,
                    selected_yaml,
                    sha256,
                },
            )
        }
    };

    let diagnostic = metadata.diagnostic();
    if let Some(fallback_reason) = diagnostic.fallback_reason {
        tracing::warn!(
            selected_state_table_source = diagnostic.selected_source,
            selected_state_table_fallback_reason = fallback_reason,
            selected_state_table_sha256 = diagnostic.sha256,
            "Configured state table was not selected; using the embedded default"
        );
    } else {
        tracing::info!(
            selected_state_table_source = diagnostic.selected_source,
            selected_state_table_sha256 = diagnostic.sha256,
            "Selected runtime state table"
        );
    }

    StateTableSelection { table, metadata }
}

#[cfg(test)]
mod selected_source_tests {
    use super::*;
    use std::{fs, path::Path};
    use tempfile::TempDir;

    fn write_yaml(root: &Path, name: &str, yaml: &[u8]) -> PathBuf {
        let path = root.join(name);
        fs::write(&path, yaml).expect("write YAML fixture");
        path
    }

    fn assert_embedded(selection: &StateTableSelection) {
        assert_eq!(
            selection.metadata.selected_yaml.as_ref(),
            YamlTableLoader::embedded_default_yaml_bytes()
        );
        assert_eq!(
            selection.metadata.sha256,
            yaml_sha256(YamlTableLoader::embedded_default_yaml_bytes())
        );
    }

    #[test]
    fn none_selects_exact_embedded_yaml_and_hash() {
        let selection = load_state_table_selection_with_config(None);
        assert!(matches!(
            &selection.metadata.source,
            StateTableSelectedSource::EmbeddedDefault
        ));
        assert_embedded(&selection);
    }

    #[test]
    fn configured_valid_extension_selects_exact_file_bytes_and_hash() {
        let root = TempDir::new().expect("temp dir");
        let yaml = br#"version: "2.0"
metadata:
  description: "valid external extension"
states:
  - name: "Idle"
transitions:
  - role: "UAC"
    state: "Idle"
    event:
      type: "MakeCall"
    next_state: "Idle"
    actions: []
"#;
        let path = write_yaml(root.path(), "extension.yaml", yaml);

        let selection = load_state_table_selection_with_config(path.to_str());
        match &selection.metadata.source {
            StateTableSelectedSource::ConfiguredPath { path: selected } => {
                assert_eq!(selected, &path)
            }
            _ => panic!("valid configured YAML did not become authoritative"),
        }
        assert_eq!(selection.metadata.selected_yaml.as_ref(), yaml);
        assert_eq!(selection.metadata.sha256, yaml_sha256(yaml));
        assert_eq!(selection.table.transition_count(), 1);
    }

    #[test]
    fn missing_configured_file_falls_back_with_bounded_reason() {
        let root = TempDir::new().expect("temp dir");
        let path = root.path().join("missing-secret-tenant.yaml");
        let selection = load_state_table_selection_with_config(path.to_str());
        assert!(matches!(
            &selection.metadata.source,
            StateTableSelectedSource::ConfiguredPathFallback {
                reason: StateTableFallbackReason::ReadFailed,
                ..
            }
        ));
        assert_embedded(&selection);
    }

    #[test]
    fn malformed_and_unknown_yaml_fall_back_without_parser_details() {
        let root = TempDir::new().expect("temp dir");
        let malformed = write_yaml(root.path(), "malformed.yaml", b"version: [unterminated");
        let unknown_yaml =
            String::from_utf8(YamlTableLoader::embedded_default_yaml_bytes().to_vec())
                .expect("embedded YAML is UTF-8")
                .replacen("type: \"MakeCall\"", "type: \"TenantSecretEvent\"", 1);
        let unknown = write_yaml(root.path(), "unknown.yaml", unknown_yaml.as_bytes());

        for path in [malformed, unknown] {
            let selection = load_state_table_selection_with_config(path.to_str());
            assert!(matches!(
                &selection.metadata.source,
                StateTableSelectedSource::ConfiguredPathFallback {
                    reason: StateTableFallbackReason::LoadFailed,
                    ..
                }
            ));
            assert_embedded(&selection);
        }
    }

    #[test]
    fn non_utf8_yaml_falls_back_with_bounded_decode_reason() {
        let root = TempDir::new().expect("temp dir");
        let path = write_yaml(root.path(), "invalid-utf8.yaml", &[0xff, 0xfe, 0xfd]);
        let selection = load_state_table_selection_with_config(path.to_str());
        assert!(matches!(
            &selection.metadata.source,
            StateTableSelectedSource::ConfiguredPathFallback {
                reason: StateTableFallbackReason::DecodeFailed,
                ..
            }
        ));
        assert_embedded(&selection);
    }

    #[test]
    fn incomplete_lifecycle_falls_back_after_table_validation() {
        let root = TempDir::new().expect("temp dir");
        let yaml = br#"version: "2.0"
states:
  - name: "Idle"
  - name: "Active"
transitions:
  - role: "UAC"
    state: "Idle"
    event:
      type: "MakeCall"
    next_state: "Active"
    actions: []
"#;
        let path = write_yaml(root.path(), "incomplete.yaml", yaml);
        let selection = load_state_table_selection_with_config(path.to_str());
        assert!(matches!(
            &selection.metadata.source,
            StateTableSelectedSource::ConfiguredPathFallback {
                reason: StateTableFallbackReason::ValidationFailed,
                ..
            }
        ));
        assert_embedded(&selection);
    }

    #[test]
    fn runtime_diagnostic_never_contains_configured_path_or_loader_detail() {
        let root = TempDir::new().expect("temp dir");
        let secret = "tenant-password-super-secret";
        let path = root.path().join(format!("{secret}.yaml"));
        let selection = load_state_table_selection_with_config(path.to_str());
        let rendered = format!("{:?}", selection.metadata.diagnostic());

        assert!(!rendered.contains(secret));
        assert!(!rendered.contains(&root.path().display().to_string()));
        assert!(!rendered.contains("No such file"));
        assert!(rendered.contains("read-failed"));
    }
}
