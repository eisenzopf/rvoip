use std::io::{self, Write};

use crate::api_client::ApiClient;
use crate::config::{save_config, CliConfig};
use crate::output::{print_error, print_success};

pub async fn execute(api: &ApiClient, config: &mut CliConfig) -> anyhow::Result<()> {
    print!("Username: ");
    io::stdout().flush()?;
    let mut username = String::new();
    io::stdin().read_line(&mut username)?;
    let username = username.trim().to_string();

    print!("Password: ");
    io::stdout().flush()?;
    let mut password = String::new();
    io::stdin().read_line(&mut password)?;
    let password = password.trim().to_string();

    match api.login(&username, &password).await {
        Ok(resp) => {
            let token = resp["access_token"]
                .as_str()
                .unwrap_or_default()
                .to_string();

            if token.is_empty() {
                print_error("No access token in response");
                return Ok(());
            }

            config.auth.token = Some(token);
            config.auth.username = Some(username.clone());
            save_config(config)?;

            print_success(&format!("Logged in as {}", username));
        }
        Err(e) => {
            print_error(&format!("Login failed: {}", e));
        }
    }

    Ok(())
}
