use crate::audit::{AuditLogger, ToolCallLog};
use crate::command_bus::{NodeGraphRuntimeState, RenderSettings};
use crate::config::{AiMode, ApiConfig, EngineAiConfig, LocalMllConfig};
use crate::tool_registry::{ToolRuntime, ToolSchema};
use crate::world_builder::build_scene_from_prompt;
use anyhow::{Context, bail};
use assets::SceneFile;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use std::time::Instant;

pub struct AiOrchestrator {
    config: EngineAiConfig,
    runtime: RuntimeBackend,
    audit_logger: AuditLogger,
    tool_runtime: ToolRuntime,
}

enum RuntimeBackend {
    Off,
    Api(ApiRuntime),
    Local(LocalRuntime),
}

struct ApiRuntime {
    config: ApiConfig,
    http_client: reqwest::blocking::Client,
}

struct LocalRuntime {
    supervisor: LocalMllSupervisor,
    rpc_client: reqwest::blocking::Client,
    rpc_endpoint: String,
    rpc_tool_calls: bool,
    rpc_tool_calls_strict: bool,
}

#[derive(Debug, Serialize)]
struct ToolRpcRequest<'a> {
    schema_version: u32,
    session_id: &'a str,
    mode: &'a str,
    tool_name: &'a str,
    params: &'a Value,
    timestamp_utc: String,
}

#[derive(Debug, Deserialize)]
struct ToolRpcResponse {
    status: String,
    result: Option<Value>,
    error: Option<String>,
    trace_id: Option<String>,
}

impl AiOrchestrator {
    pub fn from_env(audit_root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let config = EngineAiConfig::from_env();
        Self::new(config, audit_root)
    }

    pub fn new(config: EngineAiConfig, audit_root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let runtime = RuntimeBackend::from_mode(&config.mode, &config)?;
        let project_root = std::env::current_dir().unwrap_or_else(|_| ".".into());
        let mut tool_runtime = ToolRuntime::new(project_root);
        tool_runtime.set_ai_mode(config.mode.as_str());
        Ok(Self {
            config,
            runtime,
            audit_logger: AuditLogger::new(audit_root),
            tool_runtime,
        })
    }

    pub fn mode(&self) -> AiMode {
        self.config.mode
    }

    pub fn status(&self) -> String {
        let backend = match self.runtime {
            RuntimeBackend::Off => "off",
            RuntimeBackend::Api(_) => "api",
            RuntimeBackend::Local(_) => "local",
        };
        format!("mode={} backend={}", self.config.mode.as_str(), backend)
    }

    pub fn set_mode(&mut self, mode: AiMode) -> anyhow::Result<()> {
        if self.config.mode == mode {
            return Ok(());
        }
        self.runtime.shutdown();
        self.config.mode = mode;
        self.runtime = RuntimeBackend::from_mode(&self.config.mode, &self.config)?;
        self.tool_runtime.set_ai_mode(self.config.mode.as_str());
        Ok(())
    }

    pub fn set_frame_stats(&mut self, fps: f32) {
        self.tool_runtime.set_frame_stats(fps);
    }

    pub fn sync_scene_from_editor(
        &mut self,
        scene: SceneFile,
        open_scene_path: Option<std::path::PathBuf>,
    ) -> anyhow::Result<()> {
        self.tool_runtime
            .sync_scene_from_editor(scene, open_scene_path)
    }

    pub fn tool_scene_snapshot(&self) -> SceneFile {
        self.tool_runtime.scene_snapshot()
    }

    pub fn tool_scene_revision(&self) -> u64 {
        self.tool_runtime.scene_revision()
    }

    pub fn tool_render_settings(&self) -> RenderSettings {
        self.tool_runtime.render_settings()
    }

    pub fn tool_lowcode_state(&self) -> NodeGraphRuntimeState {
        self.tool_runtime.lowcode_state()
    }

    pub fn tick(&mut self) -> anyhow::Result<()> {
        match &mut self.runtime {
            RuntimeBackend::Off => Ok(()),
            RuntimeBackend::Api(_) => Ok(()),
            RuntimeBackend::Local(local) => local.supervisor.ensure_running(),
        }
    }

    pub fn tool_catalog(&self) -> Vec<ToolSchema> {
        self.tool_runtime.list_tools()
    }

