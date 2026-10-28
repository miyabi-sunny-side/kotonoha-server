//! 環境変数からの設定。

use std::path::PathBuf;

#[derive(Debug, PartialEq, Eq)]
pub struct Config {
    pub port: u16,
    pub llama_server_url: String,
    pub api_keys_file: PathBuf,
}

impl Config {
    /// # Errors
    /// `PORT` が数でないとき、または鍵の file の既定の置き場を決める `HOME` が無いとき。
    pub fn from_env(get: impl Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let port = match get("PORT") {
            Some(v) => v.parse().map_err(|e| anyhow::anyhow!("PORT={v:?}: {e}"))?,
            None => 5013,
        };
        let llama_server_url =
            get("LLAMA_SERVER_URL").unwrap_or_else(|| "http://127.0.0.1:5012".to_owned());
        let api_keys_file = if let Some(path) = get("API_KEYS_FILE") {
            PathBuf::from(path)
        } else {
            let home = get("HOME").ok_or_else(|| anyhow::anyhow!("set API_KEYS_FILE or HOME"))?;
            PathBuf::from(home).join(".config/kotonoha-server/api-keys")
        };
        Ok(Self {
            port,
            llama_server_url,
            api_keys_file,
        })
    }
}

/// `LOG_LEVEL` (off/error/warn/info/debug/trace)。未指定と不正な値は info。
#[must_use]
pub fn log_level(value: Option<&str>) -> tracing::level_filters::LevelFilter {
    use tracing::level_filters::LevelFilter;
    match value {
        Some("off") => LevelFilter::OFF,
        Some("error") => LevelFilter::ERROR,
        Some("warn") => LevelFilter::WARN,
        Some("debug") => LevelFilter::DEBUG,
        Some("trace") => LevelFilter::TRACE,
        _ => LevelFilter::INFO,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tracing::level_filters::LevelFilter;

    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn defaults() {
        let c = Config::from_env(env(&[("HOME", "/home/u")])).unwrap();
        assert_eq!(
            c,
            Config {
                port: 5013,
                llama_server_url: "http://127.0.0.1:5012".into(),
                api_keys_file: "/home/u/.config/kotonoha-server/api-keys".into(),
            },
            "既定値"
        );
    }

    #[test]
    fn overrides() {
        let c = Config::from_env(env(&[
            ("PORT", "8080"),
            ("LLAMA_SERVER_URL", "http://gpu:9000"),
            ("API_KEYS_FILE", "/etc/keys"),
        ]))
        .unwrap();
        assert_eq!(c.port, 8080, "PORT で待ち受けを変える");
        assert_eq!(c.llama_server_url, "http://gpu:9000", "上流を env で変える");
        assert_eq!(
            c.api_keys_file,
            PathBuf::from("/etc/keys"),
            "鍵の置き場を env で変える"
        );
    }

    #[test]
    fn rejects_bad_port_and_missing_home() {
        assert!(
            Config::from_env(env(&[("PORT", "x"), ("HOME", "/h")])).is_err(),
            "数でない PORT"
        );
        assert!(
            Config::from_env(env(&[])).is_err(),
            "鍵の置き場が決まらない"
        );
    }

    #[test]
    fn log_levels() {
        assert_eq!(log_level(Some("debug")), LevelFilter::DEBUG, "指定どおり");
        assert_eq!(log_level(Some("off")), LevelFilter::OFF, "off");
        assert_eq!(
            log_level(Some("loud")),
            LevelFilter::INFO,
            "不正な値は info"
        );
        assert_eq!(log_level(None), LevelFilter::INFO, "未指定は info");
    }
}
