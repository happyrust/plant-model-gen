use aios_database::web_server::start_web_server_with_config;
use clap::{Arg, Command};
use std::process::Stdio;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let matches = Command::new("aios-web-server")
        .version("0.1.4")
        .about("AIOS Web UI Server")
        .arg(
            Arg::new("config")
                .long("config")
                .short('c')
                .help("Path to the configuration file (Without extension)")
                .value_name("CONFIG_PATH")
                .default_value(if cfg!(target_os = "macos") {
                    "db_options/DbOption-mac"
                } else {
                    "db_options/DbOption"
                }),
        )
        .get_matches();

    // 获取配置文件路径
    let config_path = matches
        .get_one::<String>("config")
        .expect("default value ensures this exists");

    // 设置环境变量，让 rs-core 库使用正确的配置文件
    unsafe {
        std::env::set_var("DB_OPTION_FILE", config_path);
    }

    // 初始化日志，设置更详细的日志级别
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();

    // 读取配置文件
    let config_file = format!("{}.toml", config_path);
    let db_option: aios_core::options::DbOption = {
        let content = std::fs::read_to_string(&config_file)
            .unwrap_or_else(|e| panic!("❌ 无法读取配置文件 {}: {}", config_file, e));
        toml::from_str(&content)
            .unwrap_or_else(|e| panic!("❌ 配置文件解析失败 {}: {}", config_file, e))
    };

    let ws_cfg = &db_option.web_server;
    let port = ws_cfg.port;

    // 自启动 SurrealDB
    let _surreal_child = if ws_cfg.auto_start_surreal {
        let data_path = ws_cfg.effective_data_path(db_option.surrealdb.path.as_deref());
        println!("🗄️  自启动 SurrealDB...");
        println!("   - 可执行文件: {}", ws_cfg.surreal_bin);
        println!("   - 数据路径: rocksdb://{}", data_path);
        println!("   - 监听地址: {}", ws_cfg.surreal_bind);
        println!("   - 用户: {}", ws_cfg.surreal_user);

        let child = std::process::Command::new(&ws_cfg.surreal_bin)
            .arg("start")
            .arg("--bind")
            .arg(&ws_cfg.surreal_bind)
            .arg("--user")
            .arg(&ws_cfg.surreal_user)
            .arg("--pass")
            .arg(&ws_cfg.surreal_password)
            .arg(format!("rocksdb://{}", data_path))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        match child {
            Ok(child) => {
                println!("✅ SurrealDB 进程已启动 (PID: {})", child.id());
                // 覆盖 surrealdb 连接模式为 ws，避免 rocksdb 文件锁冲突
                let (bind_ip, bind_port) = ws_cfg
                    .surreal_bind
                    .split_once(':')
                    .unwrap_or(("0.0.0.0", "8020"));
                let conn_ip = if bind_ip == "0.0.0.0" {
                    "127.0.0.1"
                } else {
                    bind_ip
                };
                unsafe {
                    std::env::set_var("SURREAL_CONN_MODE", "ws");
                    std::env::set_var("SURREAL_CONN_IP", conn_ip);
                    std::env::set_var("SURREAL_CONN_PORT", bind_port);
                }
                // 等待 SurrealDB 就绪
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                Some(child)
            }
            Err(e) => {
                eprintln!("❌ 无法启动 SurrealDB: {}", e);
                eprintln!(
                    "   请确认 '{}' 在 PATH 中或配置 surreal_bin 为完整路径",
                    ws_cfg.surreal_bin
                );
                return Err(e.into());
            }
        }
    } else {
        println!("⏭️  跳过 SurrealDB 自启动（auto_start_surreal = false）");
        None
    };

    println!("🚀 正在启动 AIOS Web UI 服务器...");
    println!("📱 访问地址: http://localhost:{}", port);
    println!("⚙️  使用配置文件: {}", config_file);

    start_web_server_with_config(port, Some(config_path)).await?;

    Ok(())
}