    pub fn execute_tool(&mut self, tool_name: &str, params: Value) -> anyhow::Result<Value> {
        let mode = self.config.mode;
        let input_raw = serde_json::to_string(&params).unwrap_or_else(|_| "{}".to_string());
        let remote_started = Instant::now();
        match self
            .runtime
            .try_remote_tool_call(&self.config.session_id, mode, tool_name, &params)
        {
            Ok(Some(result)) => {
                self.log_tool_call(
                    "tool_remote",
                    tool_name,
                    mode,
                    &input_raw,
                    true,
                    remote_started.elapsed().as_millis(),
                );
                return Ok(result);
            }
            Ok(None) => {}
            Err(err) => {
                self.log_tool_call(
                    "tool_remote",
                    tool_name,
                    mode,
                    &input_raw,
                    false,
                    remote_started.elapsed().as_millis(),
                );
                if self.runtime.remote_tool_call_strict() {
                    return Err(err.context("remote tool call failed in strict mode"));
                }
                log::warn!("remote tool call failed, falling back to local runtime: {err}");
            }
        }

        let local_started = Instant::now();
        let result = self.tool_runtime.execute(tool_name, params);
        self.log_tool_call(
            "tool_runtime",
            tool_name,
            mode,
            &input_raw,
            result.is_ok(),
            local_started.elapsed().as_millis(),
        );
        result
    }

    pub fn world_builder(&mut self, prompt: &str) -> anyhow::Result<SceneFile> {
        let mode = self.config.mode;
        let started = Instant::now();

        let result = match &mut self.runtime {
            RuntimeBackend::Off => Ok(build_scene_from_prompt(prompt)),
            RuntimeBackend::Api(_) => Ok(build_scene_from_prompt(prompt)),
            RuntimeBackend::Local(local) => {
                if let Err(err) = local.supervisor.ensure_running() {
                    log::warn!(
                        "local MLL unavailable for world builder, using deterministic fallback: {err}"
                    );
                }
                Ok(build_scene_from_prompt(prompt))
            }
        };

        self.log_tool_call(
            "world_builder",
            "build_scene",
            mode,
            prompt,
            result.is_ok(),
            started.elapsed().as_millis(),
        );

        result
    }

    fn log_tool_call(
        &self,
        agent_id: &str,
        tool_name: &str,
        mode: AiMode,
        input_raw: &str,
        success: bool,
        duration_ms: u128,
    ) {
        let _ = self.audit_logger.log_tool_call(&ToolCallLog {
            timestamp_utc: Utc::now().to_rfc3339(),
            session_id: self.config.session_id.clone(),
            agent_id: agent_id.to_string(),
            tool_name: tool_name.to_string(),
            mode: mode.as_str().to_string(),
            input_hash: hash_text(input_raw),
            input_preview: input_raw.chars().take(120).collect::<String>(),
            result_status: if success {
                "ok".to_string()
            } else {
                "error".to_string()
            },
            duration_ms,
        });
    }
}

impl RuntimeBackend {
    fn from_mode(mode: &AiMode, config: &EngineAiConfig) -> anyhow::Result<Self> {
        match mode {
            AiMode::Off => Ok(Self::Off),
            AiMode::Api => {
                let timeout = Duration::from_millis(config.api.timeout_ms.max(250));
                let http_client = reqwest::blocking::Client::builder()
                    .timeout(timeout)
                    .build()
                    .context("failed to initialize API HTTP client")?;
                Ok(Self::Api(ApiRuntime {
                    config: config.api.clone(),
                    http_client,
                }))
            }
            AiMode::Local => {
                let timeout = Duration::from_millis(config.local.rpc_timeout_ms.max(250));
                let rpc_client = reqwest::blocking::Client::builder()
                    .timeout(timeout)
                    .build()
                    .context("failed to initialize LOCAL RPC HTTP client")?;
                Ok(Self::Local(LocalRuntime {
                    supervisor: LocalMllSupervisor::start(config.local.clone())?,
                    rpc_client,
                    rpc_endpoint: format!(
                        "http://{}:{}{}",
                        config.local.host,
                        config.local.port,
                        normalize_rpc_path(&config.local.rpc_path)
                    ),
                    rpc_tool_calls: config.local.rpc_tool_calls,
                    rpc_tool_calls_strict: config.local.rpc_tool_calls_strict,
                }))
            }
        }
    }

    fn try_remote_tool_call(
        &mut self,
        session_id: &str,
        mode: AiMode,
        tool_name: &str,
        params: &Value,
    ) -> anyhow::Result<Option<Value>> {
        match self {
            RuntimeBackend::Off => Ok(None),
            RuntimeBackend::Api(api) => {
                api.try_remote_tool_call(session_id, mode, tool_name, params)
            }
            RuntimeBackend::Local(local) => {
                local.supervisor.ensure_running()?;
                local.try_remote_tool_call(session_id, mode, tool_name, params)
            }
        }
    }

