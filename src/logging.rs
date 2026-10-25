//! tracing 初始化：按天轮转的文件日志 + stderr 双输出。

use std::path::Path;

use tracing::level_filters::LevelFilter;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

use crate::config::LoggingConfig;

/// 初始化日志。
///
/// - 文件日志按天轮转，保留 `config.keep_days` 天（`max_log_files`）。
/// - 同时输出到 stderr。
/// - 返回的 [`WorkerGuard`] 必须由调用方持有，直到进程退出，否则日志线程会提前终止。
pub fn init(
    log_dir: &Path,
    config: &LoggingConfig,
) -> Result<WorkerGuard, Box<dyn std::error::Error + Send + Sync>> {
    std::fs::create_dir_all(log_dir)?;

    let file_appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("websocket2serial")
        .filename_suffix("log")
        .max_log_files(config.keep_days)
        .build(log_dir)?;

    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let level_filter = config
        .level
        .parse::<LevelFilter>()
        .unwrap_or(LevelFilter::INFO);

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(level_filter);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_filter(level_filter);

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .try_init()?;

    Ok(guard)
}
