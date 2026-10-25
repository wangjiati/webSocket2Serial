//! 配置结构、默认值与 Origin 白名单。

use std::path::Path;

use serde::{Deserialize, Serialize};

/// 顶层配置。解析失败或文件缺失时回退到 [`Config::default`]。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub server: ServerConfig,
    pub admin: AdminConfig,
    pub session: SessionConfig,
    pub security: SecurityConfig,
    pub logging: LoggingConfig,
}

impl Config {
    /// 从 `path` 加载配置。
    ///
    /// - 文件缺失：返回默认值，`Err` 侧为 `None`（不视为错误）。
    /// - 解析失败（含未知字段）：返回默认值 + 错误字符串，保证仍能启动进管理页修复。
    pub fn load_or_default(path: &Path) -> (Config, Option<String>) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(_) => return (Config::default(), None),
        };
        match toml::from_str::<Config>(&text) {
            Ok(cfg) => (cfg, None),
            Err(err) => (Config::default(), Some(err.to_string())),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    pub management_port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            management_port: 8765,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AdminConfig {
    pub enabled: bool,
    pub port: u16,
}

impl Default for AdminConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: 8766,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SessionConfig {
    pub connect_timeout_ms: u64,
    pub grace_ms: u64,
    pub max_sessions: usize,
    pub read_timeout_ms: u64,
    pub write_timeout_ms: u64,
    pub rx_queue_bytes: usize,
    pub tx_queue_bytes: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            connect_timeout_ms: 5000,
            grace_ms: 10000,
            max_sessions: 8,
            read_timeout_ms: 5,
            write_timeout_ms: 5000,
            rx_queue_bytes: 1_048_576,
            tx_queue_bytes: 1_048_576,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SecurityConfig {
    pub allowed_origins: Vec<String>,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            allowed_origins: vec![
                "http://127.0.0.1:*".to_string(),
                "http://localhost:*".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LoggingConfig {
    pub level: String,
    pub keep_days: usize,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            keep_days: 7,
        }
    }
}

/// Origin 白名单：支持精确匹配与 `prefix:*` 端口通配。
///
/// - 单独 `*` 被拒绝（构造时忽略）。
/// - `None`（如 `file://` 页面的 `Origin: null`）一律拒绝。
#[derive(Debug, Clone)]
pub struct OriginAllowlist {
    entries: Vec<String>,
}

impl OriginAllowlist {
    /// 从条目构造；忽略非法的单独 `*`。
    pub fn new(entries: &[String]) -> Self {
        Self {
            entries: entries
                .iter()
                .filter(|entry| entry.as_str() != "*")
                .cloned()
                .collect(),
        }
    }

    /// 判断给定 Origin 是否被允许。
    pub fn allows(&self, origin: Option<&str>) -> bool {
        let origin = match origin {
            Some(origin) => origin,
            None => return false,
        };
        self.entries.iter().any(|entry| {
            if let Some(prefix) = entry.strip_suffix(":*") {
                // 端口通配：`http://127.0.0.1:*` 匹配 `http://127.0.0.1:<任意端口>`。
                origin.starts_with(prefix)
            } else {
                origin == entry
            }
        })
    }
}
