use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

pub const GRAPH_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeGraphFile {
    pub version: u32,
    pub graph_name: String,
    #[serde(default)]
    pub nodes: Vec<NodeGraphNode>,
    #[serde(default)]
    pub edges: Vec<NodeGraphEdge>,
}

impl NodeGraphFile {
    pub fn new(graph_name: impl Into<String>) -> Self {
        Self {
            version: GRAPH_SCHEMA_VERSION,
            graph_name: graph_name.into(),
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeGraphNode {
    pub id: String,
    #[serde(rename = "type")]
    pub node_type: String,
    #[serde(default = "default_graph_params")]
    pub params: Value,
}

fn default_graph_params() -> Value {
    json!({})
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeGraphEdge {
    pub from: String,
    pub to: String,
    #[serde(default = "default_graph_edge_pin")]
    pub pin: String,
}

fn default_graph_edge_pin() -> String {
    "flow".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct NodeGraphValidationReport {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

pub fn validate_node_graph(graph: &NodeGraphFile) -> NodeGraphValidationReport {
    let mut errors = Vec::<String>::new();
    let mut warnings = Vec::<String>::new();
    if graph.version != GRAPH_SCHEMA_VERSION {
        errors.push(format!(
            "unsupported graph version {} (expected {})",
            graph.version, GRAPH_SCHEMA_VERSION
        ));
    }
    if graph.graph_name.trim().is_empty() {
        errors.push("graph_name cannot be empty".to_string());
    }

    let mut node_ids = HashSet::<String>::new();
    let mut node_types = HashMap::<String, String>::new();
    for node in &graph.nodes {
        let node_id = node.id.trim();
        if node_id.is_empty() {
            errors.push("graph node id cannot be empty".to_string());
            continue;
        }
        if !node_ids.insert(node_id.to_string()) {
            errors.push(format!("duplicate node id '{}'", node_id));
        }
        if !supported_node_type(node.node_type.as_str()) {
            errors.push(format!(
                "node '{}' has unsupported type '{}'",
                node_id, node.node_type
            ));
        }
        node_types.insert(node_id.to_string(), node.node_type.clone());
    }
    if graph.nodes.is_empty() {
        warnings.push("graph has no nodes".to_string());
    }

    let mut edge_set = HashSet::<(String, String, String)>::new();
    let mut adjacency = HashMap::<String, Vec<String>>::new();
    let mut indegree = HashMap::<String, usize>::new();
    for node_id in &node_ids {
        indegree.insert(node_id.clone(), 0);
    }

    for edge in &graph.edges {
        let from = edge.from.trim();
        let to = edge.to.trim();
        let pin = edge.pin.trim();
        if from.is_empty() || to.is_empty() {
            errors.push("graph edge 'from'/'to' cannot be empty".to_string());
            continue;
        }
        if !node_ids.contains(from) {
            errors.push(format!(
                "edge references missing source node '{}' (from='{}', to='{}')",
                from, from, to
            ));
        }
        if !node_ids.contains(to) {
            errors.push(format!(
                "edge references missing target node '{}' (from='{}', to='{}')",
                to, from, to
            ));
        }
        if from == to {
            errors.push(format!("self-edge is not allowed ('{}' -> '{}')", from, to));
        }
        if !edge_set.insert((from.to_string(), to.to_string(), pin.to_string())) {
            errors.push(format!(
                "duplicate edge detected (from='{}', to='{}', pin='{}')",
                from, to, pin
            ));
        }
        if node_ids.contains(from) && node_ids.contains(to) {
            adjacency
                .entry(from.to_string())
                .or_default()
                .push(to.to_string());
            if let Some(entry) = indegree.get_mut(to) {
                *entry += 1;
            }
        }
    }

    let event_roots = node_types
        .iter()
        .filter_map(|(node_id, node_type)| {
            if is_event_node(node_type) {
                Some(node_id.clone())
            } else {
                None
            }
        })
        .collect::<Vec<String>>();
    if event_roots.is_empty() {
        warnings
            .push("graph has no event root nodes (OnStart/OnUpdate/OnTriggerEnter)".to_string());
    }

    let mut zero_indegree = BTreeSet::<String>::new();
    for (node_id, degree) in &indegree {
        if *degree == 0 {
            zero_indegree.insert(node_id.clone());
        }
    }
    let mut visited = 0usize;
    let mut indegree_mut = indegree;
    while let Some(next) = zero_indegree.iter().next().cloned() {
        zero_indegree.remove(&next);
        visited += 1;
        if let Some(children) = adjacency.get(&next) {
            let mut sorted_children = children.clone();
            sorted_children.sort();
            for child in sorted_children {
                if let Some(entry) = indegree_mut.get_mut(&child) {
                    *entry = entry.saturating_sub(1);
                    if *entry == 0 {
                        zero_indegree.insert(child);
                    }
                }
            }
        }
    }
    if visited < graph.nodes.len() {
        errors.push("graph contains a cycle; deterministic execution requires a DAG".to_string());
    }

    NodeGraphValidationReport {
        valid: errors.is_empty(),
        errors,
        warnings,
    }
}

pub fn supported_node_types() -> &'static [&'static str] {
    &[
        "OnStart",
        "OnUpdate",
        "OnTriggerEnter",
        "Sequence",
        "Branch",
        "Delay",
        "SpawnEntity",
        "MoveTo",
        "PlayAnimation",
        "ApplyDamage",
        "SetLightPreset",
        "LoadSubScene",
        "SetWeather",
        "ShowMessage",
        "SetObjective",
        "GetEntity",
        "GetPlayer",
        "RandomFloat",
    ]
}

pub fn supported_node_type(node_type: &str) -> bool {
    supported_node_types()
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(node_type.trim()))
}

fn is_event_node(node_type: &str) -> bool {
    node_type.eq_ignore_ascii_case("OnStart")
        || node_type.eq_ignore_ascii_case("OnUpdate")
        || node_type.eq_ignore_ascii_case("OnTriggerEnter")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemplateAssetBundle {
    pub template_id: String,
    #[serde(default)]
    pub mesh_assets: Vec<String>,
    #[serde(default)]
    pub material_assets: Vec<String>,
    #[serde(default)]
    pub audio_assets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TemplateSpec {
    pub template_id: String,
    pub display_name: String,
    pub scene: SceneFile,
    pub graph: NodeGraphFile,
    pub asset_bundle: TemplateAssetBundle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TemplateBundleValidationReport {
    pub valid: bool,
    pub missing_assets: Vec<String>,
}

pub fn builtin_template_specs() -> Vec<TemplateSpec> {
    vec![
        shooter_template_spec(),
        medieval_island_template_spec(),
        platform_runner_template_spec(),
    ]
}

pub fn builtin_template_spec(template_id: &str) -> Option<TemplateSpec> {
    builtin_template_specs()
        .into_iter()
        .find(|spec| spec.template_id.eq_ignore_ascii_case(template_id))
}

pub fn builtin_template_bundle(template_id: &str) -> Option<TemplateAssetBundle> {
    builtin_template_spec(template_id).map(|spec| spec.asset_bundle)
}

pub fn validate_template_bundle(
    bundle: &TemplateAssetBundle,
    project_root: impl AsRef<Path>,
) -> TemplateBundleValidationReport {
    let project_root = project_root.as_ref();
    let mut missing = Vec::<String>::new();
    for asset in bundle
        .mesh_assets
        .iter()
        .chain(bundle.material_assets.iter())
        .chain(bundle.audio_assets.iter())
    {
        if asset.trim().is_empty() {
            missing.push("<empty asset id>".to_string());
            continue;
        }
        if is_builtin_asset_id(asset) || !asset_reference_looks_like_path(asset) {
            continue;
        }
        let path = resolve_project_path(project_root, Path::new(asset));
        if !path.exists() || !path.is_file() {
            missing.push(asset.clone());
        }
    }
    missing.sort();
    missing.dedup();
    TemplateBundleValidationReport {
        valid: missing.is_empty(),
        missing_assets: missing,
    }
}

fn is_builtin_asset_id(asset_id: &str) -> bool {
    matches!(
        asset_id.trim().to_ascii_lowercase().as_str(),
        "cube" | "triangle" | "octahedron" | "tetrahedron"
    )
}

fn asset_reference_looks_like_path(asset_ref: &str) -> bool {
    let trimmed = asset_ref.trim();
    if trimmed.contains('/') || trimmed.contains('\\') {
        return true;
    }
    Path::new(trimmed).extension().is_some()
}

fn shooter_template_spec() -> TemplateSpec {
    TemplateSpec {
        template_id: "template_shooter_arena".to_string(),
        display_name: "Shooter Arena".to_string(),
        scene: SceneFile {
            name: "Template Shooter Arena".to_string(),
            entities: vec![
                SceneEntity {
                    name: "PlayerSpawn".to_string(),
                    mesh: "cube".to_string(),
                    translation: [0.0, 0.0, 0.0],
                },
                SceneEntity {
                    name: "ArenaCenter".to_string(),
                    mesh: "cube".to_string(),
                    translation: [0.0, -1.0, 0.0],
                },
                SceneEntity {
                    name: "AmmoPickup_A".to_string(),
                    mesh: "cube".to_string(),
                    translation: [3.5, 0.0, -1.5],
                },
            ],
        },
        graph: NodeGraphFile {
            version: GRAPH_SCHEMA_VERSION,
            graph_name: "template_shooter".to_string(),
            nodes: vec![
                NodeGraphNode {
                    id: "n_start".to_string(),
                    node_type: "OnStart".to_string(),
                    params: json!({}),
                },
                NodeGraphNode {
                    id: "n_spawn".to_string(),
                    node_type: "SpawnEntity".to_string(),
                    params: json!({
                        "base_name": "EnemyGrunt",
                        "mesh": "cube",
                        "origin": [0.0, 0.0, -4.0],
                        "count": 3
                    }),
                },
                NodeGraphNode {
                    id: "n_objective".to_string(),
                    node_type: "SetObjective".to_string(),
                    params: json!({ "text": "Defeat enemy wave" }),
                },
                NodeGraphNode {
                    id: "n_message".to_string(),
                    node_type: "ShowMessage".to_string(),
                    params: json!({ "text": "Shooter template ready" }),
                },
            ],
            edges: vec![
                NodeGraphEdge {
                    from: "n_start".to_string(),
                    to: "n_spawn".to_string(),
                    pin: "flow".to_string(),
                },
                NodeGraphEdge {
                    from: "n_spawn".to_string(),
                    to: "n_objective".to_string(),
                    pin: "flow".to_string(),
                },
                NodeGraphEdge {
                    from: "n_objective".to_string(),
                    to: "n_message".to_string(),
                    pin: "flow".to_string(),
                },
            ],
        },
        asset_bundle: TemplateAssetBundle {
            template_id: "template_shooter_arena".to_string(),
            mesh_assets: vec!["cube".to_string()],
            material_assets: vec!["mat_shooter_default".to_string()],
            audio_assets: vec!["audio_shot_basic".to_string()],
        },
    }
}

fn medieval_island_template_spec() -> TemplateSpec {
    TemplateSpec {
        template_id: "template_medieval_island".to_string(),
        display_name: "Medieval Island".to_string(),
        scene: SceneFile {
            name: "Template Medieval Island".to_string(),
            entities: vec![
                SceneEntity {
                    name: "IslandTerrain".to_string(),
                    mesh: "cube".to_string(),
                    translation: [0.0, -1.0, 0.0],
                },
                SceneEntity {
                    name: "VillageCore".to_string(),
                    mesh: "cube".to_string(),
                    translation: [2.5, 0.0, -1.2],
                },
                SceneEntity {
                    name: "QuestNpc_A".to_string(),
                    mesh: "cube".to_string(),
                    translation: [-1.5, 0.0, 1.0],
                },
            ],
        },
        graph: NodeGraphFile {
            version: GRAPH_SCHEMA_VERSION,
            graph_name: "template_medieval_island".to_string(),
            nodes: vec![
                NodeGraphNode {
                    id: "start".to_string(),
                    node_type: "OnStart".to_string(),
                    params: json!({}),
                },
                NodeGraphNode {
                    id: "weather".to_string(),
                    node_type: "SetWeather".to_string(),
                    params: json!({ "preset": "sunset_hazy" }),
                },
                NodeGraphNode {
                    id: "light".to_string(),
                    node_type: "SetLightPreset".to_string(),
                    params: json!({ "preset": "golden_hour" }),
                },
                NodeGraphNode {
                    id: "msg".to_string(),
                    node_type: "ShowMessage".to_string(),
                    params: json!({ "text": "Island template loaded" }),
                },
            ],
            edges: vec![
                NodeGraphEdge {
                    from: "start".to_string(),
                    to: "weather".to_string(),
                    pin: "flow".to_string(),
                },
                NodeGraphEdge {
                    from: "weather".to_string(),
                    to: "light".to_string(),
                    pin: "flow".to_string(),
                },
                NodeGraphEdge {
                    from: "light".to_string(),
                    to: "msg".to_string(),
                    pin: "flow".to_string(),
                },
            ],
        },
        asset_bundle: TemplateAssetBundle {
            template_id: "template_medieval_island".to_string(),
            mesh_assets: vec!["cube".to_string()],
            material_assets: vec![
                "mat_island_stone".to_string(),
                "mat_island_ground".to_string(),
            ],
            audio_assets: vec!["audio_wind_loop".to_string()],
        },
    }
}

fn platform_runner_template_spec() -> TemplateSpec {
    TemplateSpec {
        template_id: "template_platform_runner".to_string(),
        display_name: "Platform Runner".to_string(),
        scene: SceneFile {
            name: "Template Platform Runner".to_string(),
            entities: vec![
                SceneEntity {
                    name: "StartPlatform".to_string(),
                    mesh: "cube".to_string(),
                    translation: [0.0, 0.0, 0.0],
                },
                SceneEntity {
                    name: "CheckpointA".to_string(),
                    mesh: "cube".to_string(),
                    translation: [2.0, 0.5, -1.2],
                },
                SceneEntity {
                    name: "GoalPlatform".to_string(),
                    mesh: "cube".to_string(),
                    translation: [6.0, 1.2, -2.5],
                },
            ],
        },
        graph: NodeGraphFile {
            version: GRAPH_SCHEMA_VERSION,
            graph_name: "template_platform_runner".to_string(),
            nodes: vec![
                NodeGraphNode {
                    id: "start".to_string(),
                    node_type: "OnStart".to_string(),
                    params: json!({}),
                },
                NodeGraphNode {
                    id: "objective".to_string(),
                    node_type: "SetObjective".to_string(),
                    params: json!({ "text": "Reach the goal platform" }),
                },
                NodeGraphNode {
                    id: "hint".to_string(),
                    node_type: "ShowMessage".to_string(),
                    params: json!({ "text": "Use checkpoints to recover progress" }),
                },
            ],
            edges: vec![
                NodeGraphEdge {
                    from: "start".to_string(),
                    to: "objective".to_string(),
                    pin: "flow".to_string(),
                },
                NodeGraphEdge {
                    from: "objective".to_string(),
                    to: "hint".to_string(),
                    pin: "flow".to_string(),
                },
            ],
        },
        asset_bundle: TemplateAssetBundle {
            template_id: "template_platform_runner".to_string(),
            mesh_assets: vec!["cube".to_string()],
            material_assets: vec!["mat_platform_default".to_string()],
            audio_assets: vec!["audio_jump".to_string()],
        },
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct AssetCacheStats {
    pub capacity_bytes: usize,
    pub used_bytes: usize,
    pub cached_assets: usize,
    pub pending_requests: usize,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub evictions: u64,
}

struct CachedAssetEntry {
    bytes: Arc<Vec<u8>>,
    size_bytes: usize,
    last_access_tick: u64,
}

struct LoadRequest {
    asset_id: String,
    path: PathBuf,
}

struct LoadResponse {
    asset_id: String,
    bytes: Result<Vec<u8>, String>,
}

pub struct AsyncAssetCache {
    project_root: PathBuf,
    capacity_bytes: usize,
    used_bytes: usize,
    entries: HashMap<String, CachedAssetEntry>,
    pending: HashSet<String>,
    request_tx: Sender<LoadRequest>,
    response_rx: Receiver<LoadResponse>,
    access_tick: u64,
    cache_hits: u64,
    cache_misses: u64,
    evictions: u64,
}

impl AsyncAssetCache {
    pub fn new(project_root: impl Into<PathBuf>, capacity_bytes: usize) -> Self {
        let (request_tx, request_rx) = mpsc::channel::<LoadRequest>();
        let (response_tx, response_rx) = mpsc::channel::<LoadResponse>();

        thread::Builder::new()
            .name("asset-cache-worker".to_string())
            .spawn(move || {
                while let Ok(request) = request_rx.recv() {
                    let bytes = fs::read(&request.path).map_err(|err| {
                        format!(
                            "failed to read asset '{}' from '{}': {}",
                            request.asset_id,
                            request.path.display(),
                            err
                        )
                    });
                    let _ = response_tx.send(LoadResponse {
                        asset_id: request.asset_id,
                        bytes,
                    });
                }
            })
            .expect("failed to spawn asset cache worker");

        Self {
            project_root: project_root.into(),
            capacity_bytes: capacity_bytes.max(1),
            used_bytes: 0,
            entries: HashMap::new(),
            pending: HashSet::new(),
            request_tx,
            response_rx,
            access_tick: 0,
            cache_hits: 0,
            cache_misses: 0,
            evictions: 0,
        }
    }

    pub fn request_load(&mut self, asset_id: impl Into<String>, path: impl AsRef<Path>) -> bool {
        let asset_id = asset_id.into();
        if asset_id.trim().is_empty()
            || self.entries.contains_key(&asset_id)
            || self.pending.contains(&asset_id)
        {
            return false;
        }
        let path = resolve_project_path(&self.project_root, path.as_ref());
        if self
            .request_tx
            .send(LoadRequest {
                asset_id: asset_id.clone(),
                path,
            })
            .is_err()
        {
            return false;
        }
        self.pending.insert(asset_id);
        true
    }

    pub fn prefetch_paths<I>(&mut self, items: I) -> usize
    where
        I: IntoIterator<Item = (String, PathBuf)>,
    {
        let mut requested = 0usize;
        for (asset_id, path) in items {
            if self.request_load(asset_id, path) {
                requested += 1;
            }
        }
        requested
    }

    pub fn poll(&mut self) -> usize {
        let mut completed = 0usize;
        while let Ok(response) = self.response_rx.try_recv() {
            completed += 1;
            self.pending.remove(&response.asset_id);
            if let Ok(bytes) = response.bytes {
                self.insert_entry(response.asset_id, bytes);
            }
        }
        completed
    }

    pub fn get(&mut self, asset_id: &str) -> Option<Arc<Vec<u8>>> {
        self.access_tick = self.access_tick.saturating_add(1);
        if let Some(entry) = self.entries.get_mut(asset_id) {
            self.cache_hits = self.cache_hits.saturating_add(1);
            entry.last_access_tick = self.access_tick;
            Some(entry.bytes.clone())
        } else {
            self.cache_misses = self.cache_misses.saturating_add(1);
            None
        }
    }

    pub fn unload(&mut self, asset_id: &str) -> bool {
        if let Some(entry) = self.entries.remove(asset_id) {
            self.used_bytes = self.used_bytes.saturating_sub(entry.size_bytes);
            true
        } else {
            false
        }
    }

    pub fn stats(&self) -> AssetCacheStats {
        AssetCacheStats {
            capacity_bytes: self.capacity_bytes,
            used_bytes: self.used_bytes,
            cached_assets: self.entries.len(),
            pending_requests: self.pending.len(),
            cache_hits: self.cache_hits,
            cache_misses: self.cache_misses,
            evictions: self.evictions,
        }
    }

    fn insert_entry(&mut self, asset_id: String, bytes: Vec<u8>) {
        let size_bytes = bytes.len();
        if size_bytes > self.capacity_bytes {
            return;
        }

        self.evict_until_fits(size_bytes);

        if let Some(previous) = self.entries.remove(&asset_id) {
            self.used_bytes = self.used_bytes.saturating_sub(previous.size_bytes);
        }

        self.access_tick = self.access_tick.saturating_add(1);
        self.used_bytes = self.used_bytes.saturating_add(size_bytes);
        self.entries.insert(
            asset_id,
            CachedAssetEntry {
                bytes: Arc::new(bytes),
                size_bytes,
                last_access_tick: self.access_tick,
            },
        );
    }

    fn evict_until_fits(&mut self, required_bytes: usize) {
        while self.used_bytes.saturating_add(required_bytes) > self.capacity_bytes
            && !self.entries.is_empty()
        {
            let Some((oldest_id, _)) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_access_tick)
            else {
                break;
            };
            let oldest_id = oldest_id.clone();
            if let Some(removed) = self.entries.remove(&oldest_id) {
                self.used_bytes = self.used_bytes.saturating_sub(removed.size_bytes);
                self.evictions = self.evictions.saturating_add(1);
            } else {
                break;
            }
        }
    }
}

fn resolve_project_path(project_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    #[test]
    fn async_asset_cache_loads_and_respects_capacity() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("asset_cache_test_{}", unique));
        fs::create_dir_all(&root).expect("should create temp directory");
        let file_a = root.join("a.bin");
        let file_b = root.join("b.bin");
        fs::write(&file_a, vec![1u8; 4]).expect("should write file_a");
        fs::write(&file_b, vec![2u8; 4]).expect("should write file_b");

        let mut cache = AsyncAssetCache::new(&root, 6);
        assert!(cache.request_load("a", PathBuf::from("a.bin")));
        assert!(cache.request_load("b", PathBuf::from("b.bin")));
        for _ in 0..32 {
            cache.poll();
            if cache.stats().pending_requests == 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        let stats = cache.stats();
        assert_eq!(stats.pending_requests, 0);
        assert!(stats.used_bytes <= stats.capacity_bytes);
        assert!(stats.cached_assets >= 1);
        assert!(cache.get("a").is_some() || cache.get("b").is_some());

        fs::remove_dir_all(&root).expect("should clean temp directory");
    }

    #[test]
    fn async_asset_cache_ignores_duplicate_requests() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("asset_cache_test_dup_{}", unique));
        fs::create_dir_all(&root).expect("should create temp directory");
        fs::write(root.join("mesh.bin"), vec![9u8; 8]).expect("should write mesh");

        let mut cache = AsyncAssetCache::new(&root, 1024);
        assert!(cache.request_load("mesh", PathBuf::from("mesh.bin")));
        assert!(!cache.request_load("mesh", PathBuf::from("mesh.bin")));

        fs::remove_dir_all(&root).expect("should clean temp directory");
    }

    #[test]
    fn validate_node_graph_accepts_builtin_template_graphs() {
        for spec in builtin_template_specs() {
            let report = validate_node_graph(&spec.graph);
            assert!(
                report.valid,
                "template '{}' should have valid graph, errors={:?}",
                spec.template_id, report.errors
            );
        }
    }

    #[test]
    fn validate_node_graph_reports_cycle() {
        let graph = NodeGraphFile {
            version: GRAPH_SCHEMA_VERSION,
            graph_name: "cycle_graph".to_string(),
            nodes: vec![
                NodeGraphNode {
                    id: "a".to_string(),
                    node_type: "OnStart".to_string(),
                    params: json!({}),
                },
                NodeGraphNode {
                    id: "b".to_string(),
                    node_type: "Sequence".to_string(),
                    params: json!({}),
                },
            ],
            edges: vec![
                NodeGraphEdge {
                    from: "a".to_string(),
                    to: "b".to_string(),
                    pin: "flow".to_string(),
                },
                NodeGraphEdge {
                    from: "b".to_string(),
                    to: "a".to_string(),
                    pin: "flow".to_string(),
                },
            ],
        };
        let report = validate_node_graph(&graph);
        assert!(!report.valid);
        assert!(report.errors.iter().any(|error| error.contains("cycle")));
    }

    #[test]
    fn validate_template_bundle_flags_missing_path_asset() {
        let bundle = TemplateAssetBundle {
            template_id: "test".to_string(),
            mesh_assets: vec!["cube".to_string()],
            material_assets: vec!["assets/materials/does_not_exist.json".to_string()],
            audio_assets: vec![],
        };
        let report = validate_template_bundle(&bundle, ".");
        assert!(!report.valid);
        assert_eq!(report.missing_assets.len(), 1);
    }
}