    fn remote_tool_call_strict(&self) -> bool {
        match self {
            RuntimeBackend::Off => false,
            RuntimeBackend::Api(api) => api.config.remote_tool_calls_strict,
            RuntimeBackend::Local(local) => local.rpc_tool_calls_strict,
        }
    }

    fn shutdown(&mut self) {
        if let RuntimeBackend::Local(local) = self {
            local.supervisor.shutdown();
        }
    }
}

impl Drop for AiOrchestrator {
    fn drop(&mut self) {
        self.runtime.shutdown();
    }
}

impl ApiRuntime {
    fn try_remote_tool_call(
        &self,
        session_id: &str,
        mode: AiMode,
        tool_name: &str,
        params: &Value,
    ) -> anyhow::Result<Option<Value>> {
        if !self.config.remote_tool_calls {
            return Ok(None);
        }
        let endpoint = resolve_api_tool_endpoint(&self.config)?;
        let request = ToolRpcRequest {
            schema_version: 1,
            session_id,
            mode: mode.as_str(),
            tool_name,
            params,
            timestamp_utc: Utc::now().to_rfc3339(),
        };
        let mut builder = self.http_client.post(&endpoint).json(&request);
        if let Some(api_key) = &self.config.api_key {
            builder = builder.bearer_auth(api_key);
        }

        let response = builder
            .send()
            .with_context(|| format!("API remote tool call failed to '{}'", endpoint))?;
        parse_rpc_response(response, "api")
    }
}

impl LocalRuntime {
    fn try_remote_tool_call(
        &self,
        session_id: &str,
        mode: AiMode,
        tool_name: &str,
        params: &Value,
    ) -> anyhow::Result<Option<Value>> {
        if !self.rpc_tool_calls {
            return Ok(None);
        }
        let request = ToolRpcRequest {
            schema_version: 1,
            session_id,
            mode: mode.as_str(),
            tool_name,
            params,
            timestamp_utc: Utc::now().to_rfc3339(),
        };
        let response = self
            .rpc_client
            .post(&self.rpc_endpoint)
            .json(&request)
            .send()
            .with_context(|| format!("LOCAL RPC tool call failed to '{}'", self.rpc_endpoint))?;
        parse_rpc_response(response, "local")
    }
}

fn resolve_api_tool_endpoint(config: &ApiConfig) -> anyhow::Result<String> {
    if let Some(endpoint) = &config.tool_endpoint {
        let endpoint = endpoint.trim();
        if endpoint.is_empty() {
            bail!("AI_API_TOOL_ENDPOINT cannot be empty when provided");
        }
        return Ok(endpoint.to_string());
    }
    let Some(base_url) = &config.base_url else {
        bail!("API remote tool calls require AI_API_BASE_URL or AI_API_TOOL_ENDPOINT");
    };
    let normalized = base_url.trim_end_matches('/');
    Ok(format!("{}/tool-call", normalized))
}

fn normalize_rpc_path(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        "/tool-call".to_string()
    } else if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{}", trimmed)
    }
}

fn parse_rpc_response(
    response: reqwest::blocking::Response,
    source: &str,
) -> anyhow::Result<Option<Value>> {
    let status = response.status();
    let body = response
        .text()
        .with_context(|| format!("{} RPC failed to read response body", source))?;
    if !status.is_success() {
        bail!(
            "{} RPC returned HTTP {}: {}",
            source,
            status,
            truncate_text(&body)
        );
    }

    let payload: ToolRpcResponse = serde_json::from_str(&body)
        .with_context(|| format!("{} RPC returned invalid JSON payload", source))?;
    match payload.status.to_ascii_lowercase().as_str() {
        "ok" | "success" => {
            if let Some(trace_id) = payload.trace_id.as_deref() {
                log::debug!("{} RPC trace_id={}", source, trace_id);
            }
            Ok(Some(
                payload.result.unwrap_or_else(|| json!({ "status": "ok" })),
            ))
        }
        _ => {
            let err = payload
                .error
                .unwrap_or_else(|| "unknown remote error".to_string());
            if let Some(trace_id) = payload.trace_id.as_deref() {
                bail!("{} RPC error: {} (trace_id={})", source, err, trace_id);
            }
            bail!("{} RPC error: {}", source, err);
        }
    }
}

fn truncate_text(text: &str) -> String {
    const MAX_CHARS: usize = 240;
    if text.chars().count() <= MAX_CHARS {
        text.to_string()
    } else {
        let mut truncated = text.chars().take(MAX_CHARS).collect::<String>();
        truncated.push_str("...");
        truncated
    }
}

