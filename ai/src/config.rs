use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::env;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiMode {
    Off,
    Api,
    Local,
}

impl AiMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "OFF",
            Self::Api => "API",
            Self::Local => "LOCAL",
        }
    }
}

impl FromStr for AiMode {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_uppercase().as_str() {
            "OFF" => Ok(Self::Off),
            "API" => Ok(Self::Api),
            "LOCAL" => Ok(Self::Local),
            _ => Err("expected OFF, API, or LOCAL"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub provider: String,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LocalMllConfig {
    pub bin_path: String,
    pub model_path: Option<String>,
    pub host: String,
    pub port: u16,
    pub extra_args: Vec<String>,
    pub max_restarts: u32,
}

#[derive(Debug, Clone)]
pub struct EngineAiConfig {
    pub mode: AiMode,
    pub session_id: String,
    pub api: ApiConfig,
    pub local: LocalMllConfig,
}

impl EngineAiConfig {
    pub fn from_env() -> Self {
        let mode = env_var("AI_MODE")
            .and_then(|raw| AiMode::from_str(&raw).ok())
            .unwrap_or(AiMode::Off);
        let session_id = env_var("AI_SESSION_ID")
            .unwrap_or_else(|| format!("session-{}", Utc::now().timestamp()));

        let api = ApiConfig {
            provider: env_var("AI_API_PROVIDER").unwrap_or_else(|| "openai".to_string()),
            base_url: env_var("AI_API_BASE_URL"),
            api_key: env_var("AI_API_KEY"),
        };

        let local = LocalMllConfig {
            bin_path: env_var("LOCAL_MLL_BIN").unwrap_or_else(|| "llama-server".to_string()),
            model_path: env_var("LOCAL_MLL_MODEL"),
            host: env_var("LOCAL_MLL_HOST").unwrap_or_else(|| "127.0.0.1".to_string()),
            port: env_var("LOCAL_MLL_PORT")
                .and_then(|raw| raw.parse::<u16>().ok())
                .unwrap_or(8080),
            extra_args: env_var("LOCAL_MLL_EXTRA_ARGS")
                .map(|raw| split_args(&raw))
                .unwrap_or_default(),
            max_restarts: env_var("LOCAL_MLL_MAX_RESTARTS")
                .and_then(|raw| raw.parse::<u32>().ok())
                .unwrap_or(2),
        };

        Self {
            mode,
            session_id,
            api,
            local,
        }
    }
}

fn env_var(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn split_args(raw: &str) -> Vec<String> {
    raw.split_whitespace().map(ToOwned::to_owned).collect()
}
