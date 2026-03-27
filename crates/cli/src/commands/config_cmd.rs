use crate::config::{load_config, save_config, CliConfig};
use crate::output::{print_error, print_success};

pub fn execute(cmd: crate::ConfigCmd) -> anyhow::Result<()> {
    match cmd {
        crate::ConfigCmd::Show => {
            let cfg = load_config();
            let content = toml::to_string_pretty(&cfg).unwrap_or_default();
            println!("{}", content);
        }
        crate::ConfigCmd::Set { key, value } => {
            let mut cfg = load_config();
            match key.as_str() {
                "server.url" => cfg.server.url = value.clone(),
                "output.format" => cfg.output.format = value.clone(),
                "auth.token" => cfg.auth.token = Some(value.clone()),
                "auth.username" => cfg.auth.username = Some(value.clone()),
                other => {
                    print_error(&format!("Unknown config key: {}", other));
                    return Ok(());
                }
            }
            save_config(&cfg)?;
            print_success(&format!("Set {} = {}", key, value));
        }
        crate::ConfigCmd::Export => {
            let cfg = load_config();
            let json = serde_json::to_string_pretty(&cfg).unwrap_or_default();
            println!("{}", json);
        }
        crate::ConfigCmd::Import { file } => {
            let content = std::fs::read_to_string(&file)?;
            let cfg: CliConfig = if file.ends_with(".json") {
                serde_json::from_str(&content)?
            } else {
                toml::from_str(&content)?
            };
            save_config(&cfg)?;
            print_success("Config imported");
        }
    }

    Ok(())
}
