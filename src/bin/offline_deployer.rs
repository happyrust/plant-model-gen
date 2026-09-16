use std::{path::PathBuf, process::Command, thread, time::Duration};

use aios_database::web_server::start_web_server_with_config;
use anyhow::{Context, Result};
use clap::{Arg, Command as ClapCommand};

fn open_browser(url: String) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(900));
        #[cfg(target_os = "windows")]
        let result = Command::new("cmd").args(["/C", "start", "", &url]).spawn();

        #[cfg(target_os = "macos")]
        let result = Command::new("open").arg(&url).spawn();

        #[cfg(all(unix, not(target_os = "macos")))]
        let result = Command::new("xdg-open").arg(&url).spawn();

        if let Err(err) = result {
            eprintln!("[offline_deployer] failed to open browser: {err}");
        }
    });
}

#[tokio::main]
async fn main() -> Result<()> {
    let matches = ClapCommand::new("offline_deployer")
        .about("Start the Plant3D offline deployment wizard")
        .arg(
            Arg::new("port")
                .long("port")
                .value_name("PORT")
                .default_value("3100")
                .help("Local HTTP port for the wizard"),
        )
        .arg(
            Arg::new("config")
                .long("config")
                .value_name("DB_OPTION")
                .default_value("db_options/DbOption")
                .help("DbOption path without .toml, same as web_server"),
        )
        .arg(
            Arg::new("no-open")
                .long("no-open")
                .action(clap::ArgAction::SetTrue)
                .help("Do not open the browser automatically"),
        )
        .get_matches();

    let port = matches
        .get_one::<String>("port")
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(3100);
    let config_path = matches
        .get_one::<String>("config")
        .map(PathBuf::from)
        .context("missing --config")?;
    let config_path_string = config_path.to_string_lossy().to_string();
    let url = format!("http://127.0.0.1:{port}/admin/#/offline-deploy");

    println!("[offline_deployer] starting local wizard server on {url}");
    println!("[offline_deployer] config: {}", config_path.display());
    unsafe {
        std::env::set_var("DB_OPTION_FILE", &config_path_string);
    }
    if !matches.get_flag("no-open") {
        open_browser(url);
    }

    start_web_server_with_config(port, Some(&config_path_string)).await
}
