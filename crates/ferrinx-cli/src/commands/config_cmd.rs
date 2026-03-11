use crate::config::CliConfig;
use crate::error::Result;
use crate::output;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum ConfigCommands {
    Set { key: String, value: String },
    Show,
}

pub fn handle_config(cmd: ConfigCommands, config: &mut CliConfig) -> Result<()> {
    match cmd {
        ConfigCommands::Set { key, value } => {
            config.set(&key, &value)?;
            config.save()?;
            output::print_success(&format!("Configuration updated: {} = {}", key, value));
        }
        ConfigCommands::Show => {
            println!("Configuration file: {:?}", CliConfig::config_path()?);
            println!();
            println!("api_url: {}", config.api_url);
            println!(
                "api_key: {}",
                config
                    .api_key
                    .as_ref()
                    .map(|k| if k.len() > 8 {
                        format!("{}...", &k[..8])
                    } else {
                        "***".to_string()
                    })
                    .unwrap_or_else(|| "not set".to_string())
            );
            println!("timeout: {}s", config.timeout);
            println!("output_format: {}", config.output_format);
            println!("verify_ssl: {}", config.verify_ssl);
        }
    }

    Ok(())
}