struct LocalMllSupervisor {
    config: LocalMllConfig,
    child: Option<Child>,
    restart_count: u32,
}

impl LocalMllSupervisor {
    fn start(config: LocalMllConfig) -> anyhow::Result<Self> {
        let mut this = Self {
            config,
            child: None,
            restart_count: 0,
        };
        this.spawn_child()?;
        Ok(this)
    }

    fn ensure_running(&mut self) -> anyhow::Result<()> {
        if let Some(child) = &mut self.child
            && let Some(status) = child
                .try_wait()
                .context("failed to poll local MLL process")?
        {
            log::warn!("local MLL exited with status: {}", status);
            self.child = None;
            if self.restart_count < self.config.max_restarts {
                self.restart_count += 1;
                self.spawn_child()?;
            } else {
                bail!("local MLL restart limit reached");
            }
        }
        Ok(())
    }

    fn spawn_child(&mut self) -> anyhow::Result<()> {
        let mut command = Command::new(&self.config.bin_path);
        if let Some(model_path) = &self.config.model_path {
            command.arg("--model").arg(model_path);
        }
        command
            .arg("--host")
            .arg(&self.config.host)
            .arg("--port")
            .arg(self.config.port.to_string());
        for arg in &self.config.extra_args {
            command.arg(arg);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = command
            .spawn()
            .with_context(|| format!("failed to spawn local MLL '{}'", self.config.bin_path))?;
        self.child = Some(child);
        log::info!(
            "local MLL process started: '{}' on {}:{}",
            self.config.bin_path,
            self.config.host,
            self.config.port
        );
        Ok(())
    }

    fn shutdown(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
            log::info!("local MLL process stopped");
        }
    }
}

fn hash_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_config(mode: AiMode) -> EngineAiConfig {
        EngineAiConfig {
            mode,
            session_id: "test-session".to_string(),
            api: ApiConfig {
                provider: "openai".to_string(),
                base_url: None,
                api_key: None,
                tool_endpoint: None,
                remote_tool_calls: false,
                remote_tool_calls_strict: false,
                timeout_ms: 1000,
            },
            local: LocalMllConfig {
                bin_path: "llama-server".to_string(),
                model_path: None,
                host: "127.0.0.1".to_string(),
                port: 8080,
                extra_args: Vec::new(),
                max_restarts: 0,
                rpc_tool_calls: false,
                rpc_tool_calls_strict: false,
                rpc_path: "/tool-call".to_string(),
                rpc_timeout_ms: 1000,
            },
        }
    }

    fn temp_audit_root(test_name: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("ai_runtime_{test_name}_{now}"))
    }

    #[test]
    fn normalize_rpc_path_adds_leading_slash() {
        assert_eq!(normalize_rpc_path("tool-call"), "/tool-call");
        assert_eq!(normalize_rpc_path("/rpc"), "/rpc");
        assert_eq!(normalize_rpc_path(""), "/tool-call");
    }

    #[test]
    fn tool_rpc_request_serializes_with_schema_version() {
        let request = ToolRpcRequest {
            schema_version: 1,
            session_id: "session-1",
            mode: "API",
            tool_name: "tool.get_engine_state",
            params: &json!({"k":"v"}),
            timestamp_utc: "2026-03-05T00:00:00Z".to_string(),
        };
        let encoded = serde_json::to_value(&request).expect("request should serialize");
        assert_eq!(
            encoded.get("schema_version").and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            encoded.get("tool_name").and_then(Value::as_str),
            Some("tool.get_engine_state")
        );
    }

    #[test]
    fn world_builder_is_available_in_off_mode() {
        let mut orchestrator = AiOrchestrator::new(
            test_config(AiMode::Off),
            temp_audit_root("world_builder_off"),
        )
        .expect("off mode orchestrator should initialize");
        let scene = orchestrator
            .world_builder("create a medieval island map")
            .expect("world builder should work in off mode");
        assert_eq!(scene.name, "Generated Medieval Island");
        assert!(!scene.entities.is_empty());
    }

    #[test]
    fn world_builder_in_api_mode_does_not_require_api_key() {
        let mut orchestrator = AiOrchestrator::new(
            test_config(AiMode::Api),
            temp_audit_root("world_builder_api"),
        )
        .expect("api mode orchestrator should initialize");
        let scene = orchestrator
            .world_builder("create a shooter arena demo")
            .expect("world builder should not require api key");
        assert_eq!(scene.name, "Generated Shooter Arena");
        assert!(!scene.entities.is_empty());
    }
}
