use anyhow::Context;
use chrono::Utc;
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct ToolCallLog {
    pub timestamp_utc: String,
    pub session_id: String,
    pub agent_id: String,
    pub tool_name: String,
    pub mode: String,
    pub input_hash: String,
    pub input_preview: String,
    pub result_status: String,
    pub duration_ms: u128,
}

#[derive(Debug, Clone)]
pub struct AuditLogger {
    root: PathBuf,
}

impl AuditLogger {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn log_tool_call(&self, entry: &ToolCallLog) -> anyhow::Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("failed to create audit log dir '{}'", self.root.display()))?;

        let date_file = format!("{}.log", Utc::now().format("%Y-%m-%d"));
        let file_path = self.root.join(date_file);
        let serialized =
            serde_json::to_string(entry).context("failed to serialize tool log entry")?;

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .with_context(|| format!("failed to open audit log file '{}'", file_path.display()))?;
        file.write_all(serialized.as_bytes())
            .context("failed to append log entry")?;
        file.write_all(b"\n").context("failed to append newline")?;
        Ok(())
    }
}
