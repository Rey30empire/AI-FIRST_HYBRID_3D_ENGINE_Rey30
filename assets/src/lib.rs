use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AssetError {
    #[error("failed to read scene file '{path}': {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse scene json '{path}': {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneFile {
    pub name: String,
    #[serde(default)]
    pub entities: Vec<SceneEntity>,
}

impl Default for SceneFile {
    fn default() -> Self {
        Self {
            name: "Empty Scene".to_string(),
            entities: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneEntity {
    pub name: String,
    #[serde(default = "default_mesh")]
    pub mesh: String,
    #[serde(default = "default_translation")]
    pub translation: [f32; 3],
}

fn default_mesh() -> String {
    "triangle".to_string()
}

fn default_translation() -> [f32; 3] {
    [0.0, 0.0, 0.0]
}

pub fn load_scene(path: impl AsRef<Path>) -> Result<SceneFile, AssetError> {
    let path = path.as_ref();
    let raw = fs::read_to_string(path).map_err(|source| AssetError::Read {
        path: path.display().to_string(),
        source,
    })?;

    serde_json::from_str(&raw).map_err(|source| AssetError::Parse {
        path: path.display().to_string(),
        source,
    })
}
