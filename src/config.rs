//! 配置结构、默认值与 Origin 白名单。

use std::path::Path;

use serde::{Deserialize, Serialize};
use url::Url;

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

/// Origin 白名单：支持精确匹配与 `scheme://host:*` 端口通配。
///
/// 匹配规则：
/// - scheme 与 host 大小写不敏感、精确相等；host 不允许后缀匹配。
/// - `http://127.0.0.1:*` / `http://localhost:*` 只匹配「带显式端口」的 loopback Origin。
/// - 精确条目按原样匹配（可含显式端口）。
/// - 单独 `*` 被拒绝（构造时忽略）；`None`（如 `file://` 页面的 `Origin: null`）一律拒绝。
#[derive(Debug, Clone)]
pub struct OriginAllowlist {
    rules: Vec<OriginRule>,
}

impl OriginAllowlist {
    /// 从条目构造；忽略非法条目（含单独 `*` 与解析失败的值）。
    pub fn new(entries: &[String]) -> Self {
        Self {
            rules: entries
                .iter()
                .filter_map(|entry| OriginRule::parse(entry))
                .collect(),
        }
    }

    /// 判断给定 Origin 是否被允许。
    pub fn allows(&self, origin: Option<&str>) -> bool {
        let origin = match origin {
            Some(origin) => origin,
            None => return false,
        };
        let origin = match Url::parse(origin) {
            Ok(url) => url,
            Err(_) => return false,
        };
        self.rules.iter().any(|rule| rule.matches(&origin))
    }
}

/// 单条 Origin 匹配规则。
#[derive(Debug, Clone)]
enum OriginRule {
    /// 精确匹配：scheme、host 精确相等，端口精确（`None` 表示无端口）。
    Exact {
        scheme: String,
        host: String,
        port: Option<u16>,
    },
    /// 端口通配：scheme、host 精确相等，Origin 必须带任意显式端口。
    AnyPort { scheme: String, host: String },
}

impl OriginRule {
    /// 解析一条白名单条目。返回 `None` 表示非法条目（单独 `*`、无 host、通配基址带端口等）。
    fn parse(entry: &str) -> Option<Self> {
        if entry == "*" {
            return None;
        }
        if let Some(base) = entry.strip_suffix(":*") {
            // 端口通配：基址必须不含端口（`http://127.0.0.1`），不允许 `http://h:80:*`。
            let url = Url::parse(base).ok()?;
            if url.port().is_some() {
                return None;
            }
            let host = url.host_str()?.to_ascii_lowercase();
            return Some(OriginRule::AnyPort {
                scheme: url.scheme().to_ascii_lowercase(),
                host,
            });
        }
        let url = Url::parse(entry).ok()?;
        let host = url.host_str()?.to_ascii_lowercase();
        Some(OriginRule::Exact {
            scheme: url.scheme().to_ascii_lowercase(),
            host,
            port: url.port(),
        })
    }

    fn matches(&self, origin: &Url) -> bool {
        let origin_host = match origin.host_str() {
            Some(host) => host,
            None => return false,
        };
        match self {
            OriginRule::Exact { scheme, host, port } => {
                origin.scheme().eq_ignore_ascii_case(scheme)
                    && origin_host.eq_ignore_ascii_case(host)
                    && *port == origin.port()
            }
            OriginRule::AnyPort { scheme, host } => {
                origin.scheme().eq_ignore_ascii_case(scheme)
                    && origin_host.eq_ignore_ascii_case(host)
                    && origin.port().is_some()
            }
        }
    }
}
