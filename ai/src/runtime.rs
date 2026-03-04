use crate::audit::{AuditLogger, ToolCallLog};
use crate::config::{AiMode, ApiConfig, EngineAiConfig, LocalMllConfig};
use crate::world_builder::build_scene_from_prompt;
use anyhow::{Context, bail};
use assets::SceneFile;
use chrono::Utc;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Instant;

pub struct AiOrchestrator {
    config: EngineAiConfig,
    runtime: RuntimeBackend,
    audit_logger: AuditLogger,
}

enum RuntimeBackend {
    Off,
    Api(ApiRuntime),
    Local(LocalRuntime),
}

struct ApiRuntime {
    config: ApiConfig,
}

struct LocalRuntime {
    supervisor: LocalMllSupervisor,
}

impl AiOrchestrator {
    pub fn from_env(audit_root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let config = EngineAiConfig::from_env();
        Self::new(config, audit_root)
    }

    pub fn new(config: EngineAiConfig, audit_root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let runtime = RuntimeBackend::from_mode(&config.mode, &config)?;
        Ok(Self {
            config,
            runtime,
            audit_logger: AuditLogger::new(audit_root),
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
        Ok(())
    }

    pub fn tick(&mut self) -> anyhow::Result<()> {
        match &mut self.runtime {
            RuntimeBackend::Off => Ok(()),
            RuntimeBackend::Api(_) => Ok(()),
            RuntimeBackend::Local(local) => local.supervisor.ensure_running(),
        }
    }

    pub fn world_builder(&mut self, prompt: &str) -> anyhow::Result<SceneFile> {
        let mode = self.config.mode;
        let started = Instant::now();
        let prompt_hash = hash_text(prompt);
        let prompt_preview = prompt.chars().take(120).collect::<String>();

        let result = match &mut self.runtime {
            RuntimeBackend::Off => bail!("AI mode is OFF; world builder is disabled"),
            RuntimeBackend::Api(api) => {
                api.ensure_ready()?;
                Ok(build_scene_from_prompt(prompt))
            }
            RuntimeBackend::Local(local) => {
                local.supervisor.ensure_running()?;
                Ok(build_scene_from_prompt(prompt))
            }
        };

        let status = if result.is_ok() {
            "ok".to_string()
        } else {
            "error".to_string()
        };

        let _ = self.audit_logger.log_tool_call(&ToolCallLog {
            timestamp_utc: Utc::now().to_rfc3339(),
            session_id: self.config.session_id.clone(),
            agent_id: "world_builder".to_string(),
            tool_name: "build_scene".to_string(),
            mode: mode.as_str().to_string(),
            input_hash: prompt_hash,
            input_preview: prompt_preview,
            result_status: status,
            duration_ms: started.elapsed().as_millis(),
        });

        result
    }
}

impl RuntimeBackend {
    fn from_mode(mode: &AiMode, config: &EngineAiConfig) -> anyhow::Result<Self> {
        match mode {
            AiMode::Off => Ok(Self::Off),
            AiMode::Api => Ok(Self::Api(ApiRuntime {
                config: config.api.clone(),
            })),
            AiMode::Local => Ok(Self::Local(LocalRuntime {
                supervisor: LocalMllSupervisor::start(config.local.clone())?,
            })),
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
    fn ensure_ready(&self) -> anyhow::Result<()> {
        if self.config.api_key.is_none() {
            bail!(
                "AI API mode requires AI_API_KEY (provider={})",
                self.config.provider
            );
        }
        Ok(())
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
        if let Some(child) = &mut self.child {
            if let Some(status) = child
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
