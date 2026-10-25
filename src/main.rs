use std::path::PathBuf;

use clap::Parser;
use tracing::{info, warn};

use websocket2serial::cli::Cli;
use websocket2serial::config::Config;
use websocket2serial::{logging, paths};

fn main() {
    let cli = Cli::parse();

    let config_path: PathBuf = cli
        .config
        .clone()
        .unwrap_or_else(|| paths::config_dir().join("config.toml"));

    let (config, config_err) = Config::load_or_default(&config_path);

    // 初始化日志；guard 需持有到进程退出，避免日志线程提前终止。
    let _guard = match logging::init(&paths::log_dir(), &config.logging) {
        Ok(guard) => guard,
        Err(err) => {
            eprintln!("日志初始化失败：{err}");
            std::process::exit(1);
        }
    };

    if let Some(err) = config_err {
        warn!("配置解析失败，已回退到默认值：{err}");
    }

    if cli.force_admin {
        info!("--admin：本次启动强制开启管理页");
    }

    info!(
        "websocket2serial v{} 启动（骨架：尚未启动服务器；配置 {}）",
        env!("CARGO_PKG_VERSION"),
        config_path.display()
    );

    // TODO(后续任务)：单实例、绑定管理 WS、HTTP 管理页、托盘、热插拔。
}
