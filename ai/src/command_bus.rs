use anyhow::{Context, bail};
use assets::{
    NodeGraphFile, NodeGraphValidationReport, SceneEntity, SceneFile,
    TemplateBundleValidationReport, builtin_template_spec, validate_node_graph,
    validate_template_bundle,
};
use ecs::{GraphEvent, GraphExecutionSummary, GraphSideEffect, SceneWorld, execute_runtime_graph};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::Digest;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct ValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
}

impl ValidationResult {
    pub fn ok() -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            valid: false,
            errors: vec![message.into()],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandResult {
    pub summary: String,
    pub payload: Value,
}

impl CommandResult {
    pub fn new(summary: impl Into<String>, payload: Value) -> Self {
        Self {
            summary: summary.into(),
            payload,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct CommandCostEstimate {
    pub cpu: u32,
    pub gpu: u32,
    pub mem_mb: u32,
    pub time_ms: u32,
}

impl Default for CommandCostEstimate {
    fn default() -> Self {
        Self {
            cpu: 1,
            gpu: 0,
            mem_mb: 1,
            time_ms: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CommandStatus {
    Completed,
    Failed,
    Canceled,
}

pub trait EngineCommand: Send {
    fn name(&self) -> &'static str;
    fn validate(&self, ctx: &CommandContext) -> ValidationResult;
    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult>;
    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()>;
    fn serialize(&self) -> Value;
    fn cost_estimate(&self) -> CommandCostEstimate {
        CommandCostEstimate::default()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ViewportCameraState {
    pub position: [f32; 3],
    pub target: [f32; 3],
    pub fov_y_deg: f32,
}

impl Default for ViewportCameraState {
    fn default() -> Self {
        Self {
            position: [0.0, 2.0, 4.0],
            target: [0.0, 0.0, 0.0],
            fov_y_deg: 60.0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineStateSnapshot {
    pub version: String,
    pub ai_mode: String,
    pub fps: f32,
    pub gpu_memory_mb: Option<u64>,
    pub system_memory_mb: Option<u64>,
    pub open_scene: Option<String>,
}

impl Default for EngineStateSnapshot {
    fn default() -> Self {
        Self {
            version: env!("CARGO_PKG_VERSION").to_string(),
            ai_mode: "OFF".to_string(),
            fps: 0.0,
            gpu_memory_mb: None,
            system_memory_mb: None,
            open_scene: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderSettings {
    pub light_direction: [f32; 3],
    pub light_color: [f32; 3],
    pub light_intensity: f32,
    pub shadow_bias: f32,
    pub shadow_strength: f32,
    pub shadow_cascade_count: u32,
    pub lod_transition_distances: [f32; 2],
    pub lod_hysteresis: f32,
    pub ibl_intensity: f32,
    pub ibl_sky_color: [f32; 3],
    pub ibl_ground_color: [f32; 3],
    pub exposure: f32,
    pub gamma: f32,
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    pub bloom_radius: f32,
    pub fog_density: f32,
    pub fog_color: [f32; 3],
    pub saturation: f32,
    pub contrast: f32,
    pub white_balance: f32,
    pub grade_tint: [f32; 3],
    pub color_grading_preset: String,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            light_direction: [-0.5, -1.0, -0.3],
            light_color: [1.0, 1.0, 1.0],
            light_intensity: 5.2,
            shadow_bias: 0.0018,
            shadow_strength: 1.0,
            shadow_cascade_count: 3,
            lod_transition_distances: [18.0, 42.0],
            lod_hysteresis: 3.0,
            ibl_intensity: 0.6,
            ibl_sky_color: [0.65, 0.75, 0.95],
            ibl_ground_color: [0.20, 0.18, 0.16],
            exposure: 1.0,
            gamma: 2.2,
            bloom_intensity: 0.08,
            bloom_threshold: 1.0,
            bloom_radius: 1.3,
            fog_density: 0.0,
            fog_color: [0.72, 0.76, 0.84],
            saturation: 1.0,
            contrast: 1.0,
            white_balance: 0.0,
            grade_tint: [1.0, 1.0, 1.0],
            color_grading_preset: "custom".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderPostprocessParams {
    pub exposure: f32,
    pub gamma: f32,
    pub bloom_intensity: f32,
    pub bloom_threshold: f32,
    pub bloom_radius: f32,
    pub fog_density: f32,
    pub fog_color: [f32; 3],
    pub saturation: f32,
    pub contrast: f32,
    pub white_balance: f32,
    pub grade_tint: [f32; 3],
    pub color_grading_preset: String,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct RenderCameraRecord {
    pub camera_id: String,
    pub position: [f32; 3],
    pub target: [f32; 3],
    pub fov_y_deg: f32,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct RenderCaptureRecord {
    pub capture_id: String,
    pub kind: String,
    pub path: String,
    pub params: Value,
    pub timestamp_utc: String,
}

#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct RenderRuntimeState {
    pub quality_preset: String,
    pub resolution: [u32; 2],
    pub resolution_scale: f32,
    pub hdr_enabled: bool,
    pub aa_mode: String,
    pub shadow_quality: Value,
    pub global_illumination_mode: String,
    pub raytracing_enabled: bool,
    pub cameras: HashMap<String, RenderCameraRecord>,
    pub active_camera_id: Option<String>,
    pub captures: Vec<RenderCaptureRecord>,
    pub material_params: HashMap<String, BTreeMap<String, Value>>,
    pub material_textures: HashMap<String, BTreeMap<String, String>>,
}

impl Default for RenderRuntimeState {
    fn default() -> Self {
        Self {
            quality_preset: "high".to_string(),
            resolution: [1280, 720],
            resolution_scale: 1.0,
            hdr_enabled: true,
            aa_mode: "taa".to_string(),
            shadow_quality: json!({
                "preset": "high",
                "cascade_count": 3,
                "resolution": 2048
            }),
            global_illumination_mode: "ssgi".to_string(),
            raytracing_enabled: false,
            cameras: HashMap::new(),
            active_camera_id: None,
            captures: Vec::new(),
            material_params: HashMap::new(),
            material_textures: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportedAssetRecord {
    pub asset_id: String,
    pub source_path: String,
    pub imported_path: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MaterialRecord {
    pub name: String,
    pub preset: String,
    pub params: Value,
    pub file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetTextureRecord {
    pub texture_id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub format: String,
    pub params: Value,
    pub file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetShaderRecord {
    pub shader_id: String,
    pub name: String,
    pub template: String,
    pub params: Value,
    pub file_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetPrefabRecord {
    pub prefab_id: String,
    pub name: String,
    pub source_entity_id: String,
    pub entity: SceneEntity,
    pub components: BTreeMap<String, Value>,
    pub metadata: Value,
    pub file_path: Option<String>,
    pub last_saved_utc: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetRebuildRecord {
    pub asset_id: String,
    pub params: Value,
    pub timestamp_utc: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetLodRecord {
    pub mesh_id: String,
    pub levels: u32,
    pub reduction: f32,
    pub params: Value,
    pub timestamp_utc: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetMeshOptimizationRecord {
    pub mesh_id: String,
    pub profile: String,
    pub params: Value,
    pub timestamp_utc: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetTextureCompressionRecord {
    pub asset_id: String,
    pub format: String,
    pub quality: String,
    pub params: Value,
    pub timestamp_utc: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssetBakeRecord {
    pub bake_id: String,
    pub params: Value,
    pub timestamp_utc: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct AssetPipelineRuntimeState {
    pub rebuilds: Vec<AssetRebuildRecord>,
    pub lods: HashMap<String, AssetLodRecord>,
    pub mesh_optimizations: HashMap<String, AssetMeshOptimizationRecord>,
    pub texture_compressions: HashMap<String, AssetTextureCompressionRecord>,
    pub lightmap_bakes: Vec<AssetBakeRecord>,
    pub reflection_probe_bakes: Vec<AssetBakeRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SceneFogSettings {
    pub density: f32,
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamChunkRecord {
    pub id: String,
    pub center: [f32; 3],
    pub radius: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorldStreamingSettings {
    pub enabled: bool,
    pub chunk_size: f32,
    pub range: u32,
    pub chunks: BTreeMap<String, StreamChunkRecord>,
    pub entity_to_chunk: HashMap<String, String>,
}

impl Default for WorldStreamingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            chunk_size: 64.0,
            range: 4,
            chunks: BTreeMap::new(),
            entity_to_chunk: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SceneRuntimeSettings {
    pub sky_preset: String,
    pub time_of_day: f32,
    pub fog: Option<SceneFogSettings>,
    pub world_streaming: WorldStreamingSettings,
    pub objective: Option<String>,
    pub last_message: Option<String>,
}

impl Default for SceneRuntimeSettings {
    fn default() -> Self {
        Self {
            sky_preset: "default_day".to_string(),
            time_of_day: 12.0,
            fog: None,
            world_streaming: WorldStreamingSettings::default(),
            objective: None,
            last_message: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct NodeGraphRuntimeState {
    pub active_template_id: Option<String>,
    pub graph: Option<NodeGraphFile>,
    pub validation: NodeGraphValidationReport,
    pub last_execution: Option<GraphExecutionSummary>,
    pub last_bundle_validation: Option<TemplateBundleValidationReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PhysicsCollider {
    pub shape: String,
    pub size: [f32; 3],
    pub is_trigger: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PhysicsRigidbody {
    pub body_type: String,
    pub mass: f32,
    pub friction: f32,
    pub restitution: f32,
    pub velocity: [f32; 3],
}

#[derive(Debug, Clone, Serialize)]
pub struct PhysicsRuntimeState {
    pub gravity: [f32; 3],
    pub colliders: HashMap<String, PhysicsCollider>,
    pub rigidbodies: HashMap<String, PhysicsRigidbody>,
    pub character_controllers: HashMap<String, PhysicsCharacterController>,
}

impl Default for PhysicsRuntimeState {
    fn default() -> Self {
        Self {
            gravity: [0.0, -9.81, 0.0],
            colliders: HashMap::new(),
            rigidbodies: HashMap::new(),
            character_controllers: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PhysicsCharacterController {
    pub radius: f32,
    pub height: f32,
    pub speed: f32,
    pub jump_strength: f32,
    pub grounded: bool,
    pub state: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameplayWeaponRecord {
    pub weapon_id: String,
    pub rate: f32,
    pub recoil: f32,
    pub spread: f32,
    pub ammo_current: u32,
    pub ammo_capacity: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameplayInputActionRecord {
    pub name: String,
    pub bindings: Vec<String>,
    pub target_event: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameplayTriggerRecord {
    pub entity_id: String,
    pub shape: String,
    pub radius: f32,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameplayPickupRecord {
    pub entity_id: String,
    pub item_data: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameplayInventoryRecord {
    pub entity_id: String,
    pub capacity: u32,
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameplayInteractableRecord {
    pub entity_id: String,
    pub prompt: String,
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnimationStateRecord {
    pub state_name: String,
    pub clip_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnimationTransitionRecord {
    pub from_state: String,
    pub to_state: String,
    pub conditions: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnimationStateMachineRecord {
    pub controller_id: String,
    pub name: String,
    pub states: BTreeMap<String, AnimationStateRecord>,
    pub transitions: Vec<AnimationTransitionRecord>,
    pub parameters: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnimationBlendRecord {
    pub clip_a: String,
    pub clip_b: String,
    pub weight: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnimationIkRecord {
    pub entity_id: String,
    pub chain: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnimationRetargetRecord {
    pub source_rig: String,
    pub target_rig: String,
    pub mapping: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnimationBakeRecord {
    pub entity_id: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct AnimationRuntimeState {
    pub state_machines: HashMap<String, AnimationStateMachineRecord>,
    pub entity_animators: HashMap<String, String>,
    pub entity_active_clips: HashMap<String, String>,
    pub entity_blends: HashMap<String, AnimationBlendRecord>,
    pub ik_solvers: HashMap<String, AnimationIkRecord>,
    pub retarget_jobs: Vec<AnimationRetargetRecord>,
    pub bake_jobs: Vec<AnimationBakeRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelMeshRecord {
    pub mesh_id: String,
    pub primitive_type: Option<String>,
    pub vertex_count: u32,
    pub face_count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelSelectionRecord {
    pub mode: String,
    pub selector: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelModifierRecord {
    pub modifier_id: String,
    pub modifier_type: String,
    pub params: Value,
    pub applied: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelUvRecord {
    pub method: String,
    pub packed: bool,
    pub lightmap_generated: bool,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelOperationRecord {
    pub tool: String,
    pub mesh_id: String,
    pub payload: Value,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ModelingRuntimeState {
    pub meshes: HashMap<String, ModelMeshRecord>,
    pub edit_modes: BTreeSet<String>,
    pub selections: HashMap<String, ModelSelectionRecord>,
    pub modifiers: HashMap<String, Vec<ModelModifierRecord>>,
    pub uv: HashMap<String, ModelUvRecord>,
    pub sculpt_masks: HashMap<String, Value>,
    pub operation_log: Vec<ModelOperationRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VfxParticleSystemRecord {
    pub particle_id: String,
    pub name: String,
    pub params: Value,
    pub emitter: Value,
    pub forces: Value,
    pub collision: Value,
    pub renderer: Value,
    pub attached_entity: Option<String>,
    pub socket: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VfxGraphNodeRecord {
    pub id: String,
    pub node_type: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct VfxGraphEdgeRecord {
    pub out_node: String,
    pub in_node: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct VfxGraphRecord {
    pub graph_id: String,
    pub name: String,
    pub nodes: Vec<VfxGraphNodeRecord>,
    pub edges: Vec<VfxGraphEdgeRecord>,
    pub compiled: bool,
    pub compile_report: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct VfxRuntimeState {
    pub particle_systems: HashMap<String, VfxParticleSystemRecord>,
    pub graphs: HashMap<String, VfxGraphRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WaterOceanRecord {
    pub ocean_id: String,
    pub size: f32,
    pub waves: Value,
    pub foam_enabled: bool,
    pub foam_params: Value,
    pub refraction_enabled: bool,
    pub refraction_params: Value,
    pub caustics_enabled: bool,
    pub caustics_params: Value,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct WaterRiverRecord {
    pub river_id: String,
    pub path: Vec<[f32; 3]>,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct WaterWaterfallRecord {
    pub waterfall_id: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct WaterRuntimeState {
    pub oceans: HashMap<String, WaterOceanRecord>,
    pub rivers: HashMap<String, WaterRiverRecord>,
    pub waterfalls: HashMap<String, WaterWaterfallRecord>,
    pub buoyancy: HashMap<String, Value>,
    pub drag: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MountHorseTemplateRecord {
    pub template_id: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct MountHorseRecord {
    pub horse_id: String,
    pub template_id: String,
    pub entity_id: String,
    pub rider_id: Option<String>,
    pub gait: String,
    pub path_follow: Option<String>,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct MountRuntimeState {
    pub horse_templates: HashMap<String, MountHorseTemplateRecord>,
    pub horses: HashMap<String, MountHorseRecord>,
    pub rider_to_horse: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NpcAiNavmeshRecord {
    pub navmesh_id: String,
    pub params: Value,
    pub baked: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct NpcAiAgentRecord {
    pub agent_id: String,
    pub entity_id: String,
    pub params: Value,
    pub destination: Option<[f32; 3]>,
    pub behavior_tree_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NpcAiBehaviorNodeRecord {
    pub node_id: String,
    pub node_type: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct NpcAiBehaviorEdgeRecord {
    pub parent: String,
    pub child: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NpcAiBehaviorTreeRecord {
    pub tree_id: String,
    pub name: String,
    pub nodes: Vec<NpcAiBehaviorNodeRecord>,
    pub edges: Vec<NpcAiBehaviorEdgeRecord>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct NpcAiRuntimeState {
    pub navmeshes: HashMap<String, NpcAiNavmeshRecord>,
    pub active_navmesh_id: Option<String>,
    pub agents: HashMap<String, NpcAiAgentRecord>,
    pub entity_agents: HashMap<String, String>,
    pub behavior_trees: HashMap<String, NpcAiBehaviorTreeRecord>,
    pub blackboard: HashMap<String, BTreeMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiCanvasRecord {
    pub canvas_id: String,
    pub name: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiElementRecord {
    pub ui_id: String,
    pub canvas_id: String,
    pub element_type: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiBindingRecord {
    pub ui_id: String,
    pub entity_id: String,
    pub component_field: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct UiRuntimeState {
    pub canvases: HashMap<String, UiCanvasRecord>,
    pub elements: HashMap<String, UiElementRecord>,
    pub bindings: HashMap<String, UiBindingRecord>,
    pub active_hud_template: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioClipRecord {
    pub clip_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioSourceRecord {
    pub source_id: String,
    pub entity_id: Option<String>,
    pub params: Value,
    pub spatial: Value,
    pub mixer_bus: Option<String>,
    pub playing_clip: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioMixerRecord {
    pub bus_id: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct AudioRuntimeState {
    pub clips: HashMap<String, AudioClipRecord>,
    pub sources: HashMap<String, AudioSourceRecord>,
    pub mixers: HashMap<String, AudioMixerRecord>,
    pub play_events: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkServerRecord {
    pub server_id: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkClientRecord {
    pub client_id: String,
    pub endpoint: String,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkingRuntimeState {
    pub server: Option<NetworkServerRecord>,
    pub clients: HashMap<String, NetworkClientRecord>,
    pub replication: HashMap<String, Vec<String>>,
    pub prediction_mode: String,
    pub rollback: Value,
}

impl Default for NetworkingRuntimeState {
    fn default() -> Self {
        Self {
            server: None,
            clients: HashMap::new(),
            replication: HashMap::new(),
            prediction_mode: "server".to_string(),
            rollback: json!({}),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BuildRuntimeState {
    pub target: String,
    pub bundle_id: Option<String>,
    pub version: String,
    pub enabled_features: BTreeSet<String>,
    pub last_export_path: Option<String>,
    pub last_installer_path: Option<String>,
}

impl Default for BuildRuntimeState {
    fn default() -> Self {
        Self {
            target: "windows".to_string(),
            bundle_id: None,
            version: "0.1.0".to_string(),
            enabled_features: BTreeSet::new(),
            last_export_path: None,
            last_installer_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DebugProfilerSnapshot {
    pub timestamp_utc: String,
    pub fps: f32,
    pub entity_count: usize,
    pub collider_count: usize,
    pub draw_call_estimate: usize,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct DebugRuntimeState {
    pub show_colliders: bool,
    pub show_navmesh: bool,
    pub wireframe: bool,
    pub captured_frames: u64,
    pub profiler_snapshots: Vec<DebugProfilerSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameplayRuntimeState {
    pub weapons: HashMap<String, GameplayWeaponRecord>,
    pub attachments: HashMap<String, String>,
    pub input_actions: HashMap<String, GameplayInputActionRecord>,
    pub triggers: HashMap<String, GameplayTriggerRecord>,
    pub pickups: HashMap<String, GameplayPickupRecord>,
    pub inventories: HashMap<String, GameplayInventoryRecord>,
    pub interactables: HashMap<String, GameplayInteractableRecord>,
    pub fire_events: u64,
    pub total_damage_applied: f32,
}

impl Default for GameplayRuntimeState {
    fn default() -> Self {
        Self {
            weapons: HashMap::new(),
            attachments: HashMap::new(),
            input_actions: HashMap::new(),
            triggers: HashMap::new(),
            pickups: HashMap::new(),
            inventories: HashMap::new(),
            interactables: HashMap::new(),
            fire_events: 0,
            total_damage_applied: 0.0,
        }
    }
}

pub struct CommandContext {
    pub project_root: PathBuf,
    pub open_scene_path: Option<PathBuf>,
    pub scene: SceneFile,
    pub scene_runtime: SceneRuntimeSettings,
    pub node_graph: NodeGraphRuntimeState,
    pub physics: PhysicsRuntimeState,
    pub gameplay: GameplayRuntimeState,
    pub animation: AnimationRuntimeState,
    pub modeling: ModelingRuntimeState,
    pub vfx: VfxRuntimeState,
    pub water: WaterRuntimeState,
    pub mount: MountRuntimeState,
    pub npc_ai: NpcAiRuntimeState,
    pub ui: UiRuntimeState,
    pub audio: AudioRuntimeState,
    pub networking: NetworkingRuntimeState,
    pub build: BuildRuntimeState,
    pub debug: DebugRuntimeState,
    pub runtime_world: SceneWorld,
    pub selection: Vec<String>,
    pub components: HashMap<String, BTreeMap<String, Value>>,
    pub imported_assets: BTreeMap<String, ImportedAssetRecord>,
    pub materials: BTreeMap<String, MaterialRecord>,
    pub textures: BTreeMap<String, AssetTextureRecord>,
    pub shaders: BTreeMap<String, AssetShaderRecord>,
    pub prefabs: BTreeMap<String, AssetPrefabRecord>,
    pub asset_pipeline: AssetPipelineRuntimeState,
    pub render_settings: RenderSettings,
    pub viewport_camera: ViewportCameraState,
    pub engine_state: EngineStateSnapshot,
    pub revision: u64,
}

impl CommandContext {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        let scene = SceneFile::default();
        let runtime_world = SceneWorld::from_scene(&scene);
        let engine_state = EngineStateSnapshot {
            open_scene: Some(scene.name.clone()),
            ..EngineStateSnapshot::default()
        };
        Self {
            project_root: project_root.into(),
            open_scene_path: None,
            scene,
            scene_runtime: SceneRuntimeSettings::default(),
            node_graph: NodeGraphRuntimeState::default(),
            physics: PhysicsRuntimeState::default(),
            gameplay: GameplayRuntimeState::default(),
            animation: AnimationRuntimeState::default(),
            modeling: ModelingRuntimeState::default(),
            vfx: VfxRuntimeState::default(),
            water: WaterRuntimeState::default(),
            mount: MountRuntimeState::default(),
            npc_ai: NpcAiRuntimeState::default(),
            ui: UiRuntimeState::default(),
            audio: AudioRuntimeState::default(),
            networking: NetworkingRuntimeState::default(),
            build: BuildRuntimeState::default(),
            debug: DebugRuntimeState::default(),
            runtime_world,
            selection: Vec::new(),
            components: HashMap::new(),
            imported_assets: BTreeMap::new(),
            materials: BTreeMap::new(),
            textures: BTreeMap::new(),
            shaders: BTreeMap::new(),
            prefabs: BTreeMap::new(),
            asset_pipeline: AssetPipelineRuntimeState::default(),
            render_settings: RenderSettings::default(),
            viewport_camera: ViewportCameraState::default(),
            engine_state,
            revision: 0,
        }
    }

    pub fn open_scene_label(&self) -> Option<String> {
        self.open_scene_path
            .as_ref()
            .map(|path| path.display().to_string())
            .or_else(|| {
                if self.scene.name.trim().is_empty() {
                    None
                } else {
                    Some(self.scene.name.clone())
                }
            })
    }

    pub fn set_ai_mode(&mut self, mode: impl Into<String>) {
        self.engine_state.ai_mode = mode.into();
    }

    pub fn set_fps(&mut self, fps: f32) {
        self.engine_state.fps = fps.max(0.0);
    }

    pub fn set_open_scene_path(&mut self, path: Option<PathBuf>) {
        self.open_scene_path = path;
        self.sync_open_scene_name();
    }

    pub fn sync_open_scene_name(&mut self) {
        self.engine_state.open_scene = self.open_scene_label();
    }

    pub fn reset_scene_runtime_state(&mut self) {
        self.scene_runtime = SceneRuntimeSettings::default();
        self.physics = PhysicsRuntimeState::default();
        self.gameplay = GameplayRuntimeState::default();
        self.animation = AnimationRuntimeState::default();
        self.modeling = ModelingRuntimeState::default();
        self.vfx = VfxRuntimeState::default();
        self.water = WaterRuntimeState::default();
        self.mount = MountRuntimeState::default();
        self.npc_ai = NpcAiRuntimeState::default();
        self.ui = UiRuntimeState::default();
        self.audio = AudioRuntimeState::default();
        self.networking = NetworkingRuntimeState::default();
        self.build = BuildRuntimeState::default();
        self.debug = DebugRuntimeState::default();
    }

    pub fn entity_exists(&self, name: &str) -> bool {
        self.runtime_world.has_entity(name)
    }

    pub fn entity_transform(&self, name: &str) -> Option<[f32; 3]> {
        self.runtime_world.get_transform(name)
    }

    pub fn ecs_entity_count(&self) -> usize {
        self.runtime_world.entity_count()
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn replace_scene_from_editor(
        &mut self,
        scene: SceneFile,
        open_scene_path: Option<PathBuf>,
    ) -> anyhow::Result<()> {
        self.scene = scene;
        self.open_scene_path = open_scene_path;
        self.rebuild_runtime_world()
    }

    pub fn rebuild_runtime_world(&mut self) -> anyhow::Result<()> {
        self.runtime_world.rebuild_from_scene(&self.scene);
        self.components
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.selection
            .retain(|entity_name| self.runtime_world.has_entity(entity_name));

        let component_snapshot = self
            .components
            .iter()
            .map(|(entity_name, bucket)| {
                (
                    entity_name.clone(),
                    bucket
                        .iter()
                        .map(|(component_type, data)| (component_type.clone(), data.clone()))
                        .collect::<Vec<(String, Value)>>(),
                )
            })
            .collect::<Vec<(String, Vec<(String, Value)>)>>();

        for (entity_name, bucket) in component_snapshot {
            for (component_type, data) in bucket {
                self.runtime_world
                    .upsert_dynamic_component(&entity_name, &component_type, data)
                    .with_context(|| {
                        format!(
                            "failed to sync component '{}' for entity '{}' into ECS world",
                            component_type, entity_name
                        )
                    })?;
            }
        }

        self.scene_runtime
            .world_streaming
            .entity_to_chunk
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.physics
            .colliders
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.physics
            .rigidbodies
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.physics
            .character_controllers
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.gameplay
            .attachments
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.gameplay
            .attachments
            .retain(|_, weapon_id| self.gameplay.weapons.contains_key(weapon_id));
        self.gameplay
            .triggers
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.gameplay
            .pickups
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.gameplay
            .inventories
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.gameplay
            .interactables
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.animation
            .entity_animators
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.animation
            .entity_animators
            .retain(|_, controller_id| self.animation.state_machines.contains_key(controller_id));
        self.animation
            .entity_active_clips
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.animation
            .entity_blends
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.animation
            .ik_solvers
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.animation
            .bake_jobs
            .retain(|job| self.runtime_world.has_entity(&job.entity_id));
        let known_chunks = self
            .scene_runtime
            .world_streaming
            .chunks
            .keys()
            .cloned()
            .collect::<BTreeSet<String>>();
        self.scene_runtime
            .world_streaming
            .entity_to_chunk
            .retain(|_, chunk_id| known_chunks.contains(chunk_id));

        let known_meshes = self
            .scene
            .entities
            .iter()
            .map(|entity| entity.mesh.clone())
            .collect::<BTreeSet<String>>();
        self.modeling
            .meshes
            .retain(|mesh_id, _| known_meshes.contains(mesh_id));
        self.modeling
            .edit_modes
            .retain(|mesh_id| self.modeling.meshes.contains_key(mesh_id));
        self.modeling
            .selections
            .retain(|mesh_id, _| self.modeling.meshes.contains_key(mesh_id));
        self.modeling
            .modifiers
            .retain(|mesh_id, _| self.modeling.meshes.contains_key(mesh_id));
        self.modeling
            .uv
            .retain(|mesh_id, _| self.modeling.meshes.contains_key(mesh_id));
        self.modeling
            .sculpt_masks
            .retain(|mesh_id, _| self.modeling.meshes.contains_key(mesh_id));
        self.asset_pipeline
            .lods
            .retain(|mesh_id, _| known_meshes.contains(mesh_id));
        self.asset_pipeline
            .mesh_optimizations
            .retain(|mesh_id, _| known_meshes.contains(mesh_id));
        self.asset_pipeline
            .texture_compressions
            .retain(|asset_id, _| {
                self.imported_assets.contains_key(asset_id) || self.textures.contains_key(asset_id)
            });
        for particle in self.vfx.particle_systems.values_mut() {
            if particle
                .attached_entity
                .as_ref()
                .map(|entity_id| !self.runtime_world.has_entity(entity_id))
                .unwrap_or(false)
            {
                particle.attached_entity = None;
                particle.socket = None;
            }
        }
        self.water
            .buoyancy
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.water
            .drag
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        self.mount
            .horses
            .retain(|_, horse| self.runtime_world.has_entity(&horse.entity_id));
        let mounted_pairs = self
            .mount
            .horses
            .iter_mut()
            .filter_map(|(horse_id, horse)| {
                let rider_valid = horse
                    .rider_id
                    .as_ref()
                    .map(|rider_id| self.runtime_world.has_entity(rider_id))
                    .unwrap_or(false);
                if !rider_valid {
                    horse.rider_id = None;
                    return None;
                }
                horse
                    .rider_id
                    .as_ref()
                    .map(|rider_id| (rider_id.clone(), horse_id.clone()))
            })
            .collect::<Vec<(String, String)>>();
        self.mount.rider_to_horse.clear();
        for (rider_id, horse_id) in mounted_pairs {
            self.mount.rider_to_horse.insert(rider_id, horse_id);
        }
        self.npc_ai
            .agents
            .retain(|_, agent| self.runtime_world.has_entity(&agent.entity_id));
        for agent in self.npc_ai.agents.values_mut() {
            if agent
                .behavior_tree_id
                .as_ref()
                .map(|tree_id| !self.npc_ai.behavior_trees.contains_key(tree_id))
                .unwrap_or(false)
            {
                agent.behavior_tree_id = None;
            }
        }
        self.npc_ai.entity_agents.clear();
        for (agent_id, agent) in &self.npc_ai.agents {
            self.npc_ai
                .entity_agents
                .insert(agent.entity_id.clone(), agent_id.clone());
        }
        self.npc_ai
            .blackboard
            .retain(|entity_name, _| self.runtime_world.has_entity(entity_name));
        let known_canvases = self
            .ui
            .canvases
            .keys()
            .cloned()
            .collect::<BTreeSet<String>>();
        self.ui
            .elements
            .retain(|_, element| known_canvases.contains(&element.canvas_id));
        let known_ui_elements = self
            .ui
            .elements
            .keys()
            .cloned()
            .collect::<BTreeSet<String>>();
        self.ui.bindings.retain(|ui_id, binding| {
            known_ui_elements.contains(ui_id) && self.runtime_world.has_entity(&binding.entity_id)
        });
        let known_mixers = self
            .audio
            .mixers
            .keys()
            .cloned()
            .collect::<BTreeSet<String>>();
        let known_clips = self
            .audio
            .clips
            .keys()
            .cloned()
            .collect::<BTreeSet<String>>();
        for source in self.audio.sources.values_mut() {
            if source
                .entity_id
                .as_ref()
                .map(|entity_id| !self.runtime_world.has_entity(entity_id))
                .unwrap_or(false)
            {
                source.entity_id = None;
            }
            if source
                .mixer_bus
                .as_ref()
                .map(|bus_id| !known_mixers.contains(bus_id))
                .unwrap_or(false)
            {
                source.mixer_bus = None;
            }
            if source
                .playing_clip
                .as_ref()
                .map(|clip_id| !known_clips.contains(clip_id))
                .unwrap_or(false)
            {
                source.playing_clip = None;
            }
        }
        self.networking
            .replication
            .retain(|entity_name, components| {
                self.runtime_world.has_entity(entity_name) && !components.is_empty()
            });

        self.sync_open_scene_name();
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CommandReceipt {
    pub command_id: u64,
    pub status: CommandStatus,
    pub result: CommandResult,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReplayEntry {
    pub command_id: u64,
    pub command_name: String,
    pub serialized: Value,
    pub status: CommandStatus,
    pub result_summary: String,
    pub cost: CommandCostEstimate,
}

struct RecordedCommand {
    id: u64,
    name: String,
    serialized: Value,
    cost: CommandCostEstimate,
    command: Box<dyn EngineCommand>,
    result: CommandResult,
}

struct ActiveTransaction {
    name: String,
    commands: Vec<RecordedCommand>,
    checkpoints: HashMap<String, usize>,
}

pub struct CommandBus {
    context: CommandContext,
    history: Vec<RecordedCommand>,
    redo_stack: Vec<RecordedCommand>,
    history_marks: HashMap<String, usize>,
    active_txn: Option<ActiveTransaction>,
    statuses: HashMap<u64, CommandStatus>,
    replay_log: Vec<ReplayEntry>,
    next_id: u64,
}

impl CommandBus {
    pub fn new(context: CommandContext) -> Self {
        Self {
            context,
            history: Vec::new(),
            redo_stack: Vec::new(),
            history_marks: HashMap::new(),
            active_txn: None,
            statuses: HashMap::new(),
            replay_log: Vec::new(),
            next_id: 1,
        }
    }

    pub fn context(&self) -> &CommandContext {
        &self.context
    }

    pub fn context_mut(&mut self) -> &mut CommandContext {
        &mut self.context
    }

    pub fn scene_snapshot(&self) -> SceneFile {
        self.context.scene.clone()
    }

    pub fn scene_revision(&self) -> u64 {
        self.context.revision()
    }

    pub fn render_settings(&self) -> RenderSettings {
        self.context.render_settings.clone()
    }

    pub fn replace_scene_from_editor(
        &mut self,
        scene: SceneFile,
        open_scene_path: Option<PathBuf>,
    ) -> anyhow::Result<()> {
        self.context
            .replace_scene_from_editor(scene, open_scene_path)
    }

    pub fn set_ai_mode(&mut self, mode: &str) {
        self.context.set_ai_mode(mode.to_string());
    }

    pub fn set_frame_stats(&mut self, fps: f32) {
        self.context.set_fps(fps);
    }

    pub fn submit(
        &mut self,
        mut command: Box<dyn EngineCommand>,
    ) -> anyhow::Result<CommandReceipt> {
        let command_id = self.next_id;
        self.next_id += 1;

        let command_name = command.name().to_string();
        let serialized = command.serialize();
        let cost = command.cost_estimate();
        let validation = command.validate(&self.context);
        if !validation.valid {
            let summary = if validation.errors.is_empty() {
                "validation failed".to_string()
            } else {
                validation.errors.join("; ")
            };
            self.statuses.insert(command_id, CommandStatus::Failed);
            self.replay_log.push(ReplayEntry {
                command_id,
                command_name: command_name.clone(),
                serialized,
                status: CommandStatus::Failed,
                result_summary: summary.clone(),
                cost,
            });
            bail!("command '{}' validation failed: {}", command_name, summary);
        }

        let result = match command.execute(&mut self.context) {
            Ok(result) => result,
            Err(err) => {
                let summary = err.to_string();
                self.statuses.insert(command_id, CommandStatus::Failed);
                self.replay_log.push(ReplayEntry {
                    command_id,
                    command_name: command_name.clone(),
                    serialized,
                    status: CommandStatus::Failed,
                    result_summary: summary.clone(),
                    cost,
                });
                bail!("command '{}' failed: {}", command_name, summary);
            }
        };
        self.context
            .rebuild_runtime_world()
            .context("failed to sync ECS world after command execution")?;

        let replay_summary = result.summary.clone();
        let receipt = CommandReceipt {
            command_id,
            status: CommandStatus::Completed,
            result: result.clone(),
        };
        let recorded = RecordedCommand {
            id: command_id,
            name: command_name.clone(),
            serialized: serialized.clone(),
            cost,
            command,
            result,
        };

        if let Some(active_txn) = &mut self.active_txn {
            active_txn.commands.push(recorded);
        } else {
            self.history.push(recorded);
            self.redo_stack.clear();
        }
        self.statuses.insert(command_id, CommandStatus::Completed);
        self.replay_log.push(ReplayEntry {
            command_id,
            command_name,
            serialized,
            status: CommandStatus::Completed,
            result_summary: replay_summary,
            cost,
        });

        Ok(receipt)
    }

    pub fn submit_batch(
        &mut self,
        commands: Vec<Box<dyn EngineCommand>>,
    ) -> anyhow::Result<Vec<CommandReceipt>> {
        let mut receipts = Vec::with_capacity(commands.len());
        for command in commands {
            receipts.push(self.submit(command)?);
        }
        Ok(receipts)
    }

    pub fn cancel(&mut self, command_id: u64) -> bool {
        if let Some(status) = self.statuses.get(&command_id)
            && (*status == CommandStatus::Completed || *status == CommandStatus::Failed)
        {
            return false;
        }
        self.statuses.insert(command_id, CommandStatus::Canceled);
        true
    }

    pub fn get_status(&self, command_id: u64) -> Option<CommandStatus> {
        self.statuses.get(&command_id).copied()
    }

    pub fn replay(&self, command_id: u64) -> Option<ReplayEntry> {
        self.replay_log
            .iter()
            .rev()
            .find(|entry| entry.command_id == command_id)
            .cloned()
    }

    pub fn replay_log(&self) -> &[ReplayEntry] {
        &self.replay_log
    }

    pub fn begin_transaction(&mut self, name: impl Into<String>) -> anyhow::Result<()> {
        if self.active_txn.is_some() {
            bail!("a transaction is already active");
        }
        self.active_txn = Some(ActiveTransaction {
            name: name.into(),
            commands: Vec::new(),
            checkpoints: HashMap::new(),
        });
        Ok(())
    }

    pub fn commit_transaction(&mut self) -> anyhow::Result<usize> {
        let active_txn = self
            .active_txn
            .take()
            .context("no active transaction to commit")?;
        let committed_count = active_txn.commands.len();
        self.history.extend(active_txn.commands);
        self.redo_stack.clear();
        Ok(committed_count)
    }

    pub fn rollback_transaction(&mut self) -> anyhow::Result<usize> {
        let mut active_txn = self
            .active_txn
            .take()
            .context("no active transaction to rollback")?;
        let mut rolled_back = 0usize;
        while let Some(mut recorded) = active_txn.commands.pop() {
            recorded.command.undo(&mut self.context).with_context(|| {
                format!(
                    "failed to undo command '{}' during transaction rollback",
                    recorded.name
                )
            })?;
            self.statuses.insert(recorded.id, CommandStatus::Canceled);
            self.replay_log.push(ReplayEntry {
                command_id: recorded.id,
                command_name: recorded.name,
                serialized: recorded.serialized,
                status: CommandStatus::Canceled,
                result_summary: "transaction rollback".to_string(),
                cost: recorded.cost,
            });
            rolled_back += 1;
        }
        self.context
            .rebuild_runtime_world()
            .context("failed to sync ECS world after transaction rollback")?;
        Ok(rolled_back)
    }

    pub fn transaction_checkpoint(&mut self, label: impl Into<String>) -> anyhow::Result<()> {
        let active_txn = self
            .active_txn
            .as_mut()
            .context("no active transaction for checkpoint")?;
        active_txn
            .checkpoints
            .insert(label.into(), active_txn.commands.len());
        Ok(())
    }

    pub fn transaction_rollback_to(&mut self, label: &str) -> anyhow::Result<usize> {
        let rollback_index = {
            let active_txn = self
                .active_txn
                .as_ref()
                .context("no active transaction for rollback_to")?;
            *active_txn
                .checkpoints
                .get(label)
                .with_context(|| format!("checkpoint '{}' not found", label))?
        };

        let active_txn = self
            .active_txn
            .as_mut()
            .context("no active transaction for rollback_to")?;
        let mut rolled_back = 0usize;
        while active_txn.commands.len() > rollback_index {
            if let Some(mut recorded) = active_txn.commands.pop() {
                recorded.command.undo(&mut self.context).with_context(|| {
                    format!(
                        "failed to undo command '{}' during rollback_to checkpoint '{}'",
                        recorded.name, label
                    )
                })?;
                self.statuses.insert(recorded.id, CommandStatus::Canceled);
                self.replay_log.push(ReplayEntry {
                    command_id: recorded.id,
                    command_name: recorded.name,
                    serialized: recorded.serialized,
                    status: CommandStatus::Canceled,
                    result_summary: format!("rollback_to checkpoint '{}'", label),
                    cost: recorded.cost,
                });
                rolled_back += 1;
            }
        }
        self.context
            .rebuild_runtime_world()
            .context("failed to sync ECS world after checkpoint rollback")?;
        Ok(rolled_back)
    }

    pub fn current_transaction_name(&self) -> Option<&str> {
        self.active_txn.as_ref().map(|txn| txn.name.as_str())
    }

    pub fn history_undo(&mut self, steps: usize) -> anyhow::Result<usize> {
        if self.active_txn.is_some() {
            bail!("undo is blocked while a transaction is active");
        }
        let mut undone = 0usize;
        for _ in 0..steps {
            let Some(mut recorded) = self.history.pop() else {
                break;
            };
            recorded.command.undo(&mut self.context).with_context(|| {
                format!("failed to undo command '{}' from history", recorded.name)
            })?;
            self.statuses.insert(recorded.id, CommandStatus::Canceled);
            self.replay_log.push(ReplayEntry {
                command_id: recorded.id,
                command_name: recorded.name.clone(),
                serialized: recorded.serialized.clone(),
                status: CommandStatus::Canceled,
                result_summary: "history undo".to_string(),
                cost: recorded.cost,
            });
            self.redo_stack.push(recorded);
            undone += 1;
        }
        self.context
            .rebuild_runtime_world()
            .context("failed to sync ECS world after history undo")?;
        Ok(undone)
    }

    pub fn history_redo(&mut self, steps: usize) -> anyhow::Result<usize> {
        if self.active_txn.is_some() {
            bail!("redo is blocked while a transaction is active");
        }
        let mut redone = 0usize;
        for _ in 0..steps {
            let Some(mut recorded) = self.redo_stack.pop() else {
                break;
            };
            let redo_result = recorded
                .command
                .execute(&mut self.context)
                .with_context(|| format!("failed to redo command '{}'", recorded.name))?;
            recorded.result = redo_result.clone();
            self.statuses.insert(recorded.id, CommandStatus::Completed);
            self.replay_log.push(ReplayEntry {
                command_id: recorded.id,
                command_name: recorded.name.clone(),
                serialized: recorded.serialized.clone(),
                status: CommandStatus::Completed,
                result_summary: format!("history redo: {}", redo_result.summary),
                cost: recorded.cost,
            });
            self.history.push(recorded);
            redone += 1;
        }
        self.context
            .rebuild_runtime_world()
            .context("failed to sync ECS world after history redo")?;
        Ok(redone)
    }

    pub fn history_mark(&mut self, label: impl Into<String>) {
        self.history_marks.insert(label.into(), self.history.len());
    }

    pub fn history_jump_to(&mut self, label: &str) -> anyhow::Result<()> {
        let target_index = *self
            .history_marks
            .get(label)
            .with_context(|| format!("history mark '{}' not found", label))?;
        let current_index = self.history.len();
        if target_index < current_index {
            let steps = current_index - target_index;
            self.history_undo(steps)?;
        } else if target_index > current_index {
            let steps = target_index - current_index;
            if steps > self.redo_stack.len() {
                bail!(
                    "cannot jump to '{}' because redo depth {} is insufficient for {} step(s)",
                    label,
                    self.redo_stack.len(),
                    steps
                );
            }
            self.history_redo(steps)?;
        }
        Ok(())
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    pub fn redo_len(&self) -> usize {
        self.redo_stack.len()
    }
}

pub fn resolve_project_path(project_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_root.join(path)
    }
}

#[derive(Debug, Clone)]
pub struct SceneCreateCommand {
    name: String,
    previous_scene: Option<SceneFile>,
    previous_scene_path: Option<PathBuf>,
    previous_scene_runtime: Option<SceneRuntimeSettings>,
}

impl SceneCreateCommand {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            previous_scene: None,
            previous_scene_path: None,
            previous_scene_runtime: None,
        }
    }
}

impl EngineCommand for SceneCreateCommand {
    fn name(&self) -> &'static str {
        "scene.create"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.name.trim().is_empty() {
            ValidationResult::invalid("scene name cannot be empty")
        } else {
            ValidationResult::ok()
        }
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_scene = Some(ctx.scene.clone());
        self.previous_scene_path = ctx.open_scene_path.clone();
        self.previous_scene_runtime = Some(ctx.scene_runtime.clone());
        ctx.scene = SceneFile {
            name: self.name.clone(),
            entities: Vec::new(),
        };
        ctx.reset_scene_runtime_state();
        ctx.set_open_scene_path(None);
        Ok(CommandResult::new(
            format!("scene '{}' created", self.name),
            json!({
                "name": self.name,
                "entity_count": 0
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_scene) = &self.previous_scene {
            ctx.scene = previous_scene.clone();
            ctx.set_open_scene_path(self.previous_scene_path.clone());
            if let Some(previous_scene_runtime) = &self.previous_scene_runtime {
                ctx.scene_runtime = previous_scene_runtime.clone();
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "name": self.name
        })
    }
}

#[derive(Debug, Clone)]
pub struct SceneOpenCommand {
    path: PathBuf,
    previous_scene: Option<SceneFile>,
    previous_scene_path: Option<PathBuf>,
    previous_scene_runtime: Option<SceneRuntimeSettings>,
}

impl SceneOpenCommand {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            previous_scene: None,
            previous_scene_path: None,
            previous_scene_runtime: None,
        }
    }
}

impl EngineCommand for SceneOpenCommand {
    fn name(&self) -> &'static str {
        "scene.open"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.path.as_os_str().is_empty() {
            ValidationResult::invalid("scene path cannot be empty")
        } else {
            ValidationResult::ok()
        }
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let scene_path = resolve_project_path(&ctx.project_root, &self.path);
        let scene = assets::load_scene(&scene_path)
            .with_context(|| format!("failed to open scene '{}'", scene_path.display()))?;
        self.previous_scene = Some(ctx.scene.clone());
        self.previous_scene_path = ctx.open_scene_path.clone();
        self.previous_scene_runtime = Some(ctx.scene_runtime.clone());
        ctx.scene = scene.clone();
        ctx.reset_scene_runtime_state();
        ctx.set_open_scene_path(Some(scene_path.clone()));
        Ok(CommandResult::new(
            format!("scene '{}' opened", scene.name),
            json!({
                "path": scene_path.display().to_string(),
                "name": scene.name,
                "entity_count": scene.entities.len()
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_scene) = &self.previous_scene {
            ctx.scene = previous_scene.clone();
            ctx.set_open_scene_path(self.previous_scene_path.clone());
            if let Some(previous_scene_runtime) = &self.previous_scene_runtime {
                ctx.scene_runtime = previous_scene_runtime.clone();
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "path": self.path
        })
    }
}

#[derive(Debug, Clone)]
pub struct SceneSaveCommand {
    path: Option<PathBuf>,
    previous_scene_path: Option<PathBuf>,
    previous_contents: Option<String>,
    target_path: Option<PathBuf>,
}

impl SceneSaveCommand {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            previous_scene_path: None,
            previous_contents: None,
            target_path: None,
        }
    }
}

impl EngineCommand for SceneSaveCommand {
    fn name(&self) -> &'static str {
        "scene.save"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.path.is_none() && ctx.open_scene_path.is_none() {
            return ValidationResult::invalid(
                "scene.save requires an explicit path when no scene is currently open from file",
            );
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let target_path = if let Some(path) = &self.path {
            resolve_project_path(&ctx.project_root, path)
        } else {
            ctx.open_scene_path
                .clone()
                .context("missing open scene path for scene.save")?
        };

        self.previous_scene_path = ctx.open_scene_path.clone();
        self.previous_contents = fs::read_to_string(&target_path).ok();
        self.target_path = Some(target_path.clone());

        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create parent directory '{}' for scene.save",
                    parent.display()
                )
            })?;
        }

        let serialized = serde_json::to_string_pretty(&ctx.scene)
            .context("failed to serialize scene during scene.save")?;
        fs::write(&target_path, serialized)
            .with_context(|| format!("failed to write scene to '{}'", target_path.display()))?;
        ctx.set_open_scene_path(Some(target_path.clone()));
        Ok(CommandResult::new(
            "scene saved",
            json!({
                "path": target_path.display().to_string(),
                "name": ctx.scene.name,
                "entity_count": ctx.scene.entities.len()
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        let target_path = self
            .target_path
            .as_ref()
            .context("scene.save undo missing target path")?;

        match &self.previous_contents {
            Some(previous_contents) => {
                fs::write(target_path, previous_contents).with_context(|| {
                    format!(
                        "failed to restore previous scene contents for '{}'",
                        target_path.display()
                    )
                })?;
            }
            None => {
                if target_path.exists() {
                    fs::remove_file(target_path).with_context(|| {
                        format!(
                            "failed to remove scene file '{}' during undo",
                            target_path.display()
                        )
                    })?;
                }
            }
        }

        ctx.set_open_scene_path(self.previous_scene_path.clone());
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "path": self.path
        })
    }
}

#[derive(Debug, Clone)]
pub struct SceneDuplicateCommand {
    source_path: PathBuf,
    target_path: PathBuf,
    target_scene_name: Option<String>,
    previous_target_contents: Option<String>,
}

impl SceneDuplicateCommand {
    pub fn new(
        source_path: impl Into<PathBuf>,
        target_path: impl Into<PathBuf>,
        target_scene_name: Option<String>,
    ) -> Self {
        Self {
            source_path: source_path.into(),
            target_path: target_path.into(),
            target_scene_name,
            previous_target_contents: None,
        }
    }
}

impl EngineCommand for SceneDuplicateCommand {
    fn name(&self) -> &'static str {
        "scene.duplicate"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.source_path.as_os_str().is_empty() {
            return ValidationResult::invalid("source scene path cannot be empty");
        }
        if self.target_path.as_os_str().is_empty() {
            return ValidationResult::invalid("target scene path cannot be empty");
        }
        if self.source_path == self.target_path {
            return ValidationResult::invalid("source and target scene paths must be different");
        }
        if let Some(name) = &self.target_scene_name
            && name.trim().is_empty()
        {
            return ValidationResult::invalid("target_scene_name cannot be empty");
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let source_path = resolve_project_path(&ctx.project_root, &self.source_path);
        let target_path = resolve_project_path(&ctx.project_root, &self.target_path);
        if source_path == target_path {
            bail!("source and target scene paths must be different");
        }

        let mut scene = assets::load_scene(&source_path).with_context(|| {
            format!(
                "failed to open source scene '{}' for duplication",
                source_path.display()
            )
        })?;
        if let Some(target_scene_name) = &self.target_scene_name {
            scene.name = target_scene_name.trim().to_string();
        }

        self.previous_target_contents = fs::read_to_string(&target_path).ok();
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create parent directory '{}' for scene.duplicate",
                    parent.display()
                )
            })?;
        }
        let serialized = serde_json::to_string_pretty(&scene)
            .context("failed to serialize duplicated scene json")?;
        fs::write(&target_path, serialized).with_context(|| {
            format!(
                "failed to write duplicated scene to '{}'",
                target_path.display()
            )
        })?;

        Ok(CommandResult::new(
            "scene duplicated",
            json!({
                "source_path": source_path.display().to_string(),
                "target_path": target_path.display().to_string(),
                "name": scene.name,
                "entity_count": scene.entities.len()
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        let target_path = resolve_project_path(&ctx.project_root, &self.target_path);
        match &self.previous_target_contents {
            Some(previous_contents) => {
                fs::write(&target_path, previous_contents).with_context(|| {
                    format!(
                        "failed to restore previous target scene '{}' during undo",
                        target_path.display()
                    )
                })?;
            }
            None => {
                if target_path.exists() {
                    fs::remove_file(&target_path).with_context(|| {
                        format!(
                            "failed to remove duplicated scene '{}' during undo",
                            target_path.display()
                        )
                    })?;
                }
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "source_path": self.source_path,
            "target_path": self.target_path,
            "target_scene_name": self.target_scene_name
        })
    }
}

#[derive(Debug, Clone)]
pub struct SceneCloseCommand {
    previous_scene: Option<SceneFile>,
    previous_scene_path: Option<PathBuf>,
    previous_scene_runtime: Option<SceneRuntimeSettings>,
}

impl Default for SceneCloseCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneCloseCommand {
    pub fn new() -> Self {
        Self {
            previous_scene: None,
            previous_scene_path: None,
            previous_scene_runtime: None,
        }
    }
}

impl EngineCommand for SceneCloseCommand {
    fn name(&self) -> &'static str {
        "scene.close"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_scene = Some(ctx.scene.clone());
        self.previous_scene_path = ctx.open_scene_path.clone();
        self.previous_scene_runtime = Some(ctx.scene_runtime.clone());
        ctx.scene = SceneFile::default();
        ctx.reset_scene_runtime_state();
        ctx.set_open_scene_path(None);
        Ok(CommandResult::new(
            "scene closed",
            json!({
                "name": ctx.scene.name,
                "entity_count": ctx.scene.entities.len()
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_scene) = &self.previous_scene {
            ctx.scene = previous_scene.clone();
            ctx.set_open_scene_path(self.previous_scene_path.clone());
        }
        if let Some(previous_scene_runtime) = &self.previous_scene_runtime {
            ctx.scene_runtime = previous_scene_runtime.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({})
    }
}

#[derive(Debug, Clone)]
pub struct SceneSetSkyCommand {
    preset: String,
    previous_preset: Option<String>,
}

impl SceneSetSkyCommand {
    pub fn new(preset: impl Into<String>) -> Self {
        Self {
            preset: preset.into(),
            previous_preset: None,
        }
    }
}

impl EngineCommand for SceneSetSkyCommand {
    fn name(&self) -> &'static str {
        "scene.set_sky"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.preset.trim().is_empty() {
            ValidationResult::invalid("preset cannot be empty")
        } else {
            ValidationResult::ok()
        }
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_preset = Some(ctx.scene_runtime.sky_preset.clone());
        ctx.scene_runtime.sky_preset = self.preset.clone();
        Ok(CommandResult::new(
            format!("sky preset set to '{}'", self.preset),
            json!({
                "sky_preset": ctx.scene_runtime.sky_preset
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_preset) = &self.previous_preset {
            ctx.scene_runtime.sky_preset = previous_preset.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "preset": self.preset
        })
    }
}

#[derive(Debug, Clone)]
pub struct SceneSetTimeOfDayCommand {
    value: f32,
    previous_value: Option<f32>,
}

impl SceneSetTimeOfDayCommand {
    pub fn new(value: f32) -> Self {
        Self {
            value,
            previous_value: None,
        }
    }
}

impl EngineCommand for SceneSetTimeOfDayCommand {
    fn name(&self) -> &'static str {
        "scene.set_time_of_day"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if !(0.0..=24.0).contains(&self.value) {
            ValidationResult::invalid("time_of_day must be between 0.0 and 24.0")
        } else {
            ValidationResult::ok()
        }
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_value = Some(ctx.scene_runtime.time_of_day);
        ctx.scene_runtime.time_of_day = self.value;
        Ok(CommandResult::new(
            format!("time_of_day set to {:.2}", self.value),
            json!({
                "time_of_day": ctx.scene_runtime.time_of_day
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_value) = self.previous_value {
            ctx.scene_runtime.time_of_day = previous_value;
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "value": self.value
        })
    }
}

#[derive(Debug, Clone)]
pub struct SceneAddFogCommand {
    density: f32,
    color: [f32; 3],
    start: f32,
    end: f32,
    previous_fog: Option<Option<SceneFogSettings>>,
}

impl SceneAddFogCommand {
    pub fn new(density: f32, color: [f32; 3], start: f32, end: f32) -> Self {
        Self {
            density,
            color,
            start,
            end,
            previous_fog: None,
        }
    }
}

impl EngineCommand for SceneAddFogCommand {
    fn name(&self) -> &'static str {
        "scene.add_fog"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.density < 0.0 {
            return ValidationResult::invalid("fog density must be >= 0.0");
        }
        if self.start < 0.0 {
            return ValidationResult::invalid("fog start must be >= 0.0");
        }
        if self.end <= self.start {
            return ValidationResult::invalid("fog end must be greater than fog start");
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_fog = Some(ctx.scene_runtime.fog.clone());
        ctx.scene_runtime.fog = Some(SceneFogSettings {
            density: self.density,
            color: self.color,
            start: self.start,
            end: self.end,
        });
        Ok(CommandResult::new(
            "fog configured",
            json!({
                "fog": ctx.scene_runtime.fog
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_fog) = &self.previous_fog {
            ctx.scene_runtime.fog = previous_fog.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "density": self.density,
            "color": self.color,
            "start": self.start,
            "end": self.end
        })
    }
}

#[derive(Debug, Clone)]
pub struct SceneEnableWorldStreamingCommand {
    chunk_size: f32,
    range: u32,
    previous_enabled: Option<bool>,
    previous_chunk_size: Option<f32>,
    previous_range: Option<u32>,
}

impl SceneEnableWorldStreamingCommand {
    pub fn new(chunk_size: f32, range: u32) -> Self {
        Self {
            chunk_size,
            range,
            previous_enabled: None,
            previous_chunk_size: None,
            previous_range: None,
        }
    }
}

impl EngineCommand for SceneEnableWorldStreamingCommand {
    fn name(&self) -> &'static str {
        "scene.enable_world_streaming"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.chunk_size <= 0.0 {
            return ValidationResult::invalid("chunksize must be greater than 0");
        }
        if self.range == 0 {
            return ValidationResult::invalid("range must be greater than 0");
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_enabled = Some(ctx.scene_runtime.world_streaming.enabled);
        self.previous_chunk_size = Some(ctx.scene_runtime.world_streaming.chunk_size);
        self.previous_range = Some(ctx.scene_runtime.world_streaming.range);
        ctx.scene_runtime.world_streaming.enabled = true;
        ctx.scene_runtime.world_streaming.chunk_size = self.chunk_size;
        ctx.scene_runtime.world_streaming.range = self.range;
        Ok(CommandResult::new(
            "world streaming enabled",
            json!({
                "world_streaming": ctx.scene_runtime.world_streaming
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_enabled) = self.previous_enabled {
            ctx.scene_runtime.world_streaming.enabled = previous_enabled;
        }
        if let Some(previous_chunk_size) = self.previous_chunk_size {
            ctx.scene_runtime.world_streaming.chunk_size = previous_chunk_size;
        }
        if let Some(previous_range) = self.previous_range {
            ctx.scene_runtime.world_streaming.range = previous_range;
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "chunk_size": self.chunk_size,
            "range": self.range
        })
    }
}

#[derive(Debug, Clone)]
pub struct SceneCreateStreamChunkCommand {
    chunk_id: String,
    center: [f32; 3],
    radius: f32,
    previous_chunk: Option<Option<StreamChunkRecord>>,
}

impl SceneCreateStreamChunkCommand {
    pub fn new(chunk_id: impl Into<String>, center: [f32; 3], radius: f32) -> Self {
        Self {
            chunk_id: chunk_id.into(),
            center,
            radius,
            previous_chunk: None,
        }
    }
}

impl EngineCommand for SceneCreateStreamChunkCommand {
    fn name(&self) -> &'static str {
        "scene.create_stream_chunk"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if !ctx.scene_runtime.world_streaming.enabled {
            return ValidationResult::invalid(
                "world streaming is disabled; call scene.enable_world_streaming first",
            );
        }
        if self.chunk_id.trim().is_empty() {
            return ValidationResult::invalid("chunk_id cannot be empty");
        }
        if self.radius <= 0.0 {
            return ValidationResult::invalid("radius must be greater than 0");
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let chunk = StreamChunkRecord {
            id: self.chunk_id.clone(),
            center: self.center,
            radius: self.radius,
        };
        self.previous_chunk = Some(
            ctx.scene_runtime
                .world_streaming
                .chunks
                .insert(self.chunk_id.clone(), chunk),
        );
        Ok(CommandResult::new(
            format!("stream chunk '{}' upserted", self.chunk_id),
            json!({
                "chunk": ctx.scene_runtime.world_streaming.chunks.get(&self.chunk_id),
                "chunk_count": ctx.scene_runtime.world_streaming.chunks.len()
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_chunk) = self.previous_chunk.take() {
            match previous_chunk {
                Some(previous_chunk) => {
                    ctx.scene_runtime
                        .world_streaming
                        .chunks
                        .insert(self.chunk_id.clone(), previous_chunk);
                }
                None => {
                    ctx.scene_runtime
                        .world_streaming
                        .chunks
                        .remove(&self.chunk_id);
                    ctx.scene_runtime
                        .world_streaming
                        .entity_to_chunk
                        .retain(|_, chunk_id| chunk_id != &self.chunk_id);
                }
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "chunk_id": self.chunk_id,
            "center": self.center,
            "radius": self.radius
        })
    }
}

#[derive(Debug, Clone)]
pub struct SceneAssignEntityToChunkCommand {
    entity_id: String,
    chunk_id: String,
    previous_chunk: Option<Option<String>>,
}

impl SceneAssignEntityToChunkCommand {
    pub fn new(entity_id: impl Into<String>, chunk_id: impl Into<String>) -> Self {
        Self {
            entity_id: entity_id.into(),
            chunk_id: chunk_id.into(),
            previous_chunk: None,
        }
    }
}

impl EngineCommand for SceneAssignEntityToChunkCommand {
    fn name(&self) -> &'static str {
        "scene.assign_entity_to_chunk"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_id.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if self.chunk_id.trim().is_empty() {
            return ValidationResult::invalid("chunk_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_id) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_id
            ));
        }
        if !ctx
            .scene_runtime
            .world_streaming
            .chunks
            .contains_key(&self.chunk_id)
        {
            return ValidationResult::invalid(format!(
                "chunk '{}' does not exist; call scene.create_stream_chunk first",
                self.chunk_id
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_chunk = Some(
            ctx.scene_runtime
                .world_streaming
                .entity_to_chunk
                .insert(self.entity_id.clone(), self.chunk_id.clone()),
        );
        Ok(CommandResult::new(
            format!(
                "entity '{}' assigned to chunk '{}'",
                self.entity_id, self.chunk_id
            ),
            json!({
                "entity_id": self.entity_id,
                "chunk_id": self.chunk_id
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_chunk) = self.previous_chunk.take() {
            match previous_chunk {
                Some(previous_chunk) => {
                    ctx.scene_runtime
                        .world_streaming
                        .entity_to_chunk
                        .insert(self.entity_id.clone(), previous_chunk);
                }
                None => {
                    ctx.scene_runtime
                        .world_streaming
                        .entity_to_chunk
                        .remove(&self.entity_id);
                }
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_id,
            "chunk_id": self.chunk_id
        })
    }
}

#[derive(Debug, Clone)]
pub struct EntityCreateCommand {
    name: String,
    mesh: String,
    translation: [f32; 3],
    created_index: Option<usize>,
}

impl EntityCreateCommand {
    pub fn new(name: impl Into<String>, mesh: impl Into<String>, translation: [f32; 3]) -> Self {
        Self {
            name: name.into(),
            mesh: mesh.into(),
            translation,
            created_index: None,
        }
    }
}

impl EngineCommand for EntityCreateCommand {
    fn name(&self) -> &'static str {
        "entity.create"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.name.trim().is_empty() {
            return ValidationResult::invalid("entity name cannot be empty");
        }
        if ctx.entity_exists(&self.name) {
            return ValidationResult::invalid(format!(
                "entity '{}' already exists; entity names must be unique",
                self.name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let entity = SceneEntity {
            name: self.name.clone(),
            mesh: self.mesh.clone(),
            translation: self.translation,
        };
        ctx.scene.entities.push(entity);
        self.created_index = Some(ctx.scene.entities.len().saturating_sub(1));
        Ok(CommandResult::new(
            format!("entity '{}' created", self.name),
            json!({
                "name": self.name,
                "mesh": self.mesh,
                "translation": self.translation
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(index) = self.created_index
            && index < ctx.scene.entities.len()
            && ctx.scene.entities[index].name == self.name
        {
            ctx.scene.entities.remove(index);
            return Ok(());
        }

        if let Some(found_index) = ctx
            .scene
            .entities
            .iter()
            .position(|entity| entity.name == self.name)
        {
            ctx.scene.entities.remove(found_index);
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "name": self.name,
            "mesh": self.mesh,
            "translation": self.translation
        })
    }
}

#[derive(Debug, Clone)]
pub struct EntitySetTransformCommand {
    entity_name: String,
    translation: [f32; 3],
    previous_translation: Option<[f32; 3]>,
}

impl EntitySetTransformCommand {
    pub fn new(entity_name: impl Into<String>, translation: [f32; 3]) -> Self {
        Self {
            entity_name: entity_name.into(),
            translation,
            previous_translation: None,
        }
    }
}

impl EngineCommand for EntitySetTransformCommand {
    fn name(&self) -> &'static str {
        "entity.set_transform"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let entity = ctx
            .scene
            .entities
            .iter_mut()
            .find(|entity| entity.name == self.entity_name)
            .with_context(|| format!("entity '{}' not found", self.entity_name))?;
        self.previous_translation = Some(entity.translation);
        entity.translation = self.translation;
        Ok(CommandResult::new(
            format!("transform updated for '{}'", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "translation": self.translation
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        let Some(previous_translation) = self.previous_translation else {
            return Ok(());
        };
        if let Some(entity) = ctx
            .scene
            .entities
            .iter_mut()
            .find(|entity| entity.name == self.entity_name)
        {
            entity.translation = previous_translation;
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "translation": self.translation
        })
    }
}

#[derive(Debug, Clone)]
pub struct EntityAddComponentCommand {
    entity_name: String,
    component_type: String,
    data: Value,
    previous_value: Option<Value>,
}

impl EntityAddComponentCommand {
    pub fn new(
        entity_name: impl Into<String>,
        component_type: impl Into<String>,
        data: Value,
    ) -> Self {
        Self {
            entity_name: entity_name.into(),
            component_type: component_type.into(),
            data,
            previous_value: None,
        }
    }
}

impl EngineCommand for EntityAddComponentCommand {
    fn name(&self) -> &'static str {
        "entity.add_component"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if self.component_type.trim().is_empty() {
            return ValidationResult::invalid("component_type cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let component_bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        self.previous_value =
            component_bucket.insert(self.component_type.clone(), self.data.clone());
        Ok(CommandResult::new(
            format!(
                "component '{}' upserted on '{}'",
                self.component_type, self.entity_name
            ),
            json!({
                "entity_id": self.entity_name,
                "component_type": self.component_type,
                "data": self.data
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(component_bucket) = ctx.components.get_mut(&self.entity_name) {
            match &self.previous_value {
                Some(previous_value) => {
                    component_bucket.insert(self.component_type.clone(), previous_value.clone());
                }
                None => {
                    component_bucket.remove(&self.component_type);
                }
            }
            if component_bucket.is_empty() {
                ctx.components.remove(&self.entity_name);
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "component_type": self.component_type,
            "data": self.data
        })
    }
}

#[derive(Debug, Clone)]
struct EntityStateSnapshot {
    scene: SceneFile,
    scene_runtime: SceneRuntimeSettings,
    selection: Vec<String>,
    components: HashMap<String, BTreeMap<String, Value>>,
    physics: PhysicsRuntimeState,
    gameplay: GameplayRuntimeState,
    animation: AnimationRuntimeState,
    vfx: VfxRuntimeState,
    water: WaterRuntimeState,
    mount: MountRuntimeState,
    npc_ai: NpcAiRuntimeState,
    ui: UiRuntimeState,
    audio: AudioRuntimeState,
    networking: NetworkingRuntimeState,
}

impl EntityStateSnapshot {
    fn capture(ctx: &CommandContext) -> Self {
        Self {
            scene: ctx.scene.clone(),
            scene_runtime: ctx.scene_runtime.clone(),
            selection: ctx.selection.clone(),
            components: ctx.components.clone(),
            physics: ctx.physics.clone(),
            gameplay: ctx.gameplay.clone(),
            animation: ctx.animation.clone(),
            vfx: ctx.vfx.clone(),
            water: ctx.water.clone(),
            mount: ctx.mount.clone(),
            npc_ai: ctx.npc_ai.clone(),
            ui: ctx.ui.clone(),
            audio: ctx.audio.clone(),
            networking: ctx.networking.clone(),
        }
    }

    fn restore(&self, ctx: &mut CommandContext) {
        ctx.scene = self.scene.clone();
        ctx.scene_runtime = self.scene_runtime.clone();
        ctx.selection = self.selection.clone();
        ctx.components = self.components.clone();
        ctx.physics = self.physics.clone();
        ctx.gameplay = self.gameplay.clone();
        ctx.animation = self.animation.clone();
        ctx.vfx = self.vfx.clone();
        ctx.water = self.water.clone();
        ctx.mount = self.mount.clone();
        ctx.npc_ai = self.npc_ai.clone();
        ctx.ui = self.ui.clone();
        ctx.audio = self.audio.clone();
        ctx.networking = self.networking.clone();
    }
}

#[derive(Debug, Clone)]
pub struct EntitySetComponentCommand {
    entity_name: String,
    component_type: String,
    data: Value,
    previous_value: Option<Value>,
}

impl EntitySetComponentCommand {
    pub fn new(
        entity_name: impl Into<String>,
        component_type: impl Into<String>,
        data: Value,
    ) -> Self {
        Self {
            entity_name: entity_name.into(),
            component_type: component_type.into(),
            data,
            previous_value: None,
        }
    }
}

impl EngineCommand for EntitySetComponentCommand {
    fn name(&self) -> &'static str {
        "entity.set_component"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if self.component_type.trim().is_empty() {
            return ValidationResult::invalid("component_type cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let component_bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        self.previous_value =
            component_bucket.insert(self.component_type.clone(), self.data.clone());
        Ok(CommandResult::new(
            format!(
                "component '{}' set on '{}'",
                self.component_type, self.entity_name
            ),
            json!({
                "entity_id": self.entity_name,
                "component_type": self.component_type,
                "data": self.data
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(component_bucket) = ctx.components.get_mut(&self.entity_name) {
            match &self.previous_value {
                Some(previous_value) => {
                    component_bucket.insert(self.component_type.clone(), previous_value.clone());
                }
                None => {
                    component_bucket.remove(&self.component_type);
                }
            }
            if component_bucket.is_empty() {
                ctx.components.remove(&self.entity_name);
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "component_type": self.component_type,
            "data": self.data
        })
    }
}

#[derive(Debug, Clone)]
pub struct EntityRemoveComponentCommand {
    entity_name: String,
    component_type: String,
    previous_value: Option<Value>,
}

impl EntityRemoveComponentCommand {
    pub fn new(entity_name: impl Into<String>, component_type: impl Into<String>) -> Self {
        Self {
            entity_name: entity_name.into(),
            component_type: component_type.into(),
            previous_value: None,
        }
    }
}

impl EngineCommand for EntityRemoveComponentCommand {
    fn name(&self) -> &'static str {
        "entity.remove_component"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if self.component_type.trim().is_empty() {
            return ValidationResult::invalid("component_type cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let removed = if let Some(component_bucket) = ctx.components.get_mut(&self.entity_name) {
            let removed = component_bucket.remove(&self.component_type);
            if component_bucket.is_empty() {
                ctx.components.remove(&self.entity_name);
            }
            removed
        } else {
            None
        };
        self.previous_value = removed.clone();
        Ok(CommandResult::new(
            format!(
                "component '{}' removed from '{}'",
                self.component_type, self.entity_name
            ),
            json!({
                "entity_id": self.entity_name,
                "component_type": self.component_type,
                "removed": removed.is_some(),
                "data": removed
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_value) = &self.previous_value {
            let component_bucket = ctx.components.entry(self.entity_name.clone()).or_default();
            component_bucket.insert(self.component_type.clone(), previous_value.clone());
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "component_type": self.component_type
        })
    }
}

#[derive(Debug, Clone)]
pub struct EntityCloneCommand {
    source_name: String,
    requested_name: Option<String>,
    translation_offset: [f32; 3],
    copy_components: bool,
    copy_parent: bool,
    created_name: Option<String>,
    created_index: Option<usize>,
}

impl EntityCloneCommand {
    pub fn new(
        source_name: impl Into<String>,
        requested_name: Option<String>,
        translation_offset: [f32; 3],
        copy_components: bool,
        copy_parent: bool,
    ) -> Self {
        Self {
            source_name: source_name.into(),
            requested_name,
            translation_offset,
            copy_components,
            copy_parent,
            created_name: None,
            created_index: None,
        }
    }
}

impl EngineCommand for EntityCloneCommand {
    fn name(&self) -> &'static str {
        "entity.clone"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.source_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.source_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.source_name
            ));
        }
        if let Some(requested_name) = &self.requested_name {
            let requested_name = requested_name.trim();
            if requested_name.is_empty() {
                return ValidationResult::invalid("name cannot be empty when provided");
            }
            if ctx.entity_exists(requested_name) {
                return ValidationResult::invalid(format!(
                    "entity '{}' already exists; entity names must be unique",
                    requested_name
                ));
            }
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let source_entity = ctx
            .scene
            .entities
            .iter()
            .find(|entity| entity.name == self.source_name)
            .cloned()
            .with_context(|| format!("entity '{}' not found", self.source_name))?;

        let desired_name = self
            .requested_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}_copy", self.source_name));
        let created_name = unique_entity_name(&ctx.scene, &desired_name);
        if self.requested_name.is_some() && !created_name.eq_ignore_ascii_case(&desired_name) {
            bail!(
                "entity '{}' already exists; choose another name",
                desired_name
            );
        }

        let mut cloned_entity = source_entity.clone();
        cloned_entity.name = created_name.clone();
        cloned_entity.translation = [
            source_entity.translation[0] + self.translation_offset[0],
            source_entity.translation[1] + self.translation_offset[1],
            source_entity.translation[2] + self.translation_offset[2],
        ];
        ctx.scene.entities.push(cloned_entity);
        self.created_index = Some(ctx.scene.entities.len().saturating_sub(1));

        if self.copy_components
            && let Some(source_components) = ctx.components.get(&self.source_name).cloned()
        {
            let mut cloned_components = source_components;
            if !self.copy_parent {
                cloned_components.remove("HierarchyParent");
            }
            if !cloned_components.is_empty() {
                ctx.components
                    .insert(created_name.clone(), cloned_components);
            }
        }

        self.created_name = Some(created_name.clone());
        Ok(CommandResult::new(
            format!("entity '{}' cloned to '{}'", self.source_name, created_name),
            json!({
                "source_entity_id": self.source_name,
                "entity_id": created_name,
                "translation_offset": self.translation_offset,
                "copy_components": self.copy_components,
                "copy_parent": self.copy_parent
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        let Some(created_name) = &self.created_name else {
            return Ok(());
        };
        if let Some(index) = self.created_index {
            if index < ctx.scene.entities.len() && ctx.scene.entities[index].name == *created_name {
                ctx.scene.entities.remove(index);
            } else if let Some(found_index) = ctx
                .scene
                .entities
                .iter()
                .position(|entity| entity.name == *created_name)
            {
                ctx.scene.entities.remove(found_index);
            }
        } else if let Some(found_index) = ctx
            .scene
            .entities
            .iter()
            .position(|entity| entity.name == *created_name)
        {
            ctx.scene.entities.remove(found_index);
        }
        ctx.components.remove(created_name);
        ctx.selection
            .retain(|entity_name| entity_name != created_name);
        ctx.scene_runtime
            .world_streaming
            .entity_to_chunk
            .remove(created_name);
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.source_name,
            "name": self.requested_name,
            "translation_offset": self.translation_offset,
            "copy_components": self.copy_components,
            "copy_parent": self.copy_parent
        })
    }
}

#[derive(Debug, Clone)]
pub struct EntityDeleteCommand {
    entity_name: String,
    previous_state: Option<EntityStateSnapshot>,
}

impl EntityDeleteCommand {
    pub fn new(entity_name: impl Into<String>) -> Self {
        Self {
            entity_name: entity_name.into(),
            previous_state: None,
        }
    }
}

impl EngineCommand for EntityDeleteCommand {
    fn name(&self) -> &'static str {
        "entity.delete"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_state = Some(EntityStateSnapshot::capture(ctx));
        let remove_index = ctx
            .scene
            .entities
            .iter()
            .position(|entity| entity.name == self.entity_name)
            .with_context(|| format!("entity '{}' not found", self.entity_name))?;
        ctx.scene.entities.remove(remove_index);
        ctx.selection
            .retain(|entity_name| entity_name != &self.entity_name);
        ctx.components.remove(&self.entity_name);
        ctx.scene_runtime
            .world_streaming
            .entity_to_chunk
            .remove(&self.entity_name);
        clear_parent_links_to_entity(ctx, &self.entity_name);
        Ok(CommandResult::new(
            format!("entity '{}' deleted", self.entity_name),
            json!({
                "entity_id": self.entity_name
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_state) = &self.previous_state {
            previous_state.restore(ctx);
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name
        })
    }
}

#[derive(Debug, Clone)]
pub struct EntityRenameCommand {
    entity_name: String,
    new_name: String,
    previous_state: Option<EntityStateSnapshot>,
}

impl EntityRenameCommand {
    pub fn new(entity_name: impl Into<String>, new_name: impl Into<String>) -> Self {
        Self {
            entity_name: entity_name.into(),
            new_name: new_name.into(),
            previous_state: None,
        }
    }
}

impl EngineCommand for EntityRenameCommand {
    fn name(&self) -> &'static str {
        "entity.rename"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if self.new_name.trim().is_empty() {
            return ValidationResult::invalid("name cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        if self.entity_name != self.new_name && ctx.entity_exists(&self.new_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' already exists; choose another name",
                self.new_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_state = Some(EntityStateSnapshot::capture(ctx));
        let entity = ctx
            .scene
            .entities
            .iter_mut()
            .find(|entity| entity.name == self.entity_name)
            .with_context(|| format!("entity '{}' not found", self.entity_name))?;
        entity.name = self.new_name.clone();
        remap_entity_references(ctx, &self.entity_name, &self.new_name);
        Ok(CommandResult::new(
            format!(
                "entity '{}' renamed to '{}'",
                self.entity_name, self.new_name
            ),
            json!({
                "entity_id": self.entity_name,
                "name": self.new_name
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_state) = &self.previous_state {
            previous_state.restore(ctx);
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "name": self.new_name
        })
    }
}

#[derive(Debug, Clone)]
pub struct EntityParentCommand {
    child_name: String,
    parent_name: String,
    previous_parent_component: Option<Value>,
}

impl EntityParentCommand {
    pub fn new(child_name: impl Into<String>, parent_name: impl Into<String>) -> Self {
        Self {
            child_name: child_name.into(),
            parent_name: parent_name.into(),
            previous_parent_component: None,
        }
    }
}

impl EngineCommand for EntityParentCommand {
    fn name(&self) -> &'static str {
        "entity.parent"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.child_name.trim().is_empty() {
            return ValidationResult::invalid("child_id cannot be empty");
        }
        if self.parent_name.trim().is_empty() {
            return ValidationResult::invalid("parent_id cannot be empty");
        }
        if self.child_name == self.parent_name {
            return ValidationResult::invalid("child_id cannot be the same as parent_id");
        }
        if !ctx.entity_exists(&self.child_name) {
            return ValidationResult::invalid(format!(
                "child entity '{}' does not exist in open scene",
                self.child_name
            ));
        }
        if !ctx.entity_exists(&self.parent_name) {
            return ValidationResult::invalid(format!(
                "parent entity '{}' does not exist in open scene",
                self.parent_name
            ));
        }
        if parent_would_create_cycle(ctx, &self.child_name, &self.parent_name) {
            return ValidationResult::invalid(format!(
                "cannot parent '{}' under '{}' because it creates a hierarchy cycle",
                self.child_name, self.parent_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let bucket = ctx.components.entry(self.child_name.clone()).or_default();
        self.previous_parent_component = bucket.get("HierarchyParent").cloned();
        bucket.insert(
            "HierarchyParent".to_string(),
            json!({
                "parent_id": self.parent_name
            }),
        );
        Ok(CommandResult::new(
            format!(
                "entity '{}' parented under '{}'",
                self.child_name, self.parent_name
            ),
            json!({
                "child_id": self.child_name,
                "parent_id": self.parent_name
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        let bucket = ctx.components.entry(self.child_name.clone()).or_default();
        match &self.previous_parent_component {
            Some(previous_parent_component) => {
                bucket.insert(
                    "HierarchyParent".to_string(),
                    previous_parent_component.clone(),
                );
            }
            None => {
                bucket.remove("HierarchyParent");
                if bucket.is_empty() {
                    ctx.components.remove(&self.child_name);
                }
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "child_id": self.child_name,
            "parent_id": self.parent_name
        })
    }
}

#[derive(Debug, Clone)]
pub struct EntityUnparentCommand {
    child_name: String,
    previous_parent_component: Option<Value>,
}

impl EntityUnparentCommand {
    pub fn new(child_name: impl Into<String>) -> Self {
        Self {
            child_name: child_name.into(),
            previous_parent_component: None,
        }
    }
}

impl EngineCommand for EntityUnparentCommand {
    fn name(&self) -> &'static str {
        "entity.unparent"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.child_name.trim().is_empty() {
            return ValidationResult::invalid("child_id cannot be empty");
        }
        if !ctx.entity_exists(&self.child_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.child_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        if let Some(bucket) = ctx.components.get_mut(&self.child_name) {
            self.previous_parent_component = bucket.remove("HierarchyParent");
            if bucket.is_empty() {
                ctx.components.remove(&self.child_name);
            }
        } else {
            self.previous_parent_component = None;
        }
        Ok(CommandResult::new(
            format!("entity '{}' unparented", self.child_name),
            json!({
                "child_id": self.child_name,
                "removed": self.previous_parent_component.is_some()
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_parent_component) = &self.previous_parent_component {
            let bucket = ctx.components.entry(self.child_name.clone()).or_default();
            bucket.insert(
                "HierarchyParent".to_string(),
                previous_parent_component.clone(),
            );
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "child_id": self.child_name
        })
    }
}

#[derive(Debug, Clone)]
pub struct EntityTranslateCommand {
    entity_name: String,
    delta: [f32; 3],
    previous_translation: Option<[f32; 3]>,
}

impl EntityTranslateCommand {
    pub fn new(entity_name: impl Into<String>, delta: [f32; 3]) -> Self {
        Self {
            entity_name: entity_name.into(),
            delta,
            previous_translation: None,
        }
    }
}

impl EngineCommand for EntityTranslateCommand {
    fn name(&self) -> &'static str {
        "entity.translate"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let entity = ctx
            .scene
            .entities
            .iter_mut()
            .find(|entity| entity.name == self.entity_name)
            .with_context(|| format!("entity '{}' not found", self.entity_name))?;
        self.previous_translation = Some(entity.translation);
        entity.translation = [
            entity.translation[0] + self.delta[0],
            entity.translation[1] + self.delta[1],
            entity.translation[2] + self.delta[2],
        ];
        Ok(CommandResult::new(
            format!("entity '{}' translated", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "translation": entity.translation,
                "delta": self.delta
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        let Some(previous_translation) = self.previous_translation else {
            return Ok(());
        };
        if let Some(entity) = ctx
            .scene
            .entities
            .iter_mut()
            .find(|entity| entity.name == self.entity_name)
        {
            entity.translation = previous_translation;
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "delta": self.delta
        })
    }
}

#[derive(Debug, Clone)]
pub struct EntityRotateCommand {
    entity_name: String,
    delta_euler: [f32; 3],
    previous_rotation_component: Option<Value>,
}

impl EntityRotateCommand {
    pub fn new(entity_name: impl Into<String>, delta_euler: [f32; 3]) -> Self {
        Self {
            entity_name: entity_name.into(),
            delta_euler,
            previous_rotation_component: None,
        }
    }
}

impl EngineCommand for EntityRotateCommand {
    fn name(&self) -> &'static str {
        "entity.rotate"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        self.previous_rotation_component = bucket.get("TransformRotation").cloned();
        let current_rotation =
            parse_component_vec3(self.previous_rotation_component.as_ref(), [0.0, 0.0, 0.0]);
        let next_rotation = [
            current_rotation[0] + self.delta_euler[0],
            current_rotation[1] + self.delta_euler[1],
            current_rotation[2] + self.delta_euler[2],
        ];
        bucket.insert("TransformRotation".to_string(), json!(next_rotation));
        Ok(CommandResult::new(
            format!("entity '{}' rotated", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "rotation": next_rotation,
                "delta": self.delta_euler
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(bucket) = ctx.components.get_mut(&self.entity_name) {
            match &self.previous_rotation_component {
                Some(previous_rotation_component) => {
                    bucket.insert(
                        "TransformRotation".to_string(),
                        previous_rotation_component.clone(),
                    );
                }
                None => {
                    bucket.remove("TransformRotation");
                    if bucket.is_empty() {
                        ctx.components.remove(&self.entity_name);
                    }
                }
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "delta": self.delta_euler
        })
    }
}

#[derive(Debug, Clone)]
pub struct EntityScaleCommand {
    entity_name: String,
    factor: [f32; 3],
    previous_scale_component: Option<Value>,
}

impl EntityScaleCommand {
    pub fn new(entity_name: impl Into<String>, factor: [f32; 3]) -> Self {
        Self {
            entity_name: entity_name.into(),
            factor,
            previous_scale_component: None,
        }
    }
}

impl EngineCommand for EntityScaleCommand {
    fn name(&self) -> &'static str {
        "entity.scale"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        if self
            .factor
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return ValidationResult::invalid("factor must contain finite values > 0");
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        self.previous_scale_component = bucket.get("TransformScale").cloned();
        let current_scale =
            parse_component_vec3(self.previous_scale_component.as_ref(), [1.0, 1.0, 1.0]);
        let next_scale = [
            (current_scale[0] * self.factor[0]).clamp(0.0001, 10000.0),
            (current_scale[1] * self.factor[1]).clamp(0.0001, 10000.0),
            (current_scale[2] * self.factor[2]).clamp(0.0001, 10000.0),
        ];
        bucket.insert("TransformScale".to_string(), json!(next_scale));
        Ok(CommandResult::new(
            format!("entity '{}' scaled", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "scale": next_scale,
                "factor": self.factor
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(bucket) = ctx.components.get_mut(&self.entity_name) {
            match &self.previous_scale_component {
                Some(previous_scale_component) => {
                    bucket.insert(
                        "TransformScale".to_string(),
                        previous_scale_component.clone(),
                    );
                }
                None => {
                    bucket.remove("TransformScale");
                    if bucket.is_empty() {
                        ctx.components.remove(&self.entity_name);
                    }
                }
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "factor": self.factor
        })
    }
}

#[derive(Debug, Clone)]
pub struct PhysAddColliderCommand {
    entity_name: String,
    collider: PhysicsCollider,
    previous_collider: Option<PhysicsCollider>,
}

impl PhysAddColliderCommand {
    pub fn new(
        entity_name: impl Into<String>,
        shape: impl Into<String>,
        size: [f32; 3],
        is_trigger: bool,
    ) -> Self {
        Self {
            entity_name: entity_name.into(),
            collider: PhysicsCollider {
                shape: normalize_collider_shape(shape.into()),
                size: [
                    size[0].abs().max(0.01),
                    size[1].abs().max(0.01),
                    size[2].abs().max(0.01),
                ],
                is_trigger,
            },
            previous_collider: None,
        }
    }
}

impl EngineCommand for PhysAddColliderCommand {
    fn name(&self) -> &'static str {
        "phys.add_collider"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_collider = ctx
            .physics
            .colliders
            .insert(self.entity_name.clone(), self.collider.clone());
        let component_bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        component_bucket.insert(
            "Collider".to_string(),
            json!({
                "shape": self.collider.shape,
                "size": self.collider.size,
                "is_trigger": self.collider.is_trigger
            }),
        );
        Ok(CommandResult::new(
            format!("collider set on '{}'", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "collider": self.collider
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        match &self.previous_collider {
            Some(previous) => {
                ctx.physics
                    .colliders
                    .insert(self.entity_name.clone(), previous.clone());
                if let Some(component_bucket) = ctx.components.get_mut(&self.entity_name) {
                    component_bucket.insert(
                        "Collider".to_string(),
                        json!({
                            "shape": previous.shape,
                            "size": previous.size,
                            "is_trigger": previous.is_trigger
                        }),
                    );
                }
            }
            None => {
                ctx.physics.colliders.remove(&self.entity_name);
                if let Some(component_bucket) = ctx.components.get_mut(&self.entity_name) {
                    component_bucket.remove("Collider");
                }
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "shape": self.collider.shape,
            "size": self.collider.size,
            "is_trigger": self.collider.is_trigger
        })
    }
}

#[derive(Debug, Clone)]
pub struct PhysAddRigidbodyCommand {
    entity_name: String,
    rigidbody: PhysicsRigidbody,
    previous_rigidbody: Option<PhysicsRigidbody>,
}

impl PhysAddRigidbodyCommand {
    pub fn new(
        entity_name: impl Into<String>,
        body_type: impl Into<String>,
        mass: f32,
        friction: f32,
        restitution: f32,
    ) -> Self {
        let body_type = normalize_body_type(body_type.into());
        Self {
            entity_name: entity_name.into(),
            rigidbody: PhysicsRigidbody {
                body_type,
                mass: mass.max(0.01),
                friction: friction.clamp(0.0, 2.0),
                restitution: restitution.clamp(0.0, 1.0),
                velocity: [0.0, 0.0, 0.0],
            },
            previous_rigidbody: None,
        }
    }
}

impl EngineCommand for PhysAddRigidbodyCommand {
    fn name(&self) -> &'static str {
        "phys.add_rigidbody"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_rigidbody = ctx
            .physics
            .rigidbodies
            .insert(self.entity_name.clone(), self.rigidbody.clone());
        let component_bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        component_bucket.insert(
            "Rigidbody".to_string(),
            json!({
                "type": self.rigidbody.body_type,
                "mass": self.rigidbody.mass,
                "friction": self.rigidbody.friction,
                "restitution": self.rigidbody.restitution,
                "velocity": self.rigidbody.velocity
            }),
        );
        Ok(CommandResult::new(
            format!("rigidbody set on '{}'", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "rigidbody": self.rigidbody
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        match &self.previous_rigidbody {
            Some(previous) => {
                ctx.physics
                    .rigidbodies
                    .insert(self.entity_name.clone(), previous.clone());
                if let Some(component_bucket) = ctx.components.get_mut(&self.entity_name) {
                    component_bucket.insert(
                        "Rigidbody".to_string(),
                        json!({
                            "type": previous.body_type,
                            "mass": previous.mass,
                            "friction": previous.friction,
                            "restitution": previous.restitution,
                            "velocity": previous.velocity
                        }),
                    );
                }
            }
            None => {
                ctx.physics.rigidbodies.remove(&self.entity_name);
                if let Some(component_bucket) = ctx.components.get_mut(&self.entity_name) {
                    component_bucket.remove("Rigidbody");
                }
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "type": self.rigidbody.body_type,
            "mass": self.rigidbody.mass,
            "friction": self.rigidbody.friction,
            "restitution": self.rigidbody.restitution
        })
    }
}

#[derive(Debug, Clone)]
pub struct PhysApplyImpulseCommand {
    entity_name: String,
    impulse: [f32; 3],
    previous_rigidbody: Option<PhysicsRigidbody>,
    previous_translation: Option<[f32; 3]>,
}

impl PhysApplyImpulseCommand {
    pub fn new(entity_name: impl Into<String>, impulse: [f32; 3]) -> Self {
        Self {
            entity_name: entity_name.into(),
            impulse,
            previous_rigidbody: None,
            previous_translation: None,
        }
    }
}

impl EngineCommand for PhysApplyImpulseCommand {
    fn name(&self) -> &'static str {
        "phys.apply_impulse"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        if !ctx.physics.rigidbodies.contains_key(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' has no rigidbody; call phys.add_rigidbody first",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let rigidbody = ctx
            .physics
            .rigidbodies
            .get_mut(&self.entity_name)
            .with_context(|| format!("entity '{}' has no rigidbody", self.entity_name))?;
        self.previous_rigidbody = Some(rigidbody.clone());

        if rigidbody.body_type != "static" {
            let inv_mass = rigidbody.mass.max(0.01).recip();
            rigidbody.velocity = [
                rigidbody.velocity[0] + self.impulse[0] * inv_mass,
                rigidbody.velocity[1] + self.impulse[1] * inv_mass,
                rigidbody.velocity[2] + self.impulse[2] * inv_mass,
            ];
            let step = 0.016;
            if let Some(entity) = ctx
                .scene
                .entities
                .iter_mut()
                .find(|entity| entity.name == self.entity_name)
            {
                self.previous_translation = Some(entity.translation);
                entity.translation = [
                    entity.translation[0] + rigidbody.velocity[0] * step,
                    entity.translation[1] + rigidbody.velocity[1] * step,
                    entity.translation[2] + rigidbody.velocity[2] * step,
                ];
            }
        }

        if let Some(component_bucket) = ctx.components.get_mut(&self.entity_name) {
            component_bucket.insert(
                "Rigidbody".to_string(),
                json!({
                    "type": rigidbody.body_type,
                    "mass": rigidbody.mass,
                    "friction": rigidbody.friction,
                    "restitution": rigidbody.restitution,
                    "velocity": rigidbody.velocity
                }),
            );
        }

        Ok(CommandResult::new(
            format!("impulse applied to '{}'", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "impulse": self.impulse,
                "velocity": rigidbody.velocity
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_rigidbody) = &self.previous_rigidbody {
            ctx.physics
                .rigidbodies
                .insert(self.entity_name.clone(), previous_rigidbody.clone());
            if let Some(component_bucket) = ctx.components.get_mut(&self.entity_name) {
                component_bucket.insert(
                    "Rigidbody".to_string(),
                    json!({
                        "type": previous_rigidbody.body_type,
                        "mass": previous_rigidbody.mass,
                        "friction": previous_rigidbody.friction,
                        "restitution": previous_rigidbody.restitution,
                        "velocity": previous_rigidbody.velocity
                    }),
                );
            }
        }
        if let Some(previous_translation) = self.previous_translation
            && let Some(entity) = ctx
                .scene
                .entities
                .iter_mut()
                .find(|entity| entity.name == self.entity_name)
        {
            entity.translation = previous_translation;
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "impulse": self.impulse
        })
    }
}

#[derive(Debug, Clone)]
pub struct PhysSetGravityCommand {
    gravity: [f32; 3],
    previous_gravity: Option<[f32; 3]>,
}

impl PhysSetGravityCommand {
    pub fn new(gravity: [f32; 3]) -> Self {
        Self {
            gravity,
            previous_gravity: None,
        }
    }
}

impl EngineCommand for PhysSetGravityCommand {
    fn name(&self) -> &'static str {
        "phys.set_gravity"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_gravity = Some(ctx.physics.gravity);
        ctx.physics.gravity = self.gravity;
        Ok(CommandResult::new(
            "gravity updated",
            json!({
                "gravity": ctx.physics.gravity
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_gravity) = self.previous_gravity {
            ctx.physics.gravity = previous_gravity;
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "gravity": self.gravity
        })
    }
}

#[derive(Debug, Clone)]
pub struct PhysRemoveColliderCommand {
    entity_name: String,
    previous_collider: Option<PhysicsCollider>,
    previous_component: Option<Value>,
}

impl PhysRemoveColliderCommand {
    pub fn new(entity_name: impl Into<String>) -> Self {
        Self {
            entity_name: entity_name.into(),
            previous_collider: None,
            previous_component: None,
        }
    }
}

impl EngineCommand for PhysRemoveColliderCommand {
    fn name(&self) -> &'static str {
        "phys.remove_collider"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_collider = ctx.physics.colliders.remove(&self.entity_name);
        self.previous_component = ctx
            .components
            .get(&self.entity_name)
            .and_then(|bucket| bucket.get("Collider"))
            .cloned();
        if let Some(bucket) = ctx.components.get_mut(&self.entity_name) {
            bucket.remove("Collider");
            if bucket.is_empty() {
                ctx.components.remove(&self.entity_name);
            }
        }
        Ok(CommandResult::new(
            format!("collider removed from '{}'", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "removed": self.previous_collider.is_some()
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_collider) = &self.previous_collider {
            ctx.physics
                .colliders
                .insert(self.entity_name.clone(), previous_collider.clone());
        }
        if let Some(previous_component) = &self.previous_component {
            let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
            bucket.insert("Collider".to_string(), previous_component.clone());
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name
        })
    }
}

#[derive(Debug, Clone)]
pub struct PhysSetRigidbodyParamsCommand {
    entity_name: String,
    mass: Option<f32>,
    friction: Option<f32>,
    restitution: Option<f32>,
    previous_rigidbody: Option<PhysicsRigidbody>,
}

impl PhysSetRigidbodyParamsCommand {
    pub fn new(
        entity_name: impl Into<String>,
        mass: Option<f32>,
        friction: Option<f32>,
        restitution: Option<f32>,
    ) -> Self {
        Self {
            entity_name: entity_name.into(),
            mass,
            friction,
            restitution,
            previous_rigidbody: None,
        }
    }
}

impl EngineCommand for PhysSetRigidbodyParamsCommand {
    fn name(&self) -> &'static str {
        "phys.set_rigidbody_params"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        if !ctx.physics.rigidbodies.contains_key(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' has no rigidbody; call phys.add_rigidbody first",
                self.entity_name
            ));
        }
        if self.mass.is_none() && self.friction.is_none() && self.restitution.is_none() {
            return ValidationResult::invalid(
                "at least one of mass/friction/restitution must be provided",
            );
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let rigidbody = ctx
            .physics
            .rigidbodies
            .get_mut(&self.entity_name)
            .with_context(|| format!("entity '{}' has no rigidbody", self.entity_name))?;
        self.previous_rigidbody = Some(rigidbody.clone());
        if let Some(mass) = self.mass {
            rigidbody.mass = mass.max(0.01);
        }
        if let Some(friction) = self.friction {
            rigidbody.friction = friction.clamp(0.0, 2.0);
        }
        if let Some(restitution) = self.restitution {
            rigidbody.restitution = restitution.clamp(0.0, 1.0);
        }
        let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        bucket.insert(
            "Rigidbody".to_string(),
            json!({
                "type": rigidbody.body_type,
                "mass": rigidbody.mass,
                "friction": rigidbody.friction,
                "restitution": rigidbody.restitution,
                "velocity": rigidbody.velocity
            }),
        );
        Ok(CommandResult::new(
            format!("rigidbody params updated on '{}'", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "rigidbody": rigidbody
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_rigidbody) = &self.previous_rigidbody {
            ctx.physics
                .rigidbodies
                .insert(self.entity_name.clone(), previous_rigidbody.clone());
            let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
            bucket.insert(
                "Rigidbody".to_string(),
                json!({
                    "type": previous_rigidbody.body_type,
                    "mass": previous_rigidbody.mass,
                    "friction": previous_rigidbody.friction,
                    "restitution": previous_rigidbody.restitution,
                    "velocity": previous_rigidbody.velocity
                }),
            );
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "mass": self.mass,
            "friction": self.friction,
            "restitution": self.restitution
        })
    }
}

#[derive(Debug, Clone)]
pub struct PhysApplyForceCommand {
    entity_name: String,
    force: [f32; 3],
    dt: f32,
    previous_rigidbody: Option<PhysicsRigidbody>,
    previous_translation: Option<[f32; 3]>,
}

impl PhysApplyForceCommand {
    pub fn new(entity_name: impl Into<String>, force: [f32; 3], dt: f32) -> Self {
        Self {
            entity_name: entity_name.into(),
            force,
            dt: dt.max(0.0001),
            previous_rigidbody: None,
            previous_translation: None,
        }
    }
}

impl EngineCommand for PhysApplyForceCommand {
    fn name(&self) -> &'static str {
        "phys.apply_force"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        if !ctx.physics.rigidbodies.contains_key(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' has no rigidbody; call phys.add_rigidbody first",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let rigidbody = ctx
            .physics
            .rigidbodies
            .get_mut(&self.entity_name)
            .with_context(|| format!("entity '{}' has no rigidbody", self.entity_name))?;
        self.previous_rigidbody = Some(rigidbody.clone());

        if rigidbody.body_type != "static" {
            let inv_mass = rigidbody.mass.max(0.01).recip();
            let acceleration = [
                self.force[0] * inv_mass + ctx.physics.gravity[0],
                self.force[1] * inv_mass + ctx.physics.gravity[1],
                self.force[2] * inv_mass + ctx.physics.gravity[2],
            ];
            rigidbody.velocity = [
                rigidbody.velocity[0] + acceleration[0] * self.dt,
                rigidbody.velocity[1] + acceleration[1] * self.dt,
                rigidbody.velocity[2] + acceleration[2] * self.dt,
            ];

            if let Some(entity) = ctx
                .scene
                .entities
                .iter_mut()
                .find(|entity| entity.name == self.entity_name)
            {
                self.previous_translation = Some(entity.translation);
                entity.translation = [
                    entity.translation[0] + rigidbody.velocity[0] * self.dt,
                    entity.translation[1] + rigidbody.velocity[1] * self.dt,
                    entity.translation[2] + rigidbody.velocity[2] * self.dt,
                ];
            }
        }

        let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        bucket.insert(
            "Rigidbody".to_string(),
            json!({
                "type": rigidbody.body_type,
                "mass": rigidbody.mass,
                "friction": rigidbody.friction,
                "restitution": rigidbody.restitution,
                "velocity": rigidbody.velocity
            }),
        );

        Ok(CommandResult::new(
            format!("force applied to '{}'", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "force": self.force,
                "dt": self.dt,
                "velocity": rigidbody.velocity
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_rigidbody) = &self.previous_rigidbody {
            ctx.physics
                .rigidbodies
                .insert(self.entity_name.clone(), previous_rigidbody.clone());
            let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
            bucket.insert(
                "Rigidbody".to_string(),
                json!({
                    "type": previous_rigidbody.body_type,
                    "mass": previous_rigidbody.mass,
                    "friction": previous_rigidbody.friction,
                    "restitution": previous_rigidbody.restitution,
                    "velocity": previous_rigidbody.velocity
                }),
            );
        }
        if let Some(previous_translation) = self.previous_translation
            && let Some(entity) = ctx
                .scene
                .entities
                .iter_mut()
                .find(|entity| entity.name == self.entity_name)
        {
            entity.translation = previous_translation;
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "force": self.force,
            "dt": self.dt
        })
    }
}

#[derive(Debug, Clone)]
pub struct PhysAddCharacterControllerCommand {
    entity_name: String,
    controller: PhysicsCharacterController,
    previous_controller: Option<PhysicsCharacterController>,
    previous_component: Option<Value>,
}

impl PhysAddCharacterControllerCommand {
    pub fn new(
        entity_name: impl Into<String>,
        radius: f32,
        height: f32,
        speed: f32,
        jump_strength: f32,
    ) -> Self {
        Self {
            entity_name: entity_name.into(),
            controller: PhysicsCharacterController {
                radius: radius.max(0.05),
                height: height.max(0.2),
                speed: speed.max(0.01),
                jump_strength: jump_strength.max(0.01),
                grounded: true,
                state: "idle".to_string(),
            },
            previous_controller: None,
            previous_component: None,
        }
    }
}

impl EngineCommand for PhysAddCharacterControllerCommand {
    fn name(&self) -> &'static str {
        "phys.add_character_controller"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_controller = ctx
            .physics
            .character_controllers
            .insert(self.entity_name.clone(), self.controller.clone());
        self.previous_component = ctx
            .components
            .get(&self.entity_name)
            .and_then(|bucket| bucket.get("CharacterController"))
            .cloned();
        let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        bucket.insert(
            "CharacterController".to_string(),
            json!({
                "radius": self.controller.radius,
                "height": self.controller.height,
                "speed": self.controller.speed,
                "jump_strength": self.controller.jump_strength,
                "grounded": self.controller.grounded,
                "state": self.controller.state
            }),
        );
        Ok(CommandResult::new(
            format!("character controller configured on '{}'", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "controller": self.controller
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        match &self.previous_controller {
            Some(previous_controller) => {
                ctx.physics
                    .character_controllers
                    .insert(self.entity_name.clone(), previous_controller.clone());
            }
            None => {
                ctx.physics.character_controllers.remove(&self.entity_name);
            }
        }
        match &self.previous_component {
            Some(previous_component) => {
                let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
                bucket.insert(
                    "CharacterController".to_string(),
                    previous_component.clone(),
                );
            }
            None => {
                if let Some(bucket) = ctx.components.get_mut(&self.entity_name) {
                    bucket.remove("CharacterController");
                }
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "radius": self.controller.radius,
            "height": self.controller.height,
            "speed": self.controller.speed,
            "jump_strength": self.controller.jump_strength
        })
    }
}

#[derive(Debug, Clone)]
pub struct PhysCharacterMoveCommand {
    entity_name: String,
    input: [f32; 3],
    dt: f32,
    previous_translation: Option<[f32; 3]>,
    previous_controller: Option<PhysicsCharacterController>,
}

impl PhysCharacterMoveCommand {
    pub fn new(entity_name: impl Into<String>, input: [f32; 3], dt: f32) -> Self {
        Self {
            entity_name: entity_name.into(),
            input,
            dt: dt.max(0.0001),
            previous_translation: None,
            previous_controller: None,
        }
    }
}

impl EngineCommand for PhysCharacterMoveCommand {
    fn name(&self) -> &'static str {
        "phys.character_move"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        if !ctx
            .physics
            .character_controllers
            .contains_key(&self.entity_name)
        {
            return ValidationResult::invalid(format!(
                "entity '{}' has no character controller; call phys.add_character_controller first",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let controller = ctx
            .physics
            .character_controllers
            .get_mut(&self.entity_name)
            .with_context(|| {
                format!("entity '{}' has no character controller", self.entity_name)
            })?;
        self.previous_controller = Some(controller.clone());

        let magnitude = (self.input[0] * self.input[0]
            + self.input[1] * self.input[1]
            + self.input[2] * self.input[2])
            .sqrt();
        let direction = if magnitude > 1e-6 {
            [
                self.input[0] / magnitude,
                self.input[1] / magnitude,
                self.input[2] / magnitude,
            ]
        } else {
            [0.0, 0.0, 0.0]
        };
        let distance = controller.speed * self.dt * magnitude.min(1.0);
        let delta = [
            direction[0] * distance,
            direction[1] * distance,
            direction[2] * distance,
        ];
        controller.state = if magnitude > 0.01 {
            "moving".to_string()
        } else {
            "idle".to_string()
        };
        let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        bucket.insert(
            "CharacterController".to_string(),
            json!({
                "radius": controller.radius,
                "height": controller.height,
                "speed": controller.speed,
                "jump_strength": controller.jump_strength,
                "grounded": controller.grounded,
                "state": controller.state
            }),
        );

        if let Some(entity) = ctx
            .scene
            .entities
            .iter_mut()
            .find(|entity| entity.name == self.entity_name)
        {
            self.previous_translation = Some(entity.translation);
            entity.translation = [
                entity.translation[0] + delta[0],
                entity.translation[1] + delta[1],
                entity.translation[2] + delta[2],
            ];
        }

        Ok(CommandResult::new(
            format!("character '{}' moved", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "input": self.input,
                "dt": self.dt,
                "delta": delta
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_controller) = &self.previous_controller {
            ctx.physics
                .character_controllers
                .insert(self.entity_name.clone(), previous_controller.clone());
            let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
            bucket.insert(
                "CharacterController".to_string(),
                json!({
                    "radius": previous_controller.radius,
                    "height": previous_controller.height,
                    "speed": previous_controller.speed,
                    "jump_strength": previous_controller.jump_strength,
                    "grounded": previous_controller.grounded,
                    "state": previous_controller.state
                }),
            );
        }
        if let Some(previous_translation) = self.previous_translation
            && let Some(entity) = ctx
                .scene
                .entities
                .iter_mut()
                .find(|entity| entity.name == self.entity_name)
        {
            entity.translation = previous_translation;
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "input": self.input,
            "dt": self.dt
        })
    }
}

#[derive(Debug, Clone)]
pub struct PhysCharacterJumpCommand {
    entity_name: String,
    strength: Option<f32>,
    previous_translation: Option<[f32; 3]>,
    previous_controller: Option<PhysicsCharacterController>,
}

impl PhysCharacterJumpCommand {
    pub fn new(entity_name: impl Into<String>, strength: Option<f32>) -> Self {
        Self {
            entity_name: entity_name.into(),
            strength,
            previous_translation: None,
            previous_controller: None,
        }
    }
}

impl EngineCommand for PhysCharacterJumpCommand {
    fn name(&self) -> &'static str {
        "phys.character_jump"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        if !ctx
            .physics
            .character_controllers
            .contains_key(&self.entity_name)
        {
            return ValidationResult::invalid(format!(
                "entity '{}' has no character controller; call phys.add_character_controller first",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let controller = ctx
            .physics
            .character_controllers
            .get_mut(&self.entity_name)
            .with_context(|| {
                format!("entity '{}' has no character controller", self.entity_name)
            })?;
        self.previous_controller = Some(controller.clone());
        let jump_strength = self.strength.unwrap_or(controller.jump_strength).max(0.01);
        let jump_delta = jump_strength * 0.1;
        controller.grounded = false;
        controller.state = "jumping".to_string();

        let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        bucket.insert(
            "CharacterController".to_string(),
            json!({
                "radius": controller.radius,
                "height": controller.height,
                "speed": controller.speed,
                "jump_strength": controller.jump_strength,
                "grounded": controller.grounded,
                "state": controller.state
            }),
        );

        if let Some(entity) = ctx
            .scene
            .entities
            .iter_mut()
            .find(|entity| entity.name == self.entity_name)
        {
            self.previous_translation = Some(entity.translation);
            entity.translation[1] += jump_delta;
        }

        Ok(CommandResult::new(
            format!("character '{}' jumped", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "strength": jump_strength,
                "delta_y": jump_delta
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_controller) = &self.previous_controller {
            ctx.physics
                .character_controllers
                .insert(self.entity_name.clone(), previous_controller.clone());
            let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
            bucket.insert(
                "CharacterController".to_string(),
                json!({
                    "radius": previous_controller.radius,
                    "height": previous_controller.height,
                    "speed": previous_controller.speed,
                    "jump_strength": previous_controller.jump_strength,
                    "grounded": previous_controller.grounded,
                    "state": previous_controller.state
                }),
            );
        }
        if let Some(previous_translation) = self.previous_translation
            && let Some(entity) = ctx
                .scene
                .entities
                .iter_mut()
                .find(|entity| entity.name == self.entity_name)
        {
            entity.translation = previous_translation;
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "strength": self.strength
        })
    }
}

#[derive(Debug, Clone)]
pub struct PhysCharacterSetStateCommand {
    entity_name: String,
    state: String,
    previous_controller: Option<PhysicsCharacterController>,
}

impl PhysCharacterSetStateCommand {
    pub fn new(entity_name: impl Into<String>, state: impl Into<String>) -> Self {
        Self {
            entity_name: entity_name.into(),
            state: normalize_character_state(state.into()),
            previous_controller: None,
        }
    }
}

impl EngineCommand for PhysCharacterSetStateCommand {
    fn name(&self) -> &'static str {
        "phys.character_set_state"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        if !ctx
            .physics
            .character_controllers
            .contains_key(&self.entity_name)
        {
            return ValidationResult::invalid(format!(
                "entity '{}' has no character controller; call phys.add_character_controller first",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let controller = ctx
            .physics
            .character_controllers
            .get_mut(&self.entity_name)
            .with_context(|| {
                format!("entity '{}' has no character controller", self.entity_name)
            })?;
        self.previous_controller = Some(controller.clone());
        controller.state = self.state.clone();
        let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        bucket.insert(
            "CharacterController".to_string(),
            json!({
                "radius": controller.radius,
                "height": controller.height,
                "speed": controller.speed,
                "jump_strength": controller.jump_strength,
                "grounded": controller.grounded,
                "state": controller.state
            }),
        );
        Ok(CommandResult::new(
            format!("character '{}' state updated", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "state": self.state
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_controller) = &self.previous_controller {
            ctx.physics
                .character_controllers
                .insert(self.entity_name.clone(), previous_controller.clone());
            let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
            bucket.insert(
                "CharacterController".to_string(),
                json!({
                    "radius": previous_controller.radius,
                    "height": previous_controller.height,
                    "speed": previous_controller.speed,
                    "jump_strength": previous_controller.jump_strength,
                    "grounded": previous_controller.grounded,
                    "state": previous_controller.state
                }),
            );
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "state": self.state
        })
    }
}

#[derive(Debug, Clone)]
pub struct GameCreateWeaponCommand {
    weapon: GameplayWeaponRecord,
    previous_weapon: Option<GameplayWeaponRecord>,
}

impl GameCreateWeaponCommand {
    pub fn new(
        weapon_id: impl Into<String>,
        rate: f32,
        recoil: f32,
        spread: f32,
        ammo_capacity: u32,
    ) -> Self {
        let ammo_capacity = ammo_capacity.max(1);
        Self {
            weapon: GameplayWeaponRecord {
                weapon_id: weapon_id.into(),
                rate: rate.max(0.05),
                recoil: recoil.max(0.0),
                spread: spread.max(0.0),
                ammo_current: ammo_capacity,
                ammo_capacity,
            },
            previous_weapon: None,
        }
    }
}

impl EngineCommand for GameCreateWeaponCommand {
    fn name(&self) -> &'static str {
        "game.create_weapon"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.weapon.weapon_id.trim().is_empty() {
            return ValidationResult::invalid("weapon_id cannot be empty");
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_weapon = ctx
            .gameplay
            .weapons
            .insert(self.weapon.weapon_id.clone(), self.weapon.clone());
        Ok(CommandResult::new(
            format!("weapon '{}' configured", self.weapon.weapon_id),
            json!({
                "weapon": self.weapon
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        match &self.previous_weapon {
            Some(previous_weapon) => {
                ctx.gameplay
                    .weapons
                    .insert(previous_weapon.weapon_id.clone(), previous_weapon.clone());
            }
            None => {
                ctx.gameplay.weapons.remove(&self.weapon.weapon_id);
                ctx.gameplay
                    .attachments
                    .retain(|_, weapon_id| weapon_id != &self.weapon.weapon_id);
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "weapon_id": self.weapon.weapon_id,
            "rate": self.weapon.rate,
            "recoil": self.weapon.recoil,
            "spread": self.weapon.spread,
            "ammo_capacity": self.weapon.ammo_capacity
        })
    }
}

#[derive(Debug, Clone)]
pub struct GameAttachWeaponCommand {
    entity_name: String,
    weapon_id: String,
    previous_attachment: Option<String>,
}

impl GameAttachWeaponCommand {
    pub fn new(entity_name: impl Into<String>, weapon_id: impl Into<String>) -> Self {
        Self {
            entity_name: entity_name.into(),
            weapon_id: weapon_id.into(),
            previous_attachment: None,
        }
    }
}

impl EngineCommand for GameAttachWeaponCommand {
    fn name(&self) -> &'static str {
        "game.attach_weapon"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("character_id cannot be empty");
        }
        if self.weapon_id.trim().is_empty() {
            return ValidationResult::invalid("weapon_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        if !ctx.gameplay.weapons.contains_key(&self.weapon_id) {
            return ValidationResult::invalid(format!(
                "weapon '{}' does not exist; call game.create_weapon first",
                self.weapon_id
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_attachment = ctx
            .gameplay
            .attachments
            .insert(self.entity_name.clone(), self.weapon_id.clone());
        Ok(CommandResult::new(
            format!(
                "weapon '{}' attached to '{}'",
                self.weapon_id, self.entity_name
            ),
            json!({
                "character_id": self.entity_name,
                "weapon_id": self.weapon_id
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        match &self.previous_attachment {
            Some(previous_weapon) => {
                ctx.gameplay
                    .attachments
                    .insert(self.entity_name.clone(), previous_weapon.clone());
            }
            None => {
                ctx.gameplay.attachments.remove(&self.entity_name);
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "character_id": self.entity_name,
            "weapon_id": self.weapon_id
        })
    }
}

#[derive(Debug, Clone)]
pub struct GameFireWeaponCommand {
    weapon_id: String,
    previous_weapon: Option<GameplayWeaponRecord>,
    previous_fire_events: Option<u64>,
    previous_last_message: Option<Option<String>>,
}

impl GameFireWeaponCommand {
    pub fn new(weapon_id: impl Into<String>) -> Self {
        Self {
            weapon_id: weapon_id.into(),
            previous_weapon: None,
            previous_fire_events: None,
            previous_last_message: None,
        }
    }
}

impl EngineCommand for GameFireWeaponCommand {
    fn name(&self) -> &'static str {
        "game.fire_weapon"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.weapon_id.trim().is_empty() {
            return ValidationResult::invalid("weapon_id cannot be empty");
        }
        let Some(weapon) = ctx.gameplay.weapons.get(&self.weapon_id) else {
            return ValidationResult::invalid(format!(
                "weapon '{}' does not exist; call game.create_weapon first",
                self.weapon_id
            ));
        };
        if weapon.ammo_current == 0 {
            return ValidationResult::invalid(format!(
                "weapon '{}' is out of ammo",
                self.weapon_id
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let weapon = ctx
            .gameplay
            .weapons
            .get_mut(&self.weapon_id)
            .with_context(|| format!("weapon '{}' not found", self.weapon_id))?;
        self.previous_weapon = Some(weapon.clone());
        self.previous_fire_events = Some(ctx.gameplay.fire_events);
        self.previous_last_message = Some(ctx.scene_runtime.last_message.clone());
        weapon.ammo_current = weapon.ammo_current.saturating_sub(1);
        ctx.gameplay.fire_events = ctx.gameplay.fire_events.saturating_add(1);
        ctx.scene_runtime.last_message = Some(format!(
            "weapon '{}' fired (ammo {}/{})",
            self.weapon_id, weapon.ammo_current, weapon.ammo_capacity
        ));
        Ok(CommandResult::new(
            format!("weapon '{}' fired", self.weapon_id),
            json!({
                "weapon_id": self.weapon_id,
                "ammo_current": weapon.ammo_current,
                "ammo_capacity": weapon.ammo_capacity,
                "fire_events": ctx.gameplay.fire_events
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_weapon) = &self.previous_weapon {
            ctx.gameplay
                .weapons
                .insert(previous_weapon.weapon_id.clone(), previous_weapon.clone());
        }
        if let Some(previous_fire_events) = self.previous_fire_events {
            ctx.gameplay.fire_events = previous_fire_events;
        }
        if let Some(previous_last_message) = &self.previous_last_message {
            ctx.scene_runtime.last_message = previous_last_message.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "weapon_id": self.weapon_id
        })
    }
}

#[derive(Debug, Clone)]
pub struct GameAddHealthComponentCommand {
    entity_name: String,
    max_health: f32,
    current_health: f32,
    previous_value: Option<Value>,
}

impl GameAddHealthComponentCommand {
    pub fn new(entity_name: impl Into<String>, max_health: f32, current_health: f32) -> Self {
        let max_health = max_health.max(1.0);
        Self {
            entity_name: entity_name.into(),
            max_health,
            current_health: current_health.clamp(0.0, max_health),
            previous_value: None,
        }
    }
}

impl EngineCommand for GameAddHealthComponentCommand {
    fn name(&self) -> &'static str {
        "game.add_health_component"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        self.previous_value = bucket.insert(
            "Health".to_string(),
            json!({
                "value": self.current_health,
                "max": self.max_health
            }),
        );
        Ok(CommandResult::new(
            format!("health component set on '{}'", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "health": {
                    "value": self.current_health,
                    "max": self.max_health
                }
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(bucket) = ctx.components.get_mut(&self.entity_name) {
            match &self.previous_value {
                Some(previous_value) => {
                    bucket.insert("Health".to_string(), previous_value.clone());
                }
                None => {
                    bucket.remove("Health");
                }
            }
            if bucket.is_empty() {
                ctx.components.remove(&self.entity_name);
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "max_health": self.max_health,
            "current_health": self.current_health
        })
    }
}

#[derive(Debug, Clone)]
pub struct GameApplyDamageCommand {
    entity_name: String,
    amount: f32,
    damage_type: String,
    previous_value: Option<Value>,
    previous_total_damage: Option<f32>,
}

impl GameApplyDamageCommand {
    pub fn new(
        entity_name: impl Into<String>,
        amount: f32,
        damage_type: impl Into<String>,
    ) -> Self {
        Self {
            entity_name: entity_name.into(),
            amount: amount.max(0.0),
            damage_type: damage_type.into(),
            previous_value: None,
            previous_total_damage: None,
        }
    }
}

impl EngineCommand for GameApplyDamageCommand {
    fn name(&self) -> &'static str {
        "game.apply_damage"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("target_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        self.previous_value = bucket.get("Health").cloned();
        self.previous_total_damage = Some(ctx.gameplay.total_damage_applied);

        let current = bucket
            .get("Health")
            .and_then(|value| value.get("value"))
            .and_then(Value::as_f64)
            .unwrap_or(100.0) as f32;
        let max = bucket
            .get("Health")
            .and_then(|value| value.get("max"))
            .and_then(Value::as_f64)
            .unwrap_or(current.max(100.0) as f64) as f32;
        let next = (current - self.amount).max(0.0);
        bucket.insert(
            "Health".to_string(),
            json!({
                "value": next,
                "max": max
            }),
        );
        ctx.gameplay.total_damage_applied += self.amount.max(0.0);
        ctx.scene_runtime.last_message = Some(format!(
            "'{}' received {:.1} {} damage",
            self.entity_name, self.amount, self.damage_type
        ));

        Ok(CommandResult::new(
            format!("damage applied to '{}'", self.entity_name),
            json!({
                "target_id": self.entity_name,
                "amount": self.amount,
                "damage_type": self.damage_type,
                "health": {
                    "value": next,
                    "max": max
                },
                "total_damage_applied": ctx.gameplay.total_damage_applied
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(bucket) = ctx.components.get_mut(&self.entity_name) {
            match &self.previous_value {
                Some(previous_value) => {
                    bucket.insert("Health".to_string(), previous_value.clone());
                }
                None => {
                    bucket.remove("Health");
                }
            }
        }
        if let Some(previous_total_damage) = self.previous_total_damage {
            ctx.gameplay.total_damage_applied = previous_total_damage;
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "target_id": self.entity_name,
            "amount": self.amount,
            "damage_type": self.damage_type
        })
    }
}

#[derive(Debug, Clone)]
pub struct GameCreateInputActionCommand {
    action: GameplayInputActionRecord,
    previous_action: Option<GameplayInputActionRecord>,
}

impl GameCreateInputActionCommand {
    pub fn new(name: impl Into<String>, bindings: Vec<String>) -> Self {
        let mut normalized_bindings = bindings
            .into_iter()
            .map(|binding| binding.trim().to_string())
            .filter(|binding| !binding.is_empty())
            .collect::<Vec<String>>();
        normalized_bindings.sort();
        normalized_bindings.dedup();
        Self {
            action: GameplayInputActionRecord {
                name: name.into(),
                bindings: normalized_bindings,
                target_event: None,
            },
            previous_action: None,
        }
    }
}

impl EngineCommand for GameCreateInputActionCommand {
    fn name(&self) -> &'static str {
        "game.create_input_action"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.action.name.trim().is_empty() {
            return ValidationResult::invalid("name cannot be empty");
        }
        if self.action.bindings.is_empty() {
            return ValidationResult::invalid("bindings cannot be empty");
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_action = ctx
            .gameplay
            .input_actions
            .insert(self.action.name.clone(), self.action.clone());
        Ok(CommandResult::new(
            format!("input action '{}' configured", self.action.name),
            json!({
                "action": self.action
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        match &self.previous_action {
            Some(previous_action) => {
                ctx.gameplay
                    .input_actions
                    .insert(previous_action.name.clone(), previous_action.clone());
            }
            None => {
                ctx.gameplay.input_actions.remove(&self.action.name);
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "name": self.action.name,
            "bindings": self.action.bindings
        })
    }
}

#[derive(Debug, Clone)]
pub struct GameBindActionCommand {
    action_name: String,
    target_event: String,
    previous_action: Option<GameplayInputActionRecord>,
}

impl GameBindActionCommand {
    pub fn new(action_name: impl Into<String>, target_event: impl Into<String>) -> Self {
        Self {
            action_name: action_name.into(),
            target_event: target_event.into(),
            previous_action: None,
        }
    }
}

impl EngineCommand for GameBindActionCommand {
    fn name(&self) -> &'static str {
        "game.bind_action"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.action_name.trim().is_empty() {
            return ValidationResult::invalid("name cannot be empty");
        }
        if self.target_event.trim().is_empty() {
            return ValidationResult::invalid("target_script_event cannot be empty");
        }
        if !ctx.gameplay.input_actions.contains_key(&self.action_name) {
            return ValidationResult::invalid(format!(
                "input action '{}' not found; call game.create_input_action first",
                self.action_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let action = ctx
            .gameplay
            .input_actions
            .get_mut(&self.action_name)
            .with_context(|| format!("input action '{}' not found", self.action_name))?;
        self.previous_action = Some(action.clone());
        action.target_event = Some(self.target_event.clone());
        Ok(CommandResult::new(
            format!("action '{}' bound", self.action_name),
            json!({
                "name": self.action_name,
                "target_script_event": self.target_event
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_action) = &self.previous_action {
            ctx.gameplay
                .input_actions
                .insert(previous_action.name.clone(), previous_action.clone());
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "name": self.action_name,
            "target_script_event": self.target_event
        })
    }
}

#[derive(Debug, Clone)]
pub struct GameSetRebindCommand {
    action_name: String,
    binding: String,
    previous_action: Option<GameplayInputActionRecord>,
}

impl GameSetRebindCommand {
    pub fn new(action_name: impl Into<String>, binding: impl Into<String>) -> Self {
        Self {
            action_name: action_name.into(),
            binding: binding.into(),
            previous_action: None,
        }
    }
}

impl EngineCommand for GameSetRebindCommand {
    fn name(&self) -> &'static str {
        "game.set_rebind"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.action_name.trim().is_empty() {
            return ValidationResult::invalid("action cannot be empty");
        }
        if self.binding.trim().is_empty() {
            return ValidationResult::invalid("binding cannot be empty");
        }
        if !ctx.gameplay.input_actions.contains_key(&self.action_name) {
            return ValidationResult::invalid(format!(
                "input action '{}' not found; call game.create_input_action first",
                self.action_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let action = ctx
            .gameplay
            .input_actions
            .get_mut(&self.action_name)
            .with_context(|| format!("input action '{}' not found", self.action_name))?;
        self.previous_action = Some(action.clone());
        action.bindings.retain(|existing| existing != &self.binding);
        action.bindings.insert(0, self.binding.clone());
        Ok(CommandResult::new(
            format!("action '{}' rebound", self.action_name),
            json!({
                "action": self.action_name,
                "binding": self.binding,
                "bindings": action.bindings
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_action) = &self.previous_action {
            ctx.gameplay
                .input_actions
                .insert(previous_action.name.clone(), previous_action.clone());
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "action": self.action_name,
            "binding": self.binding
        })
    }
}

#[derive(Debug, Clone)]
pub struct GameAddTriggerCommand {
    entity_name: String,
    trigger: GameplayTriggerRecord,
    previous_trigger: Option<GameplayTriggerRecord>,
    previous_component: Option<Value>,
}

impl GameAddTriggerCommand {
    pub fn new(
        entity_name: impl Into<String>,
        shape: impl Into<String>,
        radius: f32,
        params: Value,
    ) -> Self {
        let entity_name = entity_name.into();
        Self {
            trigger: GameplayTriggerRecord {
                entity_id: entity_name.clone(),
                shape: normalize_collider_shape(shape.into()),
                radius: radius.max(0.01),
                params,
            },
            entity_name,
            previous_trigger: None,
            previous_component: None,
        }
    }
}

impl EngineCommand for GameAddTriggerCommand {
    fn name(&self) -> &'static str {
        "game.add_trigger"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_trigger = ctx
            .gameplay
            .triggers
            .insert(self.entity_name.clone(), self.trigger.clone());
        self.previous_component = ctx
            .components
            .get(&self.entity_name)
            .and_then(|bucket| bucket.get("Trigger"))
            .cloned();
        let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        bucket.insert(
            "Trigger".to_string(),
            json!({
                "shape": self.trigger.shape,
                "radius": self.trigger.radius,
                "params": self.trigger.params
            }),
        );
        Ok(CommandResult::new(
            format!("trigger configured on '{}'", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "trigger": self.trigger
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        match &self.previous_trigger {
            Some(previous_trigger) => {
                ctx.gameplay
                    .triggers
                    .insert(self.entity_name.clone(), previous_trigger.clone());
            }
            None => {
                ctx.gameplay.triggers.remove(&self.entity_name);
            }
        }
        match &self.previous_component {
            Some(previous_component) => {
                let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
                bucket.insert("Trigger".to_string(), previous_component.clone());
            }
            None => {
                if let Some(bucket) = ctx.components.get_mut(&self.entity_name) {
                    bucket.remove("Trigger");
                }
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "shape": self.trigger.shape,
            "radius": self.trigger.radius,
            "params": self.trigger.params
        })
    }
}

#[derive(Debug, Clone)]
pub struct GameAddPickupCommand {
    entity_name: String,
    pickup: GameplayPickupRecord,
    previous_pickup: Option<GameplayPickupRecord>,
    previous_component: Option<Value>,
}

impl GameAddPickupCommand {
    pub fn new(entity_name: impl Into<String>, item_data: Value) -> Self {
        let entity_name = entity_name.into();
        Self {
            pickup: GameplayPickupRecord {
                entity_id: entity_name.clone(),
                item_data,
            },
            entity_name,
            previous_pickup: None,
            previous_component: None,
        }
    }
}

impl EngineCommand for GameAddPickupCommand {
    fn name(&self) -> &'static str {
        "game.add_pickup"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_pickup = ctx
            .gameplay
            .pickups
            .insert(self.entity_name.clone(), self.pickup.clone());
        self.previous_component = ctx
            .components
            .get(&self.entity_name)
            .and_then(|bucket| bucket.get("Pickup"))
            .cloned();
        let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        bucket.insert(
            "Pickup".to_string(),
            json!({
                "item_data": self.pickup.item_data
            }),
        );
        Ok(CommandResult::new(
            format!("pickup configured on '{}'", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "pickup": self.pickup
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        match &self.previous_pickup {
            Some(previous_pickup) => {
                ctx.gameplay
                    .pickups
                    .insert(self.entity_name.clone(), previous_pickup.clone());
            }
            None => {
                ctx.gameplay.pickups.remove(&self.entity_name);
            }
        }
        match &self.previous_component {
            Some(previous_component) => {
                let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
                bucket.insert("Pickup".to_string(), previous_component.clone());
            }
            None => {
                if let Some(bucket) = ctx.components.get_mut(&self.entity_name) {
                    bucket.remove("Pickup");
                }
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "item_data": self.pickup.item_data
        })
    }
}

#[derive(Debug, Clone)]
pub struct GameAddInventoryCommand {
    entity_name: String,
    inventory: GameplayInventoryRecord,
    previous_inventory: Option<GameplayInventoryRecord>,
    previous_component: Option<Value>,
}

impl GameAddInventoryCommand {
    pub fn new(entity_name: impl Into<String>, capacity: u32, items: Vec<String>) -> Self {
        let entity_name = entity_name.into();
        Self {
            inventory: GameplayInventoryRecord {
                entity_id: entity_name.clone(),
                capacity: capacity.max(1),
                items,
            },
            entity_name,
            previous_inventory: None,
            previous_component: None,
        }
    }
}

impl EngineCommand for GameAddInventoryCommand {
    fn name(&self) -> &'static str {
        "game.add_inventory"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_inventory = ctx
            .gameplay
            .inventories
            .insert(self.entity_name.clone(), self.inventory.clone());
        self.previous_component = ctx
            .components
            .get(&self.entity_name)
            .and_then(|bucket| bucket.get("Inventory"))
            .cloned();
        let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        bucket.insert(
            "Inventory".to_string(),
            json!({
                "capacity": self.inventory.capacity,
                "items": self.inventory.items
            }),
        );
        Ok(CommandResult::new(
            format!("inventory configured on '{}'", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "inventory": self.inventory
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        match &self.previous_inventory {
            Some(previous_inventory) => {
                ctx.gameplay
                    .inventories
                    .insert(self.entity_name.clone(), previous_inventory.clone());
            }
            None => {
                ctx.gameplay.inventories.remove(&self.entity_name);
            }
        }
        match &self.previous_component {
            Some(previous_component) => {
                let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
                bucket.insert("Inventory".to_string(), previous_component.clone());
            }
            None => {
                if let Some(bucket) = ctx.components.get_mut(&self.entity_name) {
                    bucket.remove("Inventory");
                }
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "capacity": self.inventory.capacity,
            "items": self.inventory.items
        })
    }
}

#[derive(Debug, Clone)]
pub struct GameAddInteractableCommand {
    entity_name: String,
    interactable: GameplayInteractableRecord,
    previous_interactable: Option<GameplayInteractableRecord>,
    previous_component: Option<Value>,
}

impl GameAddInteractableCommand {
    pub fn new(
        entity_name: impl Into<String>,
        prompt: impl Into<String>,
        actions: Vec<String>,
    ) -> Self {
        let entity_name = entity_name.into();
        Self {
            interactable: GameplayInteractableRecord {
                entity_id: entity_name.clone(),
                prompt: prompt.into(),
                actions,
            },
            entity_name,
            previous_interactable: None,
            previous_component: None,
        }
    }
}

impl EngineCommand for GameAddInteractableCommand {
    fn name(&self) -> &'static str {
        "game.add_interactable"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        if self.interactable.prompt.trim().is_empty() {
            return ValidationResult::invalid("prompt cannot be empty");
        }
        if !ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                self.entity_name
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_interactable = ctx
            .gameplay
            .interactables
            .insert(self.entity_name.clone(), self.interactable.clone());
        self.previous_component = ctx
            .components
            .get(&self.entity_name)
            .and_then(|bucket| bucket.get("Interactable"))
            .cloned();
        let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
        bucket.insert(
            "Interactable".to_string(),
            json!({
                "prompt": self.interactable.prompt,
                "actions": self.interactable.actions
            }),
        );
        Ok(CommandResult::new(
            format!("interactable configured on '{}'", self.entity_name),
            json!({
                "entity_id": self.entity_name,
                "interactable": self.interactable
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        match &self.previous_interactable {
            Some(previous_interactable) => {
                ctx.gameplay
                    .interactables
                    .insert(self.entity_name.clone(), previous_interactable.clone());
            }
            None => {
                ctx.gameplay.interactables.remove(&self.entity_name);
            }
        }
        match &self.previous_component {
            Some(previous_component) => {
                let bucket = ctx.components.entry(self.entity_name.clone()).or_default();
                bucket.insert("Interactable".to_string(), previous_component.clone());
            }
            None => {
                if let Some(bucket) = ctx.components.get_mut(&self.entity_name) {
                    bucket.remove("Interactable");
                }
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "entity_id": self.entity_name,
            "prompt": self.interactable.prompt,
            "actions": self.interactable.actions
        })
    }
}

#[derive(Debug, Clone)]
pub struct AnimMutationCommand {
    operation: String,
    params: Value,
    previous_animation: Option<AnimationRuntimeState>,
    previous_components: Option<HashMap<String, BTreeMap<String, Value>>>,
    previous_last_message: Option<Option<String>>,
}

impl AnimMutationCommand {
    pub fn new(operation: impl Into<String>, params: Value) -> Self {
        Self {
            operation: operation.into(),
            params,
            previous_animation: None,
            previous_components: None,
            previous_last_message: None,
        }
    }

    fn op(&self) -> String {
        self.operation.trim().to_ascii_lowercase()
    }
}

impl EngineCommand for AnimMutationCommand {
    fn name(&self) -> &'static str {
        "anim.mutation"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        let op = self.op();
        let str_param =
            |key: &str| -> Option<&str> { self.params.get(key).and_then(Value::as_str) };
        let non_empty = |key: &str| -> bool {
            str_param(key)
                .map(str::trim)
                .map(|v| !v.is_empty())
                .unwrap_or(false)
        };
        if op == "create_state_machine" {
            if !non_empty("name") {
                return ValidationResult::invalid("name cannot be empty");
            }
            return ValidationResult::ok();
        }
        if op == "retarget" {
            if !non_empty("source_rig") || !non_empty("target_rig") {
                return ValidationResult::invalid("source_rig/target_rig cannot be empty");
            }
            return ValidationResult::ok();
        }

        if op == "add_state" || op == "add_transition" || op == "set_parameter" {
            if !non_empty("controller_id") {
                return ValidationResult::invalid("controller_id cannot be empty");
            }
            let controller_id = str_param("controller_id").unwrap_or_default().trim();
            if !ctx.animation.state_machines.contains_key(controller_id) {
                return ValidationResult::invalid(format!(
                    "controller '{}' does not exist; call anim.create_state_machine first",
                    controller_id
                ));
            }
            if op == "add_state" {
                if !non_empty("state_name") || !non_empty("clip_id") {
                    return ValidationResult::invalid("state_name/clip_id cannot be empty");
                }
            } else if op == "add_transition" {
                if !non_empty("from") || !non_empty("to") {
                    return ValidationResult::invalid("from/to cannot be empty");
                }
                let Some(sm) = ctx.animation.state_machines.get(controller_id) else {
                    return ValidationResult::invalid("controller not found");
                };
                let from = str_param("from").unwrap_or_default().trim();
                let to = str_param("to").unwrap_or_default().trim();
                if !sm.states.contains_key(from) || !sm.states.contains_key(to) {
                    return ValidationResult::invalid("from/to states must exist in controller");
                }
            } else if op == "set_parameter" && !non_empty("key") {
                return ValidationResult::invalid("key cannot be empty");
            }
            return ValidationResult::ok();
        }

        if !non_empty("entity_id") {
            return ValidationResult::invalid("entity_id cannot be empty");
        }
        let entity_id = str_param("entity_id").unwrap_or_default().trim();
        if !ctx.entity_exists(entity_id) {
            return ValidationResult::invalid(format!(
                "entity '{}' does not exist in open scene",
                entity_id
            ));
        }

        if op == "add_animator" {
            if !non_empty("controller_id") {
                return ValidationResult::invalid("controller_id cannot be empty");
            }
            let controller_id = str_param("controller_id").unwrap_or_default().trim();
            if !ctx.animation.state_machines.contains_key(controller_id) {
                return ValidationResult::invalid(format!(
                    "controller '{}' does not exist; call anim.create_state_machine first",
                    controller_id
                ));
            }
        } else if op == "play" {
            if !non_empty("clip_id") {
                return ValidationResult::invalid("clip_id cannot be empty");
            }
        } else if op == "blend" {
            if !non_empty("clip_a") || !non_empty("clip_b") {
                return ValidationResult::invalid("clip_a/clip_b cannot be empty");
            }
        } else if op == "add_ik" {
            if !non_empty("chain") {
                return ValidationResult::invalid("chain cannot be empty");
            }
        } else if op == "bake_animation" {
            return ValidationResult::ok();
        } else {
            return ValidationResult::invalid(format!("unsupported animation operation '{}'", op));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_animation = Some(ctx.animation.clone());
        self.previous_components = Some(ctx.components.clone());
        self.previous_last_message = Some(ctx.scene_runtime.last_message.clone());

        let op = self.op();
        let str_param = |key: &str| -> Option<String> {
            self.params
                .get(key)
                .and_then(Value::as_str)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let mut payload = json!({
            "operation": op
        });
        if op == "create_state_machine" {
            let name = str_param("name").with_context(|| "missing name")?;
            let controller_id =
                str_param("controller_id").unwrap_or_else(|| normalize_identifier(&name));
            ctx.animation.state_machines.insert(
                controller_id.clone(),
                AnimationStateMachineRecord {
                    controller_id: controller_id.clone(),
                    name: name.clone(),
                    states: BTreeMap::new(),
                    transitions: Vec::new(),
                    parameters: BTreeMap::new(),
                },
            );
            payload["controller_id"] = Value::String(controller_id);
            payload["name"] = Value::String(name);
        } else if op == "add_animator" {
            let entity_id = str_param("entity_id").with_context(|| "missing entity_id")?;
            let controller_id =
                str_param("controller_id").with_context(|| "missing controller_id")?;
            ctx.animation
                .entity_animators
                .insert(entity_id.clone(), controller_id.clone());
            let bucket = ctx.components.entry(entity_id.clone()).or_default();
            bucket.insert(
                "Animator".to_string(),
                json!({
                    "controller_id": controller_id
                }),
            );
            payload["entity_id"] = Value::String(entity_id);
            payload["controller_id"] = Value::String(controller_id);
        } else if op == "add_state" {
            let controller_id =
                str_param("controller_id").with_context(|| "missing controller_id")?;
            let state_name = str_param("state_name").with_context(|| "missing state_name")?;
            let clip_id = str_param("clip_id").with_context(|| "missing clip_id")?;
            let sm = ctx
                .animation
                .state_machines
                .get_mut(&controller_id)
                .with_context(|| format!("controller '{}' not found", controller_id))?;
            sm.states.insert(
                state_name.clone(),
                AnimationStateRecord {
                    state_name: state_name.clone(),
                    clip_id: clip_id.clone(),
                },
            );
            payload["controller_id"] = Value::String(controller_id);
            payload["state_name"] = Value::String(state_name);
            payload["clip_id"] = Value::String(clip_id);
        } else if op == "add_transition" {
            let controller_id =
                str_param("controller_id").with_context(|| "missing controller_id")?;
            let from = str_param("from").with_context(|| "missing from")?;
            let to = str_param("to").with_context(|| "missing to")?;
            let conditions = self
                .params
                .get("conditions")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let sm = ctx
                .animation
                .state_machines
                .get_mut(&controller_id)
                .with_context(|| format!("controller '{}' not found", controller_id))?;
            sm.transitions.push(AnimationTransitionRecord {
                from_state: from.clone(),
                to_state: to.clone(),
                conditions: conditions.clone(),
            });
            payload["controller_id"] = Value::String(controller_id);
            payload["from"] = Value::String(from);
            payload["to"] = Value::String(to);
            payload["conditions"] = conditions;
        } else if op == "set_parameter" {
            let controller_id =
                str_param("controller_id").with_context(|| "missing controller_id")?;
            let key = str_param("key").with_context(|| "missing key")?;
            let value = self.params.get("value").cloned().unwrap_or(Value::Null);
            let sm = ctx
                .animation
                .state_machines
                .get_mut(&controller_id)
                .with_context(|| format!("controller '{}' not found", controller_id))?;
            sm.parameters.insert(key.clone(), value.clone());
            payload["controller_id"] = Value::String(controller_id);
            payload["key"] = Value::String(key);
            payload["value"] = value;
        } else if op == "play" {
            let entity_id = str_param("entity_id").with_context(|| "missing entity_id")?;
            let clip_id = str_param("clip_id").with_context(|| "missing clip_id")?;
            ctx.animation
                .entity_active_clips
                .insert(entity_id.clone(), clip_id.clone());
            let bucket = ctx.components.entry(entity_id.clone()).or_default();
            bucket.insert(
                "AnimatorPlayback".to_string(),
                json!({
                    "clip_id": clip_id
                }),
            );
            ctx.scene_runtime.last_message =
                Some(format!("entity '{}' playing '{}'", entity_id, clip_id));
            payload["entity_id"] = Value::String(entity_id);
            payload["clip_id"] = Value::String(clip_id);
        } else if op == "blend" {
            let entity_id = str_param("entity_id").with_context(|| "missing entity_id")?;
            let clip_a = str_param("clip_a").with_context(|| "missing clip_a")?;
            let clip_b = str_param("clip_b").with_context(|| "missing clip_b")?;
            let weight = self
                .params
                .get("weight")
                .and_then(Value::as_f64)
                .map(|value| value as f32)
                .unwrap_or(0.5)
                .clamp(0.0, 1.0);
            ctx.animation.entity_blends.insert(
                entity_id.clone(),
                AnimationBlendRecord {
                    clip_a: clip_a.clone(),
                    clip_b: clip_b.clone(),
                    weight,
                },
            );
            let bucket = ctx.components.entry(entity_id.clone()).or_default();
            bucket.insert(
                "AnimatorBlend".to_string(),
                json!({
                    "clip_a": clip_a,
                    "clip_b": clip_b,
                    "weight": weight
                }),
            );
            payload["entity_id"] = Value::String(entity_id);
        } else if op == "add_ik" {
            let entity_id = str_param("entity_id").with_context(|| "missing entity_id")?;
            let chain = str_param("chain").with_context(|| "missing chain")?;
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.animation.ik_solvers.insert(
                entity_id.clone(),
                AnimationIkRecord {
                    entity_id: entity_id.clone(),
                    chain: chain.clone(),
                    params: params.clone(),
                },
            );
            let bucket = ctx.components.entry(entity_id.clone()).or_default();
            bucket.insert(
                "AnimationIK".to_string(),
                json!({
                    "chain": chain,
                    "params": params
                }),
            );
            payload["entity_id"] = Value::String(entity_id);
        } else if op == "retarget" {
            let source_rig = str_param("source_rig").with_context(|| "missing source_rig")?;
            let target_rig = str_param("target_rig").with_context(|| "missing target_rig")?;
            let mapping = self
                .params
                .get("mapping")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.animation.retarget_jobs.push(AnimationRetargetRecord {
                source_rig: source_rig.clone(),
                target_rig: target_rig.clone(),
                mapping: mapping.clone(),
            });
            payload["source_rig"] = Value::String(source_rig);
            payload["target_rig"] = Value::String(target_rig);
            payload["mapping"] = mapping;
        } else if op == "bake_animation" {
            let entity_id = str_param("entity_id").with_context(|| "missing entity_id")?;
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.animation.bake_jobs.push(AnimationBakeRecord {
                entity_id: entity_id.clone(),
                params: params.clone(),
            });
            payload["entity_id"] = Value::String(entity_id);
            payload["params"] = params;
        } else {
            bail!("unsupported animation operation '{}'", op);
        }

        Ok(CommandResult::new(
            format!("animation operation '{}' applied", op),
            payload,
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_animation) = &self.previous_animation {
            ctx.animation = previous_animation.clone();
        }
        if let Some(previous_components) = &self.previous_components {
            ctx.components = previous_components.clone();
        }
        if let Some(previous_last_message) = &self.previous_last_message {
            ctx.scene_runtime.last_message = previous_last_message.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "operation": self.operation,
            "params": self.params
        })
    }
}

#[derive(Debug, Clone)]
pub struct ModelMutationCommand {
    operation: String,
    params: Value,
    previous_modeling: Option<ModelingRuntimeState>,
    previous_scene: Option<SceneFile>,
}

impl ModelMutationCommand {
    pub fn new(operation: impl Into<String>, params: Value) -> Self {
        Self {
            operation: operation.into(),
            params,
            previous_modeling: None,
            previous_scene: None,
        }
    }

    fn op(&self) -> String {
        self.operation.trim().to_ascii_lowercase()
    }
}

impl EngineCommand for ModelMutationCommand {
    fn name(&self) -> &'static str {
        "model.mutation"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        let op = self.op();
        let str_param =
            |key: &str| -> Option<&str> { self.params.get(key).and_then(Value::as_str) };
        let non_empty = |key: &str| -> bool {
            str_param(key)
                .map(str::trim)
                .map(|v| !v.is_empty())
                .unwrap_or(false)
        };
        if op == "create_primitive" {
            if !non_empty("type") {
                return ValidationResult::invalid("type cannot be empty");
            }
            if !non_empty("name") {
                return ValidationResult::invalid("name cannot be empty");
            }
            return ValidationResult::ok();
        }

        if !non_empty("mesh_id") {
            return ValidationResult::invalid("mesh_id cannot be empty");
        }
        let mesh_id = str_param("mesh_id").unwrap_or_default().trim();
        if !model_mesh_exists(ctx, mesh_id) {
            return ValidationResult::invalid(format!("mesh '{}' does not exist", mesh_id));
        }

        if op == "select" {
            if !non_empty("mode") {
                return ValidationResult::invalid("mode cannot be empty");
            }
        } else if op == "add_modifier" {
            if !non_empty("type") {
                return ValidationResult::invalid("type cannot be empty for add_modifier");
            }
        } else if ["set_modifier", "apply_modifier", "remove_modifier"].contains(&op.as_str()) {
            if !non_empty("modifier_id") {
                return ValidationResult::invalid("modifier_id cannot be empty");
            }
        } else if op == "sculpt_brush" && !non_empty("brush_type") {
            return ValidationResult::invalid("brush_type cannot be empty");
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_modeling = Some(ctx.modeling.clone());
        self.previous_scene = Some(ctx.scene.clone());

        let op = self.op();
        let str_param = |key: &str| -> Option<String> {
            self.params
                .get(key)
                .and_then(Value::as_str)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let mut payload = json!({
            "operation": op
        });

        if op == "create_primitive" {
            let primitive_type = normalize_model_primitive_type(
                str_param("type").with_context(|| "missing type for model.create_primitive")?,
            );
            let desired_name = str_param("name").with_context(|| "missing name")?;
            let translation =
                parse_optional_vec3(self.params.get("translation")).unwrap_or([0.0, 0.0, 0.0]);
            let mesh_id = str_param("mesh_id").unwrap_or_else(|| {
                format!(
                    "mesh_{}_{}",
                    normalize_identifier(&primitive_type),
                    ctx.modeling.meshes.len() + 1
                )
            });
            let entity_name = unique_entity_name(&ctx.scene, &desired_name);
            ctx.scene.entities.push(SceneEntity {
                name: entity_name.clone(),
                mesh: mesh_id.clone(),
                translation,
            });
            let (vertex_count, face_count) = primitive_mesh_stats(&primitive_type);
            ctx.modeling.meshes.insert(
                mesh_id.clone(),
                ModelMeshRecord {
                    mesh_id: mesh_id.clone(),
                    primitive_type: Some(primitive_type.clone()),
                    vertex_count,
                    face_count,
                },
            );
            ctx.modeling.operation_log.push(ModelOperationRecord {
                tool: "model.create_primitive".to_string(),
                mesh_id: mesh_id.clone(),
                payload: json!({
                    "entity_id": entity_name,
                    "primitive_type": primitive_type
                }),
            });
            payload["entity_id"] = Value::String(entity_name);
            payload["mesh_id"] = Value::String(mesh_id);
            payload["type"] = Value::String(primitive_type);
        } else {
            let mesh_id = str_param("mesh_id").with_context(|| "missing mesh_id")?;
            ensure_model_mesh_record(ctx, &mesh_id)?;
            if op == "enter_edit_mode" {
                ctx.modeling.edit_modes.insert(mesh_id.clone());
            } else if op == "exit_edit_mode" {
                ctx.modeling.edit_modes.remove(&mesh_id);
            } else if op == "select" {
                let mode = str_param("mode").with_context(|| "missing mode")?;
                let selector = self.params.get("selector").cloned().unwrap_or(Value::Null);
                ctx.modeling
                    .selections
                    .insert(mesh_id.clone(), ModelSelectionRecord { mode, selector });
            } else if [
                "extrude",
                "inset",
                "bevel",
                "loop_cut",
                "knife",
                "merge",
                "subdivide",
                "triangulate",
                "voxel_remesh",
                "decimate",
                "smooth",
            ]
            .contains(&op.as_str())
            {
                let mesh = ctx
                    .modeling
                    .meshes
                    .get_mut(&mesh_id)
                    .with_context(|| format!("mesh '{}' not found", mesh_id))?;
                apply_model_topology_operation(mesh, &op, &self.params);
            } else if [
                "add_modifier",
                "set_modifier",
                "apply_modifier",
                "remove_modifier",
            ]
            .contains(&op.as_str())
            {
                let list = ctx.modeling.modifiers.entry(mesh_id.clone()).or_default();
                if op == "add_modifier" {
                    let modifier_type = str_param("type").with_context(|| "missing type")?;
                    let modifier_id = str_param("modifier_id").unwrap_or_else(|| {
                        format!(
                            "{}_{}",
                            normalize_identifier(&modifier_type),
                            list.len() + 1
                        )
                    });
                    let params = self
                        .params
                        .get("params")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    list.push(ModelModifierRecord {
                        modifier_id: modifier_id.clone(),
                        modifier_type,
                        params,
                        applied: false,
                    });
                    payload["modifier_id"] = Value::String(modifier_id);
                } else if op == "set_modifier" {
                    let modifier_id =
                        str_param("modifier_id").with_context(|| "missing modifier_id")?;
                    let params = self
                        .params
                        .get("params")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    let modifier = list
                        .iter_mut()
                        .find(|item| item.modifier_id == modifier_id)
                        .with_context(|| format!("modifier '{}' not found", modifier_id))?;
                    modifier.params = params;
                } else if op == "apply_modifier" {
                    let modifier_id =
                        str_param("modifier_id").with_context(|| "missing modifier_id")?;
                    let modifier = list
                        .iter_mut()
                        .find(|item| item.modifier_id == modifier_id)
                        .with_context(|| format!("modifier '{}' not found", modifier_id))?;
                    modifier.applied = true;
                } else {
                    let modifier_id =
                        str_param("modifier_id").with_context(|| "missing modifier_id")?;
                    let before = list.len();
                    list.retain(|item| item.modifier_id != modifier_id);
                    if list.len() == before {
                        bail!("modifier '{}' not found", modifier_id);
                    }
                }
                if list.is_empty() {
                    ctx.modeling.modifiers.remove(&mesh_id);
                }
            } else if ["unwrap_uv", "pack_uv", "generate_lightmap_uv"].contains(&op.as_str()) {
                let mut uv = ctx
                    .modeling
                    .uv
                    .get(&mesh_id)
                    .cloned()
                    .unwrap_or(ModelUvRecord {
                        method: "auto".to_string(),
                        packed: false,
                        lightmap_generated: false,
                        params: json!({}),
                    });
                if op == "unwrap_uv" {
                    uv.method = str_param("method").unwrap_or_else(|| "angle_based".to_string());
                    uv.params = self
                        .params
                        .get("params")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                } else if op == "pack_uv" {
                    uv.packed = true;
                    uv.params = self
                        .params
                        .get("params")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                } else {
                    uv.lightmap_generated = true;
                    uv.params = self
                        .params
                        .get("params")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                }
                ctx.modeling.uv.insert(mesh_id.clone(), uv);
            } else if op == "sculpt_brush" || op == "sculpt_mask" {
                let sculpt_value = if op == "sculpt_brush" {
                    json!({
                        "op": op,
                        "brush_type": str_param("brush_type").unwrap_or_else(|| "grab".to_string()),
                        "params": self.params.get("params").cloned().unwrap_or_else(|| json!({}))
                    })
                } else {
                    json!({
                        "op": op,
                        "params": self.params.get("params").cloned().unwrap_or_else(|| json!({}))
                    })
                };
                ctx.modeling
                    .sculpt_masks
                    .insert(mesh_id.clone(), sculpt_value);
            } else {
                bail!("unsupported model operation '{}'", op);
            }
            ctx.modeling.operation_log.push(ModelOperationRecord {
                tool: format!("model.{}", op),
                mesh_id: mesh_id.clone(),
                payload: self.params.clone(),
            });
            payload["mesh_id"] = Value::String(mesh_id);
        }

        Ok(CommandResult::new(
            format!("model operation '{}' applied", op),
            payload,
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_modeling) = &self.previous_modeling {
            ctx.modeling = previous_modeling.clone();
        }
        if let Some(previous_scene) = &self.previous_scene {
            ctx.scene = previous_scene.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "operation": self.operation,
            "params": self.params
        })
    }
}

#[derive(Debug, Clone)]
pub struct VfxMutationCommand {
    operation: String,
    params: Value,
    previous_vfx: Option<VfxRuntimeState>,
    previous_components: Option<HashMap<String, BTreeMap<String, Value>>>,
    previous_last_message: Option<Option<String>>,
}

impl VfxMutationCommand {
    pub fn new(operation: impl Into<String>, params: Value) -> Self {
        Self {
            operation: operation.into(),
            params,
            previous_vfx: None,
            previous_components: None,
            previous_last_message: None,
        }
    }

    fn op(&self) -> String {
        self.operation.trim().to_ascii_lowercase()
    }
}

impl EngineCommand for VfxMutationCommand {
    fn name(&self) -> &'static str {
        "vfx.mutation"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        let op = self.op();
        let str_param =
            |key: &str| -> Option<&str> { self.params.get(key).and_then(Value::as_str) };
        let non_empty = |key: &str| -> bool {
            str_param(key)
                .map(str::trim)
                .map(|value| !value.is_empty())
                .unwrap_or(false)
        };
        if op == "create_particle_system" || op == "create_graph" {
            if !non_empty("name") {
                return ValidationResult::invalid("name cannot be empty");
            }
            return ValidationResult::ok();
        }

        if op == "add_node" || op == "connect" || op == "compile_graph" {
            if !non_empty("graph_id") {
                return ValidationResult::invalid("graph_id cannot be empty");
            }
            let graph_id = str_param("graph_id").unwrap_or_default().trim();
            let Some(graph) = ctx.vfx.graphs.get(graph_id) else {
                return ValidationResult::invalid(format!(
                    "graph '{}' does not exist; call vfx.create_graph first",
                    graph_id
                ));
            };
            if op == "add_node" {
                if !non_empty("node_type") {
                    return ValidationResult::invalid("node_type cannot be empty");
                }
            } else if op == "connect" {
                if !non_empty("out_node") || !non_empty("in_node") {
                    return ValidationResult::invalid("out_node/in_node cannot be empty");
                }
                let out_node = str_param("out_node").unwrap_or_default().trim();
                let in_node = str_param("in_node").unwrap_or_default().trim();
                let has_out = graph.nodes.iter().any(|node| node.id == out_node);
                let has_in = graph.nodes.iter().any(|node| node.id == in_node);
                if !has_out || !has_in {
                    return ValidationResult::invalid("out_node/in_node must exist in graph");
                }
            }
            return ValidationResult::ok();
        }

        if !non_empty("particle_id") {
            return ValidationResult::invalid("particle_id cannot be empty");
        }
        let particle_id = str_param("particle_id").unwrap_or_default().trim();
        if !ctx.vfx.particle_systems.contains_key(particle_id) {
            return ValidationResult::invalid(format!(
                "particle system '{}' does not exist; call vfx.create_particle_system first",
                particle_id
            ));
        }

        if op == "attach_to_entity" {
            if !non_empty("entity_id") {
                return ValidationResult::invalid("entity_id cannot be empty");
            }
            let entity_id = str_param("entity_id").unwrap_or_default().trim();
            if !ctx.entity_exists(entity_id) {
                return ValidationResult::invalid(format!(
                    "entity '{}' does not exist in open scene",
                    entity_id
                ));
            }
        } else if !["set_emitter", "set_forces", "set_collision", "set_renderer"]
            .contains(&op.as_str())
        {
            return ValidationResult::invalid(format!("unsupported vfx operation '{}'", op));
        }

        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_vfx = Some(ctx.vfx.clone());
        self.previous_components = Some(ctx.components.clone());
        self.previous_last_message = Some(ctx.scene_runtime.last_message.clone());

        let op = self.op();
        let str_param = |key: &str| -> Option<String> {
            self.params
                .get(key)
                .and_then(Value::as_str)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let mut payload = json!({
            "operation": op
        });

        if op == "create_particle_system" {
            let name = str_param("name").with_context(|| "missing name")?;
            let particle_id = str_param("particle_id").unwrap_or_else(|| {
                format!(
                    "ps_{}_{}",
                    normalize_identifier(&name),
                    ctx.vfx.particle_systems.len() + 1
                )
            });
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.vfx.particle_systems.insert(
                particle_id.clone(),
                VfxParticleSystemRecord {
                    particle_id: particle_id.clone(),
                    name: name.clone(),
                    params: params.clone(),
                    emitter: json!({}),
                    forces: json!({}),
                    collision: json!({}),
                    renderer: json!({}),
                    attached_entity: None,
                    socket: None,
                },
            );
            payload["particle_id"] = Value::String(particle_id);
            payload["name"] = Value::String(name);
            payload["params"] = params;
        } else if op == "set_emitter"
            || op == "set_forces"
            || op == "set_collision"
            || op == "set_renderer"
        {
            let particle_id = str_param("particle_id").with_context(|| "missing particle_id")?;
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let record = ctx
                .vfx
                .particle_systems
                .get_mut(&particle_id)
                .with_context(|| format!("particle '{}' not found", particle_id))?;
            if op == "set_emitter" {
                record.emitter = params.clone();
            } else if op == "set_forces" {
                record.forces = params.clone();
            } else if op == "set_collision" {
                record.collision = params.clone();
            } else {
                record.renderer = params.clone();
            }
            payload["particle_id"] = Value::String(particle_id);
            payload["params"] = params;
        } else if op == "attach_to_entity" {
            let particle_id = str_param("particle_id").with_context(|| "missing particle_id")?;
            let entity_id = str_param("entity_id").with_context(|| "missing entity_id")?;
            let socket = str_param("socket");
            let record = ctx
                .vfx
                .particle_systems
                .get_mut(&particle_id)
                .with_context(|| format!("particle '{}' not found", particle_id))?;
            record.attached_entity = Some(entity_id.clone());
            record.socket = socket.clone();

            let bucket = ctx.components.entry(entity_id.clone()).or_default();
            let mut component_value = bucket
                .get("ParticleAttachments")
                .cloned()
                .unwrap_or_else(|| json!([]));
            if let Some(array) = component_value.as_array_mut() {
                array.push(json!({
                    "particle_id": particle_id,
                    "socket": socket
                }));
            }
            bucket.insert("ParticleAttachments".to_string(), component_value);

            payload["particle_id"] = Value::String(particle_id);
            payload["entity_id"] = Value::String(entity_id);
            payload["socket"] = socket.map(Value::String).unwrap_or(Value::Null);
        } else if op == "create_graph" {
            let name = str_param("name").with_context(|| "missing name")?;
            let graph_id = str_param("graph_id").unwrap_or_else(|| {
                format!(
                    "vfx_graph_{}_{}",
                    normalize_identifier(&name),
                    ctx.vfx.graphs.len() + 1
                )
            });
            ctx.vfx.graphs.insert(
                graph_id.clone(),
                VfxGraphRecord {
                    graph_id: graph_id.clone(),
                    name: name.clone(),
                    nodes: Vec::new(),
                    edges: Vec::new(),
                    compiled: false,
                    compile_report: None,
                },
            );
            payload["graph_id"] = Value::String(graph_id);
            payload["name"] = Value::String(name);
        } else if op == "add_node" {
            let graph_id = str_param("graph_id").with_context(|| "missing graph_id")?;
            let node_type = str_param("node_type").with_context(|| "missing node_type")?;
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let graph = ctx
                .vfx
                .graphs
                .get_mut(&graph_id)
                .with_context(|| format!("graph '{}' not found", graph_id))?;
            let node_id =
                str_param("node_id").unwrap_or_else(|| format!("node_{}", graph.nodes.len() + 1));
            graph.nodes.push(VfxGraphNodeRecord {
                id: node_id.clone(),
                node_type: node_type.clone(),
                params: params.clone(),
            });
            graph.compiled = false;
            graph.compile_report = None;
            payload["graph_id"] = Value::String(graph_id);
            payload["node_id"] = Value::String(node_id);
            payload["node_type"] = Value::String(node_type);
            payload["params"] = params;
        } else if op == "connect" {
            let graph_id = str_param("graph_id").with_context(|| "missing graph_id")?;
            let out_node = str_param("out_node").with_context(|| "missing out_node")?;
            let in_node = str_param("in_node").with_context(|| "missing in_node")?;
            let graph = ctx
                .vfx
                .graphs
                .get_mut(&graph_id)
                .with_context(|| format!("graph '{}' not found", graph_id))?;
            graph.edges.push(VfxGraphEdgeRecord {
                out_node: out_node.clone(),
                in_node: in_node.clone(),
            });
            graph.compiled = false;
            graph.compile_report = None;
            payload["graph_id"] = Value::String(graph_id);
            payload["out_node"] = Value::String(out_node);
            payload["in_node"] = Value::String(in_node);
        } else if op == "compile_graph" {
            let graph_id = str_param("graph_id").with_context(|| "missing graph_id")?;
            let graph = ctx
                .vfx
                .graphs
                .get_mut(&graph_id)
                .with_context(|| format!("graph '{}' not found", graph_id))?;
            let valid = !graph.nodes.is_empty();
            graph.compiled = valid;
            graph.compile_report = Some(json!({
                "valid": valid,
                "node_count": graph.nodes.len(),
                "edge_count": graph.edges.len()
            }));
            payload["graph_id"] = Value::String(graph_id);
            payload["compile_report"] = graph.compile_report.clone().unwrap_or(Value::Null);
        } else {
            bail!("unsupported vfx operation '{}'", op);
        }

        ctx.scene_runtime.last_message = Some(format!("vfx operation '{}' applied", op));
        Ok(CommandResult::new(
            format!("vfx operation '{}' applied", op),
            payload,
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_vfx) = &self.previous_vfx {
            ctx.vfx = previous_vfx.clone();
        }
        if let Some(previous_components) = &self.previous_components {
            ctx.components = previous_components.clone();
        }
        if let Some(previous_last_message) = &self.previous_last_message {
            ctx.scene_runtime.last_message = previous_last_message.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "operation": self.operation,
            "params": self.params
        })
    }
}

#[derive(Debug, Clone)]
pub struct WaterMutationCommand {
    operation: String,
    params: Value,
    previous_water: Option<WaterRuntimeState>,
    previous_components: Option<HashMap<String, BTreeMap<String, Value>>>,
    previous_last_message: Option<Option<String>>,
}

impl WaterMutationCommand {
    pub fn new(operation: impl Into<String>, params: Value) -> Self {
        Self {
            operation: operation.into(),
            params,
            previous_water: None,
            previous_components: None,
            previous_last_message: None,
        }
    }

    fn op(&self) -> String {
        self.operation.trim().to_ascii_lowercase()
    }
}

impl EngineCommand for WaterMutationCommand {
    fn name(&self) -> &'static str {
        "water.mutation"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        let op = self.op();
        let str_param =
            |key: &str| -> Option<&str> { self.params.get(key).and_then(Value::as_str) };
        let non_empty = |key: &str| -> bool {
            str_param(key)
                .map(str::trim)
                .map(|value| !value.is_empty())
                .unwrap_or(false)
        };

        if op == "create_ocean" || op == "create_waterfall" {
            return ValidationResult::ok();
        }
        if op == "create_river" {
            let Some(path_value) = self.params.get("path") else {
                return ValidationResult::invalid("path is required");
            };
            let Ok(path) = parse_vec3_array(path_value) else {
                return ValidationResult::invalid("path must be an array of [x,y,z]");
            };
            if path.len() < 2 {
                return ValidationResult::invalid("path must contain at least 2 points");
            }
            return ValidationResult::ok();
        }

        if [
            "set_waves",
            "enable_foam",
            "enable_refraction",
            "enable_caustics",
        ]
        .contains(&op.as_str())
        {
            if !non_empty("ocean_id") {
                return ValidationResult::invalid("ocean_id cannot be empty");
            }
            let ocean_id = str_param("ocean_id").unwrap_or_default().trim();
            if !ctx.water.oceans.contains_key(ocean_id) {
                return ValidationResult::invalid(format!(
                    "ocean '{}' does not exist; call water.create_ocean first",
                    ocean_id
                ));
            }
            return ValidationResult::ok();
        }

        if op == "add_buoyancy" || op == "add_drag" {
            if !non_empty("entity_id") {
                return ValidationResult::invalid("entity_id cannot be empty");
            }
            let entity_id = str_param("entity_id").unwrap_or_default().trim();
            if !ctx.entity_exists(entity_id) {
                return ValidationResult::invalid(format!(
                    "entity '{}' does not exist in open scene",
                    entity_id
                ));
            }
            return ValidationResult::ok();
        }

        ValidationResult::invalid(format!("unsupported water operation '{}'", op))
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_water = Some(ctx.water.clone());
        self.previous_components = Some(ctx.components.clone());
        self.previous_last_message = Some(ctx.scene_runtime.last_message.clone());

        let op = self.op();
        let str_param = |key: &str| -> Option<String> {
            self.params
                .get(key)
                .and_then(Value::as_str)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let mut payload = json!({
            "operation": op
        });

        if op == "create_ocean" {
            let ocean_id = str_param("ocean_id")
                .unwrap_or_else(|| format!("ocean_{}", ctx.water.oceans.len() + 1));
            let size = self
                .params
                .get("size")
                .and_then(Value::as_f64)
                .map(|value| value as f32)
                .unwrap_or(512.0)
                .max(1.0);
            let waves = self
                .params
                .get("waves")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.water.oceans.insert(
                ocean_id.clone(),
                WaterOceanRecord {
                    ocean_id: ocean_id.clone(),
                    size,
                    waves: waves.clone(),
                    foam_enabled: false,
                    foam_params: json!({}),
                    refraction_enabled: false,
                    refraction_params: json!({}),
                    caustics_enabled: false,
                    caustics_params: json!({}),
                    params: params.clone(),
                },
            );
            payload["ocean_id"] = Value::String(ocean_id);
            payload["size"] = json!(size);
        } else if op == "create_river" {
            let river_id = str_param("river_id")
                .unwrap_or_else(|| format!("river_{}", ctx.water.rivers.len() + 1));
            let path = parse_vec3_array(self.params.get("path").with_context(|| "missing path")?)?;
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.water.rivers.insert(
                river_id.clone(),
                WaterRiverRecord {
                    river_id: river_id.clone(),
                    path: path.clone(),
                    params: params.clone(),
                },
            );
            payload["river_id"] = Value::String(river_id);
            payload["point_count"] = json!(path.len());
        } else if op == "create_waterfall" {
            let waterfall_id = str_param("waterfall_id")
                .unwrap_or_else(|| format!("waterfall_{}", ctx.water.waterfalls.len() + 1));
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.water.waterfalls.insert(
                waterfall_id.clone(),
                WaterWaterfallRecord {
                    waterfall_id: waterfall_id.clone(),
                    params: params.clone(),
                },
            );
            payload["waterfall_id"] = Value::String(waterfall_id);
        } else if [
            "set_waves",
            "enable_foam",
            "enable_refraction",
            "enable_caustics",
        ]
        .contains(&op.as_str())
        {
            let ocean_id = str_param("ocean_id").with_context(|| "missing ocean_id")?;
            let ocean = ctx
                .water
                .oceans
                .get_mut(&ocean_id)
                .with_context(|| format!("ocean '{}' not found", ocean_id))?;
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if op == "set_waves" {
                ocean.waves = params.clone();
            } else if op == "enable_foam" {
                ocean.foam_enabled = true;
                ocean.foam_params = params.clone();
            } else if op == "enable_refraction" {
                ocean.refraction_enabled = true;
                ocean.refraction_params = params.clone();
            } else {
                ocean.caustics_enabled = true;
                ocean.caustics_params = params.clone();
            }
            payload["ocean_id"] = Value::String(ocean_id);
            payload["params"] = params;
        } else if op == "add_buoyancy" || op == "add_drag" {
            let entity_id = str_param("entity_id").with_context(|| "missing entity_id")?;
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if op == "add_buoyancy" {
                ctx.water.buoyancy.insert(entity_id.clone(), params.clone());
                let bucket = ctx.components.entry(entity_id.clone()).or_default();
                bucket.insert("Buoyancy".to_string(), params.clone());
            } else {
                ctx.water.drag.insert(entity_id.clone(), params.clone());
                let bucket = ctx.components.entry(entity_id.clone()).or_default();
                bucket.insert("WaterDrag".to_string(), params.clone());
            }
            payload["entity_id"] = Value::String(entity_id);
            payload["params"] = params;
        } else {
            bail!("unsupported water operation '{}'", op);
        }

        ctx.scene_runtime.last_message = Some(format!("water operation '{}' applied", op));
        Ok(CommandResult::new(
            format!("water operation '{}' applied", op),
            payload,
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_water) = &self.previous_water {
            ctx.water = previous_water.clone();
        }
        if let Some(previous_components) = &self.previous_components {
            ctx.components = previous_components.clone();
        }
        if let Some(previous_last_message) = &self.previous_last_message {
            ctx.scene_runtime.last_message = previous_last_message.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "operation": self.operation,
            "params": self.params
        })
    }
}

#[derive(Debug, Clone)]
pub struct MountMutationCommand {
    operation: String,
    params: Value,
    previous_mount: Option<MountRuntimeState>,
    previous_scene: Option<SceneFile>,
    previous_components: Option<HashMap<String, BTreeMap<String, Value>>>,
    previous_last_message: Option<Option<String>>,
}

impl MountMutationCommand {
    pub fn new(operation: impl Into<String>, params: Value) -> Self {
        Self {
            operation: operation.into(),
            params,
            previous_mount: None,
            previous_scene: None,
            previous_components: None,
            previous_last_message: None,
        }
    }

    fn op(&self) -> String {
        self.operation.trim().to_ascii_lowercase()
    }
}

impl EngineCommand for MountMutationCommand {
    fn name(&self) -> &'static str {
        "mount.mutation"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        let op = self.op();
        let sp = |k: &str| self.params.get(k).and_then(Value::as_str).map(str::trim);
        if op == "create_horse_template" {
            return ValidationResult::ok();
        }
        if op == "spawn_horse" {
            let Some(template_id) = sp("template_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("template_id cannot be empty");
            };
            if !ctx.mount.horse_templates.contains_key(template_id) {
                return ValidationResult::invalid(format!("template '{}' not found", template_id));
            }
            return ValidationResult::ok();
        }
        if op == "mount_rider" {
            let Some(horse_id) = sp("horse_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("horse_id cannot be empty");
            };
            let Some(rider_id) = sp("rider_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("rider_id cannot be empty");
            };
            if !ctx.mount.horses.contains_key(horse_id) {
                return ValidationResult::invalid(format!("horse '{}' not found", horse_id));
            }
            if !ctx.entity_exists(rider_id) {
                return ValidationResult::invalid(format!("rider '{}' not found", rider_id));
            }
            return ValidationResult::ok();
        }
        if op == "dismount" {
            let Some(rider_id) = sp("rider_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("rider_id cannot be empty");
            };
            if !ctx.mount.rider_to_horse.contains_key(rider_id) {
                return ValidationResult::invalid(format!("rider '{}' is not mounted", rider_id));
            }
            return ValidationResult::ok();
        }
        if op == "set_gait" || op == "set_path_follow" {
            let Some(horse_id) = sp("horse_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("horse_id cannot be empty");
            };
            if !ctx.mount.horses.contains_key(horse_id) {
                return ValidationResult::invalid(format!("horse '{}' not found", horse_id));
            }
            if op == "set_path_follow" && sp("path_id").unwrap_or_default().is_empty() {
                return ValidationResult::invalid("path_id cannot be empty");
            }
            return ValidationResult::ok();
        }
        ValidationResult::invalid(format!("unsupported mount operation '{}'", op))
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_mount = Some(ctx.mount.clone());
        self.previous_scene = Some(ctx.scene.clone());
        self.previous_components = Some(ctx.components.clone());
        self.previous_last_message = Some(ctx.scene_runtime.last_message.clone());

        let op = self.op();
        let sp = |k: &str| {
            self.params
                .get(k)
                .and_then(Value::as_str)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let mut payload = json!({"operation": op});

        if op == "create_horse_template" {
            let template_id = sp("template_id")
                .unwrap_or_else(|| format!("horse_tpl_{}", ctx.mount.horse_templates.len() + 1));
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.mount.horse_templates.insert(
                template_id.clone(),
                MountHorseTemplateRecord {
                    template_id: template_id.clone(),
                    params: params.clone(),
                },
            );
            payload["template_id"] = Value::String(template_id);
            payload["params"] = params;
        } else if op == "spawn_horse" {
            let template_id = sp("template_id").with_context(|| "missing template_id")?;
            let horse_id =
                sp("horse_id").unwrap_or_else(|| format!("horse_{}", ctx.mount.horses.len() + 1));
            let requested_entity = sp("entity_id")
                .unwrap_or_else(|| format!("Horse_{}", normalize_identifier(&horse_id)));
            let entity_id = unique_entity_name(&ctx.scene, &requested_entity);
            let translation =
                parse_optional_vec3(self.params.get("translation")).unwrap_or([0.0, 0.0, 0.0]);
            let spawn_params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.scene.entities.push(SceneEntity {
                name: entity_id.clone(),
                mesh: "horse".to_string(),
                translation,
            });
            ctx.mount.horses.insert(
                horse_id.clone(),
                MountHorseRecord {
                    horse_id: horse_id.clone(),
                    template_id: template_id.clone(),
                    entity_id: entity_id.clone(),
                    rider_id: None,
                    gait: "walk".to_string(),
                    path_follow: None,
                    params: spawn_params.clone(),
                },
            );
            payload["horse_id"] = Value::String(horse_id);
            payload["template_id"] = Value::String(template_id);
            payload["entity_id"] = Value::String(entity_id);
        } else if op == "mount_rider" {
            let horse_id = sp("horse_id").with_context(|| "missing horse_id")?;
            let rider_id = sp("rider_id").with_context(|| "missing rider_id")?;
            let horse = ctx
                .mount
                .horses
                .get_mut(&horse_id)
                .with_context(|| format!("horse '{}' not found", horse_id))?;
            horse.rider_id = Some(rider_id.clone());
            ctx.mount
                .rider_to_horse
                .insert(rider_id.clone(), horse_id.clone());
            payload["horse_id"] = Value::String(horse_id);
            payload["rider_id"] = Value::String(rider_id);
        } else if op == "dismount" {
            let rider_id = sp("rider_id").with_context(|| "missing rider_id")?;
            let horse_id = ctx
                .mount
                .rider_to_horse
                .remove(&rider_id)
                .with_context(|| format!("rider '{}' is not mounted", rider_id))?;
            if let Some(horse) = ctx.mount.horses.get_mut(&horse_id) {
                horse.rider_id = None;
            }
            payload["rider_id"] = Value::String(rider_id);
            payload["horse_id"] = Value::String(horse_id);
        } else if op == "set_gait" {
            let horse_id = sp("horse_id").with_context(|| "missing horse_id")?;
            let gait = normalize_horse_gait(sp("gait").unwrap_or_else(|| "walk".to_string()));
            if let Some(horse) = ctx.mount.horses.get_mut(&horse_id) {
                horse.gait = gait.clone();
            }
            payload["horse_id"] = Value::String(horse_id);
            payload["gait"] = Value::String(gait);
        } else if op == "set_path_follow" {
            let horse_id = sp("horse_id").with_context(|| "missing horse_id")?;
            let path_id = sp("path_id").with_context(|| "missing path_id")?;
            if let Some(horse) = ctx.mount.horses.get_mut(&horse_id) {
                horse.path_follow = Some(path_id.clone());
            }
            payload["horse_id"] = Value::String(horse_id);
            payload["path_id"] = Value::String(path_id);
        } else {
            bail!("unsupported mount operation '{}'", op);
        }

        ctx.scene_runtime.last_message = Some(format!("mount operation '{}' applied", op));
        Ok(CommandResult::new(
            format!("mount operation '{}' applied", op),
            payload,
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_mount) = &self.previous_mount {
            ctx.mount = previous_mount.clone();
        }
        if let Some(previous_scene) = &self.previous_scene {
            ctx.scene = previous_scene.clone();
        }
        if let Some(previous_components) = &self.previous_components {
            ctx.components = previous_components.clone();
        }
        if let Some(previous_last_message) = &self.previous_last_message {
            ctx.scene_runtime.last_message = previous_last_message.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "operation": self.operation,
            "params": self.params
        })
    }
}

#[derive(Debug, Clone)]
pub struct NpcAiMutationCommand {
    operation: String,
    params: Value,
    previous_npc_ai: Option<NpcAiRuntimeState>,
    previous_components: Option<HashMap<String, BTreeMap<String, Value>>>,
    previous_last_message: Option<Option<String>>,
}

impl NpcAiMutationCommand {
    pub fn new(operation: impl Into<String>, params: Value) -> Self {
        Self {
            operation: operation.into(),
            params,
            previous_npc_ai: None,
            previous_components: None,
            previous_last_message: None,
        }
    }

    fn op(&self) -> String {
        self.operation.trim().to_ascii_lowercase()
    }
}

impl EngineCommand for NpcAiMutationCommand {
    fn name(&self) -> &'static str {
        "ai.mutation"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        let op = self.op();
        let sp = |k: &str| self.params.get(k).and_then(Value::as_str).map(str::trim);
        if op == "create_navmesh" {
            return ValidationResult::ok();
        }
        if op == "bake_navmesh" {
            if let Some(navmesh_id) = sp("navmesh_id").filter(|v| !v.is_empty())
                && !ctx.npc_ai.navmeshes.contains_key(navmesh_id)
            {
                return ValidationResult::invalid(format!("navmesh '{}' not found", navmesh_id));
            }
            if ctx.npc_ai.navmeshes.is_empty() {
                return ValidationResult::invalid("no navmesh exists");
            }
            return ValidationResult::ok();
        }
        if op == "add_agent" {
            let Some(entity_id) = sp("entity_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("entity_id cannot be empty");
            };
            if !ctx.entity_exists(entity_id) {
                return ValidationResult::invalid(format!("entity '{}' not found", entity_id));
            }
            if ctx.npc_ai.entity_agents.contains_key(entity_id) {
                return ValidationResult::invalid(format!(
                    "entity '{}' already has agent",
                    entity_id
                ));
            }
            return ValidationResult::ok();
        }
        if op == "set_destination" {
            let Some(agent_id) = sp("agent_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("agent_id cannot be empty");
            };
            if !ctx.npc_ai.agents.contains_key(agent_id) {
                return ValidationResult::invalid(format!("agent '{}' not found", agent_id));
            }
            if parse_optional_vec3(self.params.get("position")).is_none() {
                return ValidationResult::invalid("position must be [x,y,z]");
            }
            return ValidationResult::ok();
        }
        if op == "create_behavior_tree" {
            if sp("name").unwrap_or_default().is_empty() {
                return ValidationResult::invalid("name cannot be empty");
            }
            return ValidationResult::ok();
        }
        if op == "bt_add_node" || op == "bt_connect" {
            let Some(tree_id) = sp("tree_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("tree_id cannot be empty");
            };
            let Some(tree) = ctx.npc_ai.behavior_trees.get(tree_id) else {
                return ValidationResult::invalid(format!("tree '{}' not found", tree_id));
            };
            if op == "bt_add_node" {
                if sp("node_type").unwrap_or_default().is_empty() {
                    return ValidationResult::invalid("node_type cannot be empty");
                }
            } else {
                let parent = sp("parent").unwrap_or_default();
                let child = sp("child").unwrap_or_default();
                if parent.is_empty() || child.is_empty() {
                    return ValidationResult::invalid("parent/child cannot be empty");
                }
                if !tree.nodes.iter().any(|n| n.node_id == parent)
                    || !tree.nodes.iter().any(|n| n.node_id == child)
                {
                    return ValidationResult::invalid("parent/child nodes not found");
                }
            }
            return ValidationResult::ok();
        }
        if op == "assign_behavior" {
            let Some(entity_id) = sp("entity_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("entity_id cannot be empty");
            };
            let Some(tree_id) = sp("tree_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("tree_id cannot be empty");
            };
            if !ctx.npc_ai.behavior_trees.contains_key(tree_id) {
                return ValidationResult::invalid(format!("tree '{}' not found", tree_id));
            }
            if !ctx.npc_ai.entity_agents.contains_key(entity_id) {
                return ValidationResult::invalid(format!(
                    "entity '{}' has no AI agent",
                    entity_id
                ));
            }
            return ValidationResult::ok();
        }
        if op == "set_blackboard" {
            let Some(entity_id) = sp("entity_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("entity_id cannot be empty");
            };
            let Some(key) = sp("key").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("key cannot be empty");
            };
            if !ctx.entity_exists(entity_id) {
                return ValidationResult::invalid(format!("entity '{}' not found", entity_id));
            }
            if key.is_empty() {
                return ValidationResult::invalid("key cannot be empty");
            }
            return ValidationResult::ok();
        }
        ValidationResult::invalid(format!("unsupported AI operation '{}'", op))
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_npc_ai = Some(ctx.npc_ai.clone());
        self.previous_components = Some(ctx.components.clone());
        self.previous_last_message = Some(ctx.scene_runtime.last_message.clone());

        let op = self.op();
        let sp = |k: &str| {
            self.params
                .get(k)
                .and_then(Value::as_str)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let mut payload = json!({"operation": op});

        if op == "create_navmesh" {
            let navmesh_id = sp("navmesh_id")
                .unwrap_or_else(|| format!("navmesh_{}", ctx.npc_ai.navmeshes.len() + 1));
            let navmesh_params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.npc_ai.navmeshes.insert(
                navmesh_id.clone(),
                NpcAiNavmeshRecord {
                    navmesh_id: navmesh_id.clone(),
                    params: navmesh_params.clone(),
                    baked: false,
                },
            );
            ctx.npc_ai.active_navmesh_id = Some(navmesh_id.clone());
            payload["navmesh_id"] = Value::String(navmesh_id);
        } else if op == "bake_navmesh" {
            let navmesh_id = sp("navmesh_id")
                .or_else(|| ctx.npc_ai.active_navmesh_id.clone())
                .or_else(|| ctx.npc_ai.navmeshes.keys().next().cloned())
                .with_context(|| "no navmesh available")?;
            if let Some(navmesh) = ctx.npc_ai.navmeshes.get_mut(&navmesh_id) {
                navmesh.baked = true;
            }
            ctx.npc_ai.active_navmesh_id = Some(navmesh_id.clone());
            payload["navmesh_id"] = Value::String(navmesh_id);
            payload["baked"] = Value::Bool(true);
        } else if op == "add_agent" {
            let entity_id = sp("entity_id").with_context(|| "missing entity_id")?;
            let agent_id = sp("agent_id")
                .unwrap_or_else(|| format!("agent_{}", normalize_identifier(&entity_id)));
            let agent_params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.npc_ai.agents.insert(
                agent_id.clone(),
                NpcAiAgentRecord {
                    agent_id: agent_id.clone(),
                    entity_id: entity_id.clone(),
                    params: agent_params.clone(),
                    destination: None,
                    behavior_tree_id: None,
                },
            );
            ctx.npc_ai
                .entity_agents
                .insert(entity_id.clone(), agent_id.clone());
            payload["entity_id"] = Value::String(entity_id);
            payload["agent_id"] = Value::String(agent_id);
        } else if op == "set_destination" {
            let agent_id = sp("agent_id").with_context(|| "missing agent_id")?;
            let position = parse_optional_vec3(self.params.get("position"))
                .with_context(|| "missing position")?;
            let agent = ctx
                .npc_ai
                .agents
                .get_mut(&agent_id)
                .with_context(|| format!("agent '{}' not found", agent_id))?;
            agent.destination = Some(position);
            payload["agent_id"] = Value::String(agent_id);
            payload["position"] = json!(position);
        } else if op == "create_behavior_tree" {
            let name = sp("name").with_context(|| "missing name")?;
            let tree_id =
                sp("tree_id").unwrap_or_else(|| format!("bt_{}", normalize_identifier(&name)));
            ctx.npc_ai.behavior_trees.insert(
                tree_id.clone(),
                NpcAiBehaviorTreeRecord {
                    tree_id: tree_id.clone(),
                    name: name.clone(),
                    nodes: Vec::new(),
                    edges: Vec::new(),
                },
            );
            payload["tree_id"] = Value::String(tree_id);
            payload["name"] = Value::String(name);
        } else if op == "bt_add_node" {
            let tree_id = sp("tree_id").with_context(|| "missing tree_id")?;
            let node_type = sp("node_type").with_context(|| "missing node_type")?;
            let node_params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let tree = ctx
                .npc_ai
                .behavior_trees
                .get_mut(&tree_id)
                .with_context(|| format!("tree '{}' not found", tree_id))?;
            let node_id = sp("node_id").unwrap_or_else(|| format!("node_{}", tree.nodes.len() + 1));
            tree.nodes.push(NpcAiBehaviorNodeRecord {
                node_id: node_id.clone(),
                node_type: node_type.clone(),
                params: node_params,
            });
            payload["tree_id"] = Value::String(tree_id);
            payload["node_id"] = Value::String(node_id);
            payload["node_type"] = Value::String(node_type);
        } else if op == "bt_connect" {
            let tree_id = sp("tree_id").with_context(|| "missing tree_id")?;
            let parent = sp("parent").with_context(|| "missing parent")?;
            let child = sp("child").with_context(|| "missing child")?;
            let tree = ctx
                .npc_ai
                .behavior_trees
                .get_mut(&tree_id)
                .with_context(|| format!("tree '{}' not found", tree_id))?;
            tree.edges.push(NpcAiBehaviorEdgeRecord {
                parent: parent.clone(),
                child: child.clone(),
            });
            payload["tree_id"] = Value::String(tree_id);
            payload["parent"] = Value::String(parent);
            payload["child"] = Value::String(child);
        } else if op == "assign_behavior" {
            let entity_id = sp("entity_id").with_context(|| "missing entity_id")?;
            let tree_id = sp("tree_id").with_context(|| "missing tree_id")?;
            let agent_id = ctx
                .npc_ai
                .entity_agents
                .get(&entity_id)
                .cloned()
                .with_context(|| format!("entity '{}' has no AI agent", entity_id))?;
            if let Some(agent) = ctx.npc_ai.agents.get_mut(&agent_id) {
                agent.behavior_tree_id = Some(tree_id.clone());
            }
            payload["entity_id"] = Value::String(entity_id);
            payload["agent_id"] = Value::String(agent_id);
            payload["tree_id"] = Value::String(tree_id);
        } else if op == "set_blackboard" {
            let entity_id = sp("entity_id").with_context(|| "missing entity_id")?;
            let key = sp("key").with_context(|| "missing key")?;
            let value = self.params.get("value").cloned().unwrap_or(Value::Null);
            let board = ctx.npc_ai.blackboard.entry(entity_id.clone()).or_default();
            board.insert(key.clone(), value.clone());
            payload["entity_id"] = Value::String(entity_id);
            payload["key"] = Value::String(key);
            payload["value"] = value;
        } else {
            bail!("unsupported AI operation '{}'", op);
        }

        ctx.scene_runtime.last_message = Some(format!("ai operation '{}' applied", op));
        Ok(CommandResult::new(
            format!("ai operation '{}' applied", op),
            payload,
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_npc_ai) = &self.previous_npc_ai {
            ctx.npc_ai = previous_npc_ai.clone();
        }
        if let Some(previous_components) = &self.previous_components {
            ctx.components = previous_components.clone();
        }
        if let Some(previous_last_message) = &self.previous_last_message {
            ctx.scene_runtime.last_message = previous_last_message.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "operation": self.operation,
            "params": self.params
        })
    }
}

#[derive(Debug, Clone)]
pub struct UiMutationCommand {
    operation: String,
    params: Value,
    previous_ui: Option<UiRuntimeState>,
    previous_last_message: Option<Option<String>>,
}

impl UiMutationCommand {
    pub fn new(operation: impl Into<String>, params: Value) -> Self {
        Self {
            operation: operation.into(),
            params,
            previous_ui: None,
            previous_last_message: None,
        }
    }

    fn op(&self) -> String {
        self.operation.trim().to_ascii_lowercase()
    }
}

impl EngineCommand for UiMutationCommand {
    fn name(&self) -> &'static str {
        "ui.mutation"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        let op = self.op();
        let sp = |k: &str| self.params.get(k).and_then(Value::as_str).map(str::trim);
        if op == "create_canvas" {
            if sp("name").unwrap_or_default().is_empty() {
                return ValidationResult::invalid("name cannot be empty");
            }
            return ValidationResult::ok();
        }
        if op == "add_panel" || op == "add_text" || op == "add_button" {
            let Some(canvas_id) = sp("canvas_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("canvas_id cannot be empty");
            };
            if !ctx.ui.canvases.contains_key(canvas_id) {
                return ValidationResult::invalid(format!(
                    "canvas '{}' does not exist; call ui.create_canvas first",
                    canvas_id
                ));
            }
            return ValidationResult::ok();
        }
        if op == "bind_to_data" {
            let Some(ui_id) = sp("ui_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("ui_id cannot be empty");
            };
            let Some(entity_id) = sp("entity_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("entity_id cannot be empty");
            };
            let Some(component_field) = sp("component_field").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("component_field cannot be empty");
            };
            if !ctx.ui.elements.contains_key(ui_id) {
                return ValidationResult::invalid(format!("ui element '{}' does not exist", ui_id));
            }
            if !ctx.entity_exists(entity_id) {
                return ValidationResult::invalid(format!(
                    "entity '{}' does not exist in open scene",
                    entity_id
                ));
            }
            if component_field.is_empty() {
                return ValidationResult::invalid("component_field cannot be empty");
            }
            return ValidationResult::ok();
        }
        if op == "create_hud_template" {
            return ValidationResult::ok();
        }
        ValidationResult::invalid(format!("unsupported UI operation '{}'", op))
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_ui = Some(ctx.ui.clone());
        self.previous_last_message = Some(ctx.scene_runtime.last_message.clone());

        let op = self.op();
        let sp = |k: &str| {
            self.params
                .get(k)
                .and_then(Value::as_str)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let mut payload = json!({"operation": op});

        if op == "create_canvas" {
            let name = sp("name").with_context(|| "missing name")?;
            let canvas_id =
                sp("canvas_id").unwrap_or_else(|| format!("canvas_{}", ctx.ui.canvases.len() + 1));
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.ui.canvases.insert(
                canvas_id.clone(),
                UiCanvasRecord {
                    canvas_id: canvas_id.clone(),
                    name: name.clone(),
                    params: params.clone(),
                },
            );
            payload["canvas_id"] = Value::String(canvas_id);
            payload["name"] = Value::String(name);
            payload["params"] = params;
        } else if op == "add_panel" || op == "add_text" || op == "add_button" {
            let canvas_id = sp("canvas_id").with_context(|| "missing canvas_id")?;
            let element_type = op.trim_start_matches("add_").to_string();
            let ui_id = sp("ui_id")
                .unwrap_or_else(|| format!("{}_{}", element_type, ctx.ui.elements.len() + 1));
            let mut params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if let Some(text) = sp("text")
                && let Some(obj) = params.as_object_mut()
            {
                obj.insert("text".to_string(), Value::String(text));
            }
            if let Some(label) = sp("label")
                && let Some(obj) = params.as_object_mut()
            {
                obj.insert("label".to_string(), Value::String(label));
            }
            ctx.ui.elements.insert(
                ui_id.clone(),
                UiElementRecord {
                    ui_id: ui_id.clone(),
                    canvas_id: canvas_id.clone(),
                    element_type: element_type.clone(),
                    params: params.clone(),
                },
            );
            payload["canvas_id"] = Value::String(canvas_id);
            payload["ui_id"] = Value::String(ui_id);
            payload["element_type"] = Value::String(element_type);
            payload["params"] = params;
        } else if op == "bind_to_data" {
            let ui_id = sp("ui_id").with_context(|| "missing ui_id")?;
            let entity_id = sp("entity_id").with_context(|| "missing entity_id")?;
            let component_field =
                sp("component_field").with_context(|| "missing component_field")?;
            ctx.ui.bindings.insert(
                ui_id.clone(),
                UiBindingRecord {
                    ui_id: ui_id.clone(),
                    entity_id: entity_id.clone(),
                    component_field: component_field.clone(),
                },
            );
            payload["ui_id"] = Value::String(ui_id);
            payload["entity_id"] = Value::String(entity_id);
            payload["component_field"] = Value::String(component_field);
        } else if op == "create_hud_template" {
            let template_type = normalize_ui_template(
                sp("type")
                    .or_else(|| sp("template_type"))
                    .unwrap_or_else(|| "shooter".to_string()),
            );
            let canvas_id = format!("hud_{}_{}", template_type, ctx.ui.canvases.len() + 1);
            ctx.ui.canvases.insert(
                canvas_id.clone(),
                UiCanvasRecord {
                    canvas_id: canvas_id.clone(),
                    name: format!("HUD {}", template_type),
                    params: json!({}),
                },
            );
            if template_type == "platformer" {
                ctx.ui.elements.insert(
                    format!("{}_lives", canvas_id),
                    UiElementRecord {
                        ui_id: format!("{}_lives", canvas_id),
                        canvas_id: canvas_id.clone(),
                        element_type: "text".to_string(),
                        params: json!({"text":"Lives: 3","anchor":"top_left"}),
                    },
                );
                ctx.ui.elements.insert(
                    format!("{}_coins", canvas_id),
                    UiElementRecord {
                        ui_id: format!("{}_coins", canvas_id),
                        canvas_id: canvas_id.clone(),
                        element_type: "text".to_string(),
                        params: json!({"text":"Coins: 0","anchor":"top_left"}),
                    },
                );
            } else {
                ctx.ui.elements.insert(
                    format!("{}_health", canvas_id),
                    UiElementRecord {
                        ui_id: format!("{}_health", canvas_id),
                        canvas_id: canvas_id.clone(),
                        element_type: "text".to_string(),
                        params: json!({"text":"HP: 100","anchor":"top_left"}),
                    },
                );
                ctx.ui.elements.insert(
                    format!("{}_ammo", canvas_id),
                    UiElementRecord {
                        ui_id: format!("{}_ammo", canvas_id),
                        canvas_id: canvas_id.clone(),
                        element_type: "text".to_string(),
                        params: json!({"text":"Ammo: 30","anchor":"bottom_right"}),
                    },
                );
                ctx.ui.elements.insert(
                    format!("{}_crosshair", canvas_id),
                    UiElementRecord {
                        ui_id: format!("{}_crosshair", canvas_id),
                        canvas_id: canvas_id.clone(),
                        element_type: "panel".to_string(),
                        params: json!({"style":"crosshair","anchor":"center"}),
                    },
                );
            }
            ctx.ui.active_hud_template = Some(template_type.clone());
            payload["canvas_id"] = Value::String(canvas_id);
            payload["type"] = Value::String(template_type);
        } else {
            bail!("unsupported UI operation '{}'", op);
        }

        ctx.scene_runtime.last_message = Some(format!("ui operation '{}' applied", op));
        Ok(CommandResult::new(
            format!("ui operation '{}' applied", op),
            payload,
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_ui) = &self.previous_ui {
            ctx.ui = previous_ui.clone();
        }
        if let Some(previous_last_message) = &self.previous_last_message {
            ctx.scene_runtime.last_message = previous_last_message.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "operation": self.operation,
            "params": self.params
        })
    }
}

#[derive(Debug, Clone)]
pub struct AudioMutationCommand {
    operation: String,
    params: Value,
    previous_audio: Option<AudioRuntimeState>,
    previous_last_message: Option<Option<String>>,
}

impl AudioMutationCommand {
    pub fn new(operation: impl Into<String>, params: Value) -> Self {
        Self {
            operation: operation.into(),
            params,
            previous_audio: None,
            previous_last_message: None,
        }
    }

    fn op(&self) -> String {
        self.operation.trim().to_ascii_lowercase()
    }
}

impl EngineCommand for AudioMutationCommand {
    fn name(&self) -> &'static str {
        "audio.mutation"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        let op = self.op();
        let sp = |k: &str| self.params.get(k).and_then(Value::as_str).map(str::trim);
        if op == "import_clip" {
            let Some(path) = sp("path").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("path cannot be empty");
            };
            let resolved = resolve_project_path(&ctx.project_root, Path::new(path));
            if !resolved.exists() || !resolved.is_file() {
                return ValidationResult::invalid(format!(
                    "audio clip '{}' does not exist",
                    resolved.display()
                ));
            }
            return ValidationResult::ok();
        }
        if op == "create_source" {
            if let Some(entity_id) = sp("entity_id").filter(|v| !v.is_empty())
                && !ctx.entity_exists(entity_id)
            {
                return ValidationResult::invalid(format!(
                    "entity '{}' does not exist in open scene",
                    entity_id
                ));
            }
            return ValidationResult::ok();
        }
        if op == "create_mixer" {
            return ValidationResult::ok();
        }
        if op == "play" {
            let Some(source_id) = sp("source_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("source_id cannot be empty");
            };
            let Some(clip_id) = sp("clip_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("clip_id cannot be empty");
            };
            if !ctx.audio.sources.contains_key(source_id) {
                return ValidationResult::invalid(format!("source '{}' does not exist", source_id));
            }
            if !ctx.audio.clips.contains_key(clip_id) {
                return ValidationResult::invalid(format!("clip '{}' does not exist", clip_id));
            }
            return ValidationResult::ok();
        }
        if op == "set_spatial" {
            let Some(source_id) = sp("source_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("source_id cannot be empty");
            };
            if !ctx.audio.sources.contains_key(source_id) {
                return ValidationResult::invalid(format!("source '{}' does not exist", source_id));
            }
            return ValidationResult::ok();
        }
        if op == "route" {
            let Some(source_id) = sp("source_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("source_id cannot be empty");
            };
            let Some(mixer_bus) = sp("mixer_bus").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("mixer_bus cannot be empty");
            };
            if !ctx.audio.sources.contains_key(source_id) {
                return ValidationResult::invalid(format!("source '{}' does not exist", source_id));
            }
            if !ctx.audio.mixers.contains_key(mixer_bus) {
                return ValidationResult::invalid(format!("mixer '{}' does not exist", mixer_bus));
            }
            return ValidationResult::ok();
        }
        ValidationResult::invalid(format!("unsupported audio operation '{}'", op))
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_audio = Some(ctx.audio.clone());
        self.previous_last_message = Some(ctx.scene_runtime.last_message.clone());

        let op = self.op();
        let sp = |k: &str| {
            self.params
                .get(k)
                .and_then(Value::as_str)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let mut payload = json!({"operation": op});

        if op == "import_clip" {
            let path = sp("path").with_context(|| "missing path")?;
            let resolved = resolve_project_path(&ctx.project_root, Path::new(&path));
            let stem = resolved
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("clip");
            let mut clip_id = sp("clip_id").unwrap_or_else(|| normalize_identifier(stem));
            if ctx.audio.clips.contains_key(&clip_id) {
                clip_id = format!("{}_{}", clip_id, ctx.audio.clips.len() + 1);
            }
            ctx.audio.clips.insert(
                clip_id.clone(),
                AudioClipRecord {
                    clip_id: clip_id.clone(),
                    path: resolved.display().to_string(),
                },
            );
            payload["clip_id"] = Value::String(clip_id);
            payload["path"] = Value::String(resolved.display().to_string());
        } else if op == "create_source" {
            let mut source_id = sp("source_id")
                .unwrap_or_else(|| format!("source_{}", ctx.audio.sources.len() + 1));
            if ctx.audio.sources.contains_key(&source_id) {
                source_id = format!("{}_{}", source_id, ctx.audio.sources.len() + 1);
            }
            let entity_id = sp("entity_id");
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let spatial = self
                .params
                .get("spatial")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.audio.sources.insert(
                source_id.clone(),
                AudioSourceRecord {
                    source_id: source_id.clone(),
                    entity_id: entity_id.clone(),
                    params: params.clone(),
                    spatial: spatial.clone(),
                    mixer_bus: None,
                    playing_clip: None,
                },
            );
            payload["source_id"] = Value::String(source_id);
            payload["entity_id"] = entity_id.map(Value::String).unwrap_or(Value::Null);
        } else if op == "play" {
            let source_id = sp("source_id").with_context(|| "missing source_id")?;
            let clip_id = sp("clip_id").with_context(|| "missing clip_id")?;
            let source = ctx
                .audio
                .sources
                .get_mut(&source_id)
                .with_context(|| format!("source '{}' not found", source_id))?;
            source.playing_clip = Some(clip_id.clone());
            ctx.audio.play_events = ctx.audio.play_events.saturating_add(1);
            payload["source_id"] = Value::String(source_id);
            payload["clip_id"] = Value::String(clip_id);
        } else if op == "set_spatial" {
            let source_id = sp("source_id").with_context(|| "missing source_id")?;
            let spatial = self
                .params
                .get("params")
                .cloned()
                .or_else(|| self.params.get("spatial").cloned())
                .unwrap_or_else(|| json!({}));
            let source = ctx
                .audio
                .sources
                .get_mut(&source_id)
                .with_context(|| format!("source '{}' not found", source_id))?;
            source.spatial = spatial.clone();
            payload["source_id"] = Value::String(source_id);
            payload["params"] = spatial;
        } else if op == "create_mixer" {
            let mut bus_id =
                sp("bus_id").unwrap_or_else(|| format!("bus_{}", ctx.audio.mixers.len() + 1));
            if ctx.audio.mixers.contains_key(&bus_id) {
                bus_id = format!("{}_{}", bus_id, ctx.audio.mixers.len() + 1);
            }
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.audio.mixers.insert(
                bus_id.clone(),
                AudioMixerRecord {
                    bus_id: bus_id.clone(),
                    params: params.clone(),
                },
            );
            payload["bus_id"] = Value::String(bus_id);
            payload["params"] = params;
        } else if op == "route" {
            let source_id = sp("source_id").with_context(|| "missing source_id")?;
            let mixer_bus = sp("mixer_bus").with_context(|| "missing mixer_bus")?;
            let source = ctx
                .audio
                .sources
                .get_mut(&source_id)
                .with_context(|| format!("source '{}' not found", source_id))?;
            source.mixer_bus = Some(mixer_bus.clone());
            payload["source_id"] = Value::String(source_id);
            payload["mixer_bus"] = Value::String(mixer_bus);
        } else {
            bail!("unsupported audio operation '{}'", op);
        }

        ctx.scene_runtime.last_message = Some(format!("audio operation '{}' applied", op));
        Ok(CommandResult::new(
            format!("audio operation '{}' applied", op),
            payload,
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_audio) = &self.previous_audio {
            ctx.audio = previous_audio.clone();
        }
        if let Some(previous_last_message) = &self.previous_last_message {
            ctx.scene_runtime.last_message = previous_last_message.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "operation": self.operation,
            "params": self.params
        })
    }
}

#[derive(Debug, Clone)]
pub struct NetMutationCommand {
    operation: String,
    params: Value,
    previous_networking: Option<NetworkingRuntimeState>,
    previous_last_message: Option<Option<String>>,
}

impl NetMutationCommand {
    pub fn new(operation: impl Into<String>, params: Value) -> Self {
        Self {
            operation: operation.into(),
            params,
            previous_networking: None,
            previous_last_message: None,
        }
    }

    fn op(&self) -> String {
        self.operation.trim().to_ascii_lowercase()
    }
}

impl EngineCommand for NetMutationCommand {
    fn name(&self) -> &'static str {
        "net.mutation"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        let op = self.op();
        let sp = |k: &str| self.params.get(k).and_then(Value::as_str).map(str::trim);
        if op == "create_server" {
            return ValidationResult::ok();
        }
        if op == "connect_client" {
            if ctx.networking.server.is_none() {
                return ValidationResult::invalid(
                    "no server configured; call net.create_server first",
                );
            }
            return ValidationResult::ok();
        }
        if op == "enable_replication" {
            let Some(entity_id) = sp("entity_id").filter(|v| !v.is_empty()) else {
                return ValidationResult::invalid("entity_id cannot be empty");
            };
            if !ctx.entity_exists(entity_id) {
                return ValidationResult::invalid(format!(
                    "entity '{}' does not exist in open scene",
                    entity_id
                ));
            }
            let has_components = self
                .params
                .get("components")
                .and_then(Value::as_array)
                .map(|items| !items.is_empty())
                .unwrap_or(false);
            if !has_components {
                return ValidationResult::invalid("components must be a non-empty array");
            }
            return ValidationResult::ok();
        }
        if op == "set_prediction" || op == "set_rollback" {
            return ValidationResult::ok();
        }
        ValidationResult::invalid(format!("unsupported networking operation '{}'", op))
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_networking = Some(ctx.networking.clone());
        self.previous_last_message = Some(ctx.scene_runtime.last_message.clone());

        let op = self.op();
        let sp = |k: &str| {
            self.params
                .get(k)
                .and_then(Value::as_str)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let mut payload = json!({"operation": op});

        if op == "create_server" {
            let server_id = sp("server_id").unwrap_or_else(|| "server_main".to_string());
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.networking.server = Some(NetworkServerRecord {
                server_id: server_id.clone(),
                params: params.clone(),
            });
            payload["server_id"] = Value::String(server_id);
            payload["params"] = params;
        } else if op == "connect_client" {
            let mut client_id = sp("client_id")
                .unwrap_or_else(|| format!("client_{}", ctx.networking.clients.len() + 1));
            if ctx.networking.clients.contains_key(&client_id) {
                client_id = format!("{}_{}", client_id, ctx.networking.clients.len() + 1);
            }
            let endpoint = sp("endpoint").unwrap_or_else(|| "127.0.0.1:7777".to_string());
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.networking.clients.insert(
                client_id.clone(),
                NetworkClientRecord {
                    client_id: client_id.clone(),
                    endpoint: endpoint.clone(),
                    params: params.clone(),
                },
            );
            payload["client_id"] = Value::String(client_id);
            payload["endpoint"] = Value::String(endpoint);
        } else if op == "enable_replication" {
            let entity_id = sp("entity_id").with_context(|| "missing entity_id")?;
            let components = self
                .params
                .get("components")
                .and_then(Value::as_array)
                .with_context(|| "components must be an array")?
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|component| !component.is_empty())
                .map(str::to_string)
                .collect::<Vec<String>>();
            if components.is_empty() {
                bail!("components must contain at least one entry");
            }
            ctx.networking
                .replication
                .insert(entity_id.clone(), components.clone());
            payload["entity_id"] = Value::String(entity_id);
            payload["components"] = json!(components);
        } else if op == "set_prediction" {
            let mode =
                normalize_prediction_mode(sp("mode").unwrap_or_else(|| "server".to_string()));
            ctx.networking.prediction_mode = mode.clone();
            payload["mode"] = Value::String(mode);
        } else if op == "set_rollback" {
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            ctx.networking.rollback = params.clone();
            payload["params"] = params;
        } else {
            bail!("unsupported networking operation '{}'", op);
        }

        ctx.scene_runtime.last_message = Some(format!("networking operation '{}' applied", op));
        Ok(CommandResult::new(
            format!("networking operation '{}' applied", op),
            payload,
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_networking) = &self.previous_networking {
            ctx.networking = previous_networking.clone();
        }
        if let Some(previous_last_message) = &self.previous_last_message {
            ctx.scene_runtime.last_message = previous_last_message.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "operation": self.operation,
            "params": self.params
        })
    }
}

#[derive(Debug, Clone)]
pub struct BuildMutationCommand {
    operation: String,
    params: Value,
    previous_build: Option<BuildRuntimeState>,
    previous_last_message: Option<Option<String>>,
    file_backups: Vec<(PathBuf, Option<Vec<u8>>)>,
}

impl BuildMutationCommand {
    pub fn new(operation: impl Into<String>, params: Value) -> Self {
        Self {
            operation: operation.into(),
            params,
            previous_build: None,
            previous_last_message: None,
            file_backups: Vec::new(),
        }
    }

    fn op(&self) -> String {
        self.operation.trim().to_ascii_lowercase()
    }
}

impl EngineCommand for BuildMutationCommand {
    fn name(&self) -> &'static str {
        "build.mutation"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        let op = self.op();
        let sp = |k: &str| self.params.get(k).and_then(Value::as_str).map(str::trim);
        if op == "set_target" {
            if sp("platform").unwrap_or_default().is_empty()
                && sp("target").unwrap_or_default().is_empty()
            {
                return ValidationResult::invalid("platform/target cannot be empty");
            }
            return ValidationResult::ok();
        }
        if op == "set_bundle_id" {
            if sp("id").unwrap_or_default().is_empty() {
                return ValidationResult::invalid("id cannot be empty");
            }
            return ValidationResult::ok();
        }
        if op == "set_version" {
            if sp("version").unwrap_or_default().is_empty() {
                return ValidationResult::invalid("version cannot be empty");
            }
            return ValidationResult::ok();
        }
        if op == "enable_feature" {
            if sp("flag").unwrap_or_default().is_empty() {
                return ValidationResult::invalid("flag cannot be empty");
            }
            return ValidationResult::ok();
        }
        if op == "export_project" || op == "generate_installer" {
            if sp("path").unwrap_or_default().is_empty() {
                return ValidationResult::invalid("path cannot be empty");
            }
            return ValidationResult::ok();
        }
        ValidationResult::invalid(format!("unsupported build operation '{}'", op))
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_build = Some(ctx.build.clone());
        self.previous_last_message = Some(ctx.scene_runtime.last_message.clone());
        self.file_backups.clear();

        let op = self.op();
        let sp = |k: &str| {
            self.params
                .get(k)
                .and_then(Value::as_str)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let mut payload = json!({"operation": op});

        if op == "set_target" {
            let target = normalize_build_target(
                sp("platform")
                    .or_else(|| sp("target"))
                    .unwrap_or_else(|| "windows".to_string()),
            );
            ctx.build.target = target.clone();
            payload["target"] = Value::String(target);
        } else if op == "set_bundle_id" {
            let bundle_id = sp("id").with_context(|| "missing id")?;
            ctx.build.bundle_id = Some(bundle_id.clone());
            payload["id"] = Value::String(bundle_id);
        } else if op == "set_version" {
            let version = sp("version").with_context(|| "missing version")?;
            ctx.build.version = version.clone();
            payload["version"] = Value::String(version);
        } else if op == "enable_feature" {
            let flag = normalize_feature_flag(sp("flag").with_context(|| "missing flag")?);
            ctx.build.enabled_features.insert(flag.clone());
            payload["flag"] = Value::String(flag);
        } else if op == "export_project" {
            let export_path = sp("path").with_context(|| "missing path")?;
            let export_dir = resolve_project_path(&ctx.project_root, Path::new(&export_path));
            fs::create_dir_all(&export_dir)
                .with_context(|| format!("failed to create '{}'", export_dir.display()))?;
            let manifest_path = export_dir.join("export_manifest.json");
            self.file_backups
                .push((manifest_path.clone(), fs::read(&manifest_path).ok()));
            let manifest = json!({
                "scene": {
                    "name": ctx.scene.name,
                    "entity_count": ctx.scene.entities.len()
                },
                "build": {
                    "target": ctx.build.target,
                    "bundle_id": ctx.build.bundle_id,
                    "version": ctx.build.version,
                    "features": ctx.build.enabled_features
                },
                "exported_utc": chrono::Utc::now().to_rfc3339()
            });
            fs::write(
                &manifest_path,
                serde_json::to_string_pretty(&manifest).context("serialize export manifest")?,
            )
            .with_context(|| format!("failed to write '{}'", manifest_path.display()))?;
            ctx.build.last_export_path = Some(export_dir.display().to_string());
            payload["path"] = Value::String(export_dir.display().to_string());
            payload["manifest_path"] = Value::String(manifest_path.display().to_string());
        } else if op == "generate_installer" {
            let raw_path = sp("path").with_context(|| "missing path")?;
            let resolved = resolve_project_path(&ctx.project_root, Path::new(&raw_path));
            let installer_path = if resolved.extension().is_some() {
                if let Some(parent) = resolved.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!(
                            "failed to create installer directory '{}'",
                            parent.display()
                        )
                    })?;
                }
                resolved
            } else {
                fs::create_dir_all(&resolved).with_context(|| {
                    format!(
                        "failed to create installer directory '{}'",
                        resolved.display()
                    )
                })?;
                resolved.join("installer_manifest.json")
            };
            self.file_backups
                .push((installer_path.clone(), fs::read(&installer_path).ok()));
            let installer = json!({
                "bundle_id": ctx.build.bundle_id,
                "version": ctx.build.version,
                "target": ctx.build.target,
                "features": ctx.build.enabled_features,
                "generated_utc": chrono::Utc::now().to_rfc3339()
            });
            fs::write(
                &installer_path,
                serde_json::to_string_pretty(&installer).context("serialize installer")?,
            )
            .with_context(|| format!("failed to write '{}'", installer_path.display()))?;
            ctx.build.last_installer_path = Some(installer_path.display().to_string());
            payload["installer_path"] = Value::String(installer_path.display().to_string());
        } else {
            bail!("unsupported build operation '{}'", op);
        }

        ctx.scene_runtime.last_message = Some(format!("build operation '{}' applied", op));
        Ok(CommandResult::new(
            format!("build operation '{}' applied", op),
            payload,
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_build) = &self.previous_build {
            ctx.build = previous_build.clone();
        }
        for (path, previous_bytes) in self.file_backups.iter().rev() {
            match previous_bytes {
                Some(bytes) => fs::write(path, bytes)
                    .with_context(|| format!("failed to restore '{}'", path.display()))?,
                None => {
                    if path.exists() {
                        fs::remove_file(path).with_context(|| {
                            format!("failed to remove '{}' during undo", path.display())
                        })?;
                    }
                }
            }
        }
        if let Some(previous_last_message) = &self.previous_last_message {
            ctx.scene_runtime.last_message = previous_last_message.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "operation": self.operation,
            "params": self.params
        })
    }
}

#[derive(Debug, Clone)]
pub struct DebugMutationCommand {
    operation: String,
    params: Value,
    previous_debug: Option<DebugRuntimeState>,
    previous_last_message: Option<Option<String>>,
}

impl DebugMutationCommand {
    pub fn new(operation: impl Into<String>, params: Value) -> Self {
        Self {
            operation: operation.into(),
            params,
            previous_debug: None,
            previous_last_message: None,
        }
    }

    fn op(&self) -> String {
        self.operation.trim().to_ascii_lowercase()
    }
}

impl EngineCommand for DebugMutationCommand {
    fn name(&self) -> &'static str {
        "debug.mutation"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        let op = self.op();
        if [
            "show_colliders",
            "show_navmesh",
            "toggle_wireframe",
            "capture_frame",
        ]
        .contains(&op.as_str())
        {
            return ValidationResult::ok();
        }
        ValidationResult::invalid(format!("unsupported debug operation '{}'", op))
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_debug = Some(ctx.debug.clone());
        self.previous_last_message = Some(ctx.scene_runtime.last_message.clone());

        let op = self.op();
        let on = self
            .params
            .get("on")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let mut payload = json!({"operation": op});
        if op == "show_colliders" {
            ctx.debug.show_colliders = on;
            payload["on"] = Value::Bool(on);
        } else if op == "show_navmesh" {
            ctx.debug.show_navmesh = on;
            payload["on"] = Value::Bool(on);
        } else if op == "toggle_wireframe" {
            ctx.debug.wireframe = on;
            payload["on"] = Value::Bool(on);
        } else if op == "capture_frame" {
            ctx.debug.captured_frames = ctx.debug.captured_frames.saturating_add(1);
            let snapshot = create_debug_profiler_snapshot(ctx);
            ctx.debug.profiler_snapshots.push(snapshot.clone());
            if ctx.debug.profiler_snapshots.len() > 64 {
                let overflow = ctx.debug.profiler_snapshots.len() - 64;
                ctx.debug.profiler_snapshots.drain(0..overflow);
            }
            payload["captured_frames"] = json!(ctx.debug.captured_frames);
            payload["snapshot"] = json!(snapshot);
        } else {
            bail!("unsupported debug operation '{}'", op);
        }

        ctx.scene_runtime.last_message = Some(format!("debug operation '{}' applied", op));
        Ok(CommandResult::new(
            format!("debug operation '{}' applied", op),
            payload,
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_debug) = &self.previous_debug {
            ctx.debug = previous_debug.clone();
        }
        if let Some(previous_last_message) = &self.previous_last_message {
            ctx.scene_runtime.last_message = previous_last_message.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "operation": self.operation,
            "params": self.params
        })
    }
}

#[derive(Debug, Clone)]
pub struct AssetImportFileCommand {
    source_path: PathBuf,
    target_subdir: PathBuf,
    previous_record: Option<ImportedAssetRecord>,
    previous_file_contents: Option<Vec<u8>>,
    target_path: Option<PathBuf>,
    asset_id: Option<String>,
}

impl AssetImportFileCommand {
    pub fn new(source_path: impl Into<PathBuf>, target_subdir: impl Into<PathBuf>) -> Self {
        Self {
            source_path: source_path.into(),
            target_subdir: target_subdir.into(),
            previous_record: None,
            previous_file_contents: None,
            target_path: None,
            asset_id: None,
        }
    }
}

impl EngineCommand for AssetImportFileCommand {
    fn name(&self) -> &'static str {
        "asset.import_file"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.source_path.as_os_str().is_empty() {
            ValidationResult::invalid("path cannot be empty")
        } else {
            ValidationResult::ok()
        }
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let source_path = resolve_project_path(&ctx.project_root, &self.source_path);
        if !source_path.exists() {
            bail!("asset source '{}' does not exist", source_path.display());
        }
        if !source_path.is_file() {
            bail!("asset source '{}' is not a file", source_path.display());
        }

        let file_name = source_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_string())
            .with_context(|| format!("invalid source file name '{}'", source_path.display()))?;

        let target_dir = resolve_project_path(&ctx.project_root, &self.target_subdir);
        fs::create_dir_all(&target_dir).with_context(|| {
            format!(
                "failed to create target import directory '{}'",
                target_dir.display()
            )
        })?;
        let target_path = target_dir.join(&file_name);

        self.previous_file_contents = fs::read(&target_path).ok();
        fs::copy(&source_path, &target_path).with_context(|| {
            format!(
                "failed to copy '{}' to '{}'",
                source_path.display(),
                target_path.display()
            )
        })?;

        let asset_id = target_path
            .strip_prefix(&ctx.project_root)
            .unwrap_or(target_path.as_path())
            .to_string_lossy()
            .replace('\\', "/");

        self.previous_record = ctx.imported_assets.get(&asset_id).cloned();
        let kind = source_path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("unknown")
            .to_ascii_lowercase();
        let record = ImportedAssetRecord {
            asset_id: asset_id.clone(),
            source_path: source_path.display().to_string(),
            imported_path: target_path.display().to_string(),
            kind,
        };
        ctx.imported_assets.insert(asset_id.clone(), record);

        self.target_path = Some(target_path.clone());
        self.asset_id = Some(asset_id.clone());
        Ok(CommandResult::new(
            "asset imported",
            json!({
                "asset_id": asset_id,
                "source_path": source_path.display().to_string(),
                "imported_path": target_path.display().to_string()
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        let Some(asset_id) = &self.asset_id else {
            return Ok(());
        };
        let Some(target_path) = &self.target_path else {
            return Ok(());
        };

        match &self.previous_file_contents {
            Some(contents) => {
                fs::write(target_path, contents).with_context(|| {
                    format!(
                        "failed to restore previous imported file '{}'",
                        target_path.display()
                    )
                })?;
            }
            None => {
                if target_path.exists() {
                    fs::remove_file(target_path).with_context(|| {
                        format!(
                            "failed to remove imported file '{}' during undo",
                            target_path.display()
                        )
                    })?;
                }
            }
        }

        if let Some(previous_record) = &self.previous_record {
            ctx.imported_assets
                .insert(asset_id.clone(), previous_record.clone());
        } else {
            ctx.imported_assets.remove(asset_id);
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "path": self.source_path,
            "target_subdir": self.target_subdir
        })
    }
}

#[derive(Debug, Clone)]
pub struct AssetCreateMaterialCommand {
    name: String,
    preset: String,
    params: Value,
    previous_record: Option<MaterialRecord>,
    previous_file_contents: Option<String>,
    material_path: Option<PathBuf>,
}

impl AssetCreateMaterialCommand {
    pub fn new(name: impl Into<String>, preset: impl Into<String>, params: Value) -> Self {
        Self {
            name: name.into(),
            preset: preset.into(),
            params,
            previous_record: None,
            previous_file_contents: None,
            material_path: None,
        }
    }
}

impl EngineCommand for AssetCreateMaterialCommand {
    fn name(&self) -> &'static str {
        "asset.create_material"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.name.trim().is_empty() {
            return ValidationResult::invalid("name cannot be empty");
        }
        if self.preset.trim().is_empty() {
            return ValidationResult::invalid("preset cannot be empty");
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let materials_dir = ctx.project_root.join("assets").join("materials");
        fs::create_dir_all(&materials_dir).with_context(|| {
            format!(
                "failed to create materials directory '{}'",
                materials_dir.display()
            )
        })?;

        let sanitized = sanitize_file_stem(&self.name);
        let material_path = materials_dir.join(format!("{}.material.json", sanitized));
        self.previous_file_contents = fs::read_to_string(&material_path).ok();
        self.previous_record = ctx.materials.get(&self.name).cloned();

        let record = MaterialRecord {
            name: self.name.clone(),
            preset: self.preset.clone(),
            params: self.params.clone(),
            file_path: Some(material_path.display().to_string()),
        };
        let serialized =
            serde_json::to_string_pretty(&record).context("failed to serialize material record")?;
        fs::write(&material_path, serialized).with_context(|| {
            format!(
                "failed to write material file '{}'",
                material_path.display()
            )
        })?;

        ctx.materials.insert(self.name.clone(), record.clone());
        self.material_path = Some(material_path.clone());
        Ok(CommandResult::new(
            format!("material '{}' created", self.name),
            json!({
                "name": record.name,
                "preset": record.preset,
                "file_path": record.file_path
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        let Some(material_path) = &self.material_path else {
            return Ok(());
        };
        match &self.previous_file_contents {
            Some(contents) => {
                fs::write(material_path, contents).with_context(|| {
                    format!(
                        "failed to restore material file '{}'",
                        material_path.display()
                    )
                })?;
            }
            None => {
                if material_path.exists() {
                    fs::remove_file(material_path).with_context(|| {
                        format!(
                            "failed to remove material file '{}' during undo",
                            material_path.display()
                        )
                    })?;
                }
            }
        }

        if let Some(previous_record) = &self.previous_record {
            ctx.materials
                .insert(previous_record.name.clone(), previous_record.clone());
        } else {
            ctx.materials.remove(&self.name);
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "name": self.name,
            "preset": self.preset,
            "params": self.params
        })
    }
}

#[derive(Debug, Clone)]
pub struct AssetInstantiatePrefabCommand {
    asset_id: String,
    entity_name: String,
    translation: [f32; 3],
    created_index: Option<usize>,
}

impl AssetInstantiatePrefabCommand {
    pub fn new(
        asset_id: impl Into<String>,
        entity_name: impl Into<String>,
        translation: [f32; 3],
    ) -> Self {
        Self {
            asset_id: asset_id.into(),
            entity_name: entity_name.into(),
            translation,
            created_index: None,
        }
    }
}

impl EngineCommand for AssetInstantiatePrefabCommand {
    fn name(&self) -> &'static str {
        "asset.instantiate_prefab"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.asset_id.trim().is_empty() {
            return ValidationResult::invalid("prefab_id/asset_id cannot be empty");
        }
        if self.entity_name.trim().is_empty() {
            return ValidationResult::invalid("entity_name cannot be empty");
        }
        if ctx.entity_exists(&self.entity_name) {
            return ValidationResult::invalid(format!(
                "entity '{}' already exists; entity names must be unique",
                self.entity_name
            ));
        }
        let asset_exists = if ctx.imported_assets.contains_key(&self.asset_id) {
            true
        } else {
            let resolved = resolve_project_path(&ctx.project_root, Path::new(&self.asset_id));
            resolved.exists() && resolved.is_file()
        };
        if !asset_exists {
            return ValidationResult::invalid(format!(
                "asset '{}' is not known/imported and no file was found at that path",
                self.asset_id
            ));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let entity = SceneEntity {
            name: self.entity_name.clone(),
            mesh: self.asset_id.clone(),
            translation: self.translation,
        };
        ctx.scene.entities.push(entity);
        self.created_index = Some(ctx.scene.entities.len().saturating_sub(1));
        Ok(CommandResult::new(
            format!(
                "asset '{}' instantiated as entity '{}'",
                self.asset_id, self.entity_name
            ),
            json!({
                "asset_id": self.asset_id,
                "entity_name": self.entity_name,
                "translation": self.translation
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(index) = self.created_index
            && index < ctx.scene.entities.len()
            && ctx.scene.entities[index].name == self.entity_name
            && ctx.scene.entities[index].mesh == self.asset_id
        {
            ctx.scene.entities.remove(index);
            return Ok(());
        }
        if let Some(found_index) = ctx
            .scene
            .entities
            .iter()
            .position(|entity| entity.name == self.entity_name && entity.mesh == self.asset_id)
        {
            ctx.scene.entities.remove(found_index);
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "prefab_id": self.asset_id,
            "entity_name": self.entity_name,
            "transform": {
                "position": self.translation
            }
        })
    }
}

#[derive(Debug, Clone)]
struct FileUndoSnapshot {
    path: PathBuf,
    previous_contents: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct AssetPipelineMutationCommand {
    operation: String,
    params: Value,
    previous_imported_assets: Option<BTreeMap<String, ImportedAssetRecord>>,
    previous_textures: Option<BTreeMap<String, AssetTextureRecord>>,
    previous_shaders: Option<BTreeMap<String, AssetShaderRecord>>,
    previous_prefabs: Option<BTreeMap<String, AssetPrefabRecord>>,
    previous_pipeline: Option<AssetPipelineRuntimeState>,
    previous_last_message: Option<Option<String>>,
    file_undo: Vec<FileUndoSnapshot>,
}

impl AssetPipelineMutationCommand {
    pub fn new(operation: impl Into<String>, params: Value) -> Self {
        Self {
            operation: operation.into(),
            params,
            previous_imported_assets: None,
            previous_textures: None,
            previous_shaders: None,
            previous_prefabs: None,
            previous_pipeline: None,
            previous_last_message: None,
            file_undo: Vec::new(),
        }
    }

    fn op(&self) -> String {
        self.operation.trim().to_ascii_lowercase()
    }

    fn remember_file_snapshot(&mut self, path: &Path) {
        if self.file_undo.iter().any(|snapshot| snapshot.path == path) {
            return;
        }
        self.file_undo.push(FileUndoSnapshot {
            path: path.to_path_buf(),
            previous_contents: fs::read(path).ok(),
        });
    }

    fn derive_download_name(url: &str) -> String {
        let parsed = reqwest::Url::parse(url).ok();
        let candidate = parsed
            .as_ref()
            .and_then(|parsed| {
                Path::new(parsed.path())
                    .file_name()
                    .and_then(|value| value.to_str())
            })
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "/")
            .map(str::to_string);
        let base_name = candidate.unwrap_or_else(|| {
            let digest = sha2::Sha256::digest(url.as_bytes());
            format!("download_{:x}.bin", digest)
        });
        if Path::new(&base_name).extension().is_some() {
            base_name
        } else {
            format!("{}.bin", sanitize_file_stem(&base_name))
        }
    }

    fn write_json_file(
        &mut self,
        path: &Path,
        payload: &Value,
        error_context: &str,
    ) -> anyhow::Result<()> {
        self.remember_file_snapshot(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create directory '{}'", parent.display()))?;
        }
        let serialized = serde_json::to_string_pretty(payload)
            .with_context(|| format!("failed to serialize {}", error_context))?;
        fs::write(path, serialized)
            .with_context(|| format!("failed to write {} '{}'", error_context, path.display()))?;
        Ok(())
    }

    fn read_prefab_components(ctx: &CommandContext, entity_id: &str) -> BTreeMap<String, Value> {
        ctx.components.get(entity_id).cloned().unwrap_or_default()
    }
}

impl EngineCommand for AssetPipelineMutationCommand {
    fn name(&self) -> &'static str {
        "asset.pipeline_mutation"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        let op = self.op();
        let sp = |key: &str| self.params.get(key).and_then(Value::as_str).map(str::trim);

        if op == "import_url" {
            let Some(url) = sp("url").filter(|value| !value.is_empty()) else {
                return ValidationResult::invalid("url cannot be empty");
            };
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return ValidationResult::invalid("url must start with http:// or https://");
            }
            return ValidationResult::ok();
        }
        if op == "create_texture" {
            if sp("name").unwrap_or_default().is_empty() {
                return ValidationResult::invalid("name cannot be empty");
            }
            let width = self
                .params
                .get("width")
                .and_then(Value::as_u64)
                .unwrap_or(1024);
            let height = self
                .params
                .get("height")
                .and_then(Value::as_u64)
                .unwrap_or(1024);
            if width == 0 || height == 0 {
                return ValidationResult::invalid("width/height must be > 0");
            }
            return ValidationResult::ok();
        }
        if op == "create_shader" {
            if sp("name").unwrap_or_default().is_empty() {
                return ValidationResult::invalid("name cannot be empty");
            }
            if sp("template").unwrap_or_default().is_empty() {
                return ValidationResult::invalid("template cannot be empty");
            }
            return ValidationResult::ok();
        }
        if op == "create_prefab" {
            if sp("name").unwrap_or_default().is_empty() {
                return ValidationResult::invalid("name cannot be empty");
            }
            let Some(entity_id) = sp("entity_id").filter(|value| !value.is_empty()) else {
                return ValidationResult::invalid("entity_id cannot be empty");
            };
            if !ctx.entity_exists(entity_id) {
                return ValidationResult::invalid(format!(
                    "entity '{}' does not exist in open scene",
                    entity_id
                ));
            }
            return ValidationResult::ok();
        }
        if op == "save_prefab" {
            let Some(prefab_id) = sp("prefab_id").filter(|value| !value.is_empty()) else {
                return ValidationResult::invalid("prefab_id cannot be empty");
            };
            if !ctx.prefabs.contains_key(prefab_id) {
                return ValidationResult::invalid(format!("prefab '{}' does not exist", prefab_id));
            }
            return ValidationResult::ok();
        }
        if op == "rebuild_import" {
            let Some(asset_id) = sp("asset_id").filter(|value| !value.is_empty()) else {
                return ValidationResult::invalid("asset_id cannot be empty");
            };
            if !ctx.imported_assets.contains_key(asset_id) {
                let resolved = resolve_project_path(&ctx.project_root, Path::new(asset_id));
                if !resolved.exists() {
                    return ValidationResult::invalid(format!(
                        "asset '{}' not found in imported registry or on disk",
                        asset_id
                    ));
                }
            }
            return ValidationResult::ok();
        }
        if op == "generate_lods" || op == "mesh_optimize" {
            let Some(mesh_id) = sp("mesh_id").filter(|value| !value.is_empty()) else {
                return ValidationResult::invalid("mesh_id cannot be empty");
            };
            if !model_mesh_exists(ctx, mesh_id) {
                return ValidationResult::invalid(format!("mesh '{}' not found", mesh_id));
            }
            return ValidationResult::ok();
        }
        if op == "compress_textures" {
            let Some(asset_id) = sp("asset_id").filter(|value| !value.is_empty()) else {
                return ValidationResult::invalid("asset_id cannot be empty");
            };
            if !ctx.textures.contains_key(asset_id) && !ctx.imported_assets.contains_key(asset_id) {
                return ValidationResult::invalid(format!(
                    "asset '{}' not found in textures/imported assets",
                    asset_id
                ));
            }
            return ValidationResult::ok();
        }
        if op == "bake_lightmaps" || op == "bake_reflection_probes" {
            return ValidationResult::ok();
        }
        ValidationResult::invalid(format!("unsupported asset pipeline operation '{}'", op))
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_imported_assets = Some(ctx.imported_assets.clone());
        self.previous_textures = Some(ctx.textures.clone());
        self.previous_shaders = Some(ctx.shaders.clone());
        self.previous_prefabs = Some(ctx.prefabs.clone());
        self.previous_pipeline = Some(ctx.asset_pipeline.clone());
        self.previous_last_message = Some(ctx.scene_runtime.last_message.clone());

        let op = self.op();
        let sp = |key: &str| {
            self.params
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        let mut payload = json!({ "operation": op });

        if op == "import_url" {
            let url = sp("url").with_context(|| "missing url")?;
            let target_subdir =
                sp("target_subdir").unwrap_or_else(|| "assets/imported".to_string());
            let target_dir = resolve_project_path(&ctx.project_root, Path::new(&target_subdir));
            fs::create_dir_all(&target_dir).with_context(|| {
                format!(
                    "failed to create target import directory '{}'",
                    target_dir.display()
                )
            })?;

            let response = reqwest::blocking::get(&url)
                .with_context(|| format!("failed to GET '{}'", url))?
                .error_for_status()
                .with_context(|| format!("request failed for '{}'", url))?;
            let bytes = response
                .bytes()
                .with_context(|| format!("failed to read bytes from '{}'", url))?;

            let file_name = sp("file_name").unwrap_or_else(|| Self::derive_download_name(&url));
            let target_path = target_dir.join(file_name);
            self.remember_file_snapshot(&target_path);
            fs::write(&target_path, &bytes).with_context(|| {
                format!(
                    "failed to write downloaded asset '{}'",
                    target_path.display()
                )
            })?;

            let asset_id = target_path
                .strip_prefix(&ctx.project_root)
                .unwrap_or(target_path.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            let kind = target_path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("unknown")
                .to_ascii_lowercase();
            ctx.imported_assets.insert(
                asset_id.clone(),
                ImportedAssetRecord {
                    asset_id: asset_id.clone(),
                    source_path: url.clone(),
                    imported_path: target_path.display().to_string(),
                    kind,
                },
            );
            payload["asset_id"] = Value::String(asset_id);
            payload["source_url"] = Value::String(url);
            payload["imported_path"] = Value::String(target_path.display().to_string());
            payload["bytes"] = json!(bytes.len());
        } else if op == "create_texture" {
            let name = sp("name").with_context(|| "missing name")?;
            let texture_id = sp("texture_id").unwrap_or_else(|| normalize_identifier(&name));
            let width = self
                .params
                .get("width")
                .and_then(Value::as_u64)
                .unwrap_or(1024)
                .clamp(1, 16384) as u32;
            let height = self
                .params
                .get("height")
                .and_then(Value::as_u64)
                .unwrap_or(1024)
                .clamp(1, 16384) as u32;
            let format = sp("format").unwrap_or_else(|| "rgba8".to_string());
            let texture_params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));

            let path = ctx
                .project_root
                .join("assets")
                .join("textures")
                .join(format!("{}.texture.json", sanitize_file_stem(&texture_id)));
            let record = AssetTextureRecord {
                texture_id: texture_id.clone(),
                name: name.clone(),
                width,
                height,
                format: format.clone(),
                params: texture_params.clone(),
                file_path: Some(path.display().to_string()),
            };
            self.write_json_file(
                &path,
                &json!({
                    "texture_id": record.texture_id,
                    "name": record.name,
                    "width": record.width,
                    "height": record.height,
                    "format": record.format,
                    "params": record.params
                }),
                "texture descriptor",
            )?;
            ctx.textures.insert(texture_id.clone(), record);
            payload["texture_id"] = Value::String(texture_id);
            payload["file_path"] = Value::String(path.display().to_string());
        } else if op == "create_shader" {
            let name = sp("name").with_context(|| "missing name")?;
            let shader_id = sp("shader_id").unwrap_or_else(|| normalize_identifier(&name));
            let template = sp("template").with_context(|| "missing template")?;
            let shader_params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let path = ctx
                .project_root
                .join("assets")
                .join("shaders")
                .join(format!("{}.shader.json", sanitize_file_stem(&shader_id)));
            let record = AssetShaderRecord {
                shader_id: shader_id.clone(),
                name: name.clone(),
                template: template.clone(),
                params: shader_params.clone(),
                file_path: Some(path.display().to_string()),
            };
            self.write_json_file(
                &path,
                &json!({
                    "shader_id": record.shader_id,
                    "name": record.name,
                    "template": record.template,
                    "params": record.params
                }),
                "shader descriptor",
            )?;
            ctx.shaders.insert(shader_id.clone(), record);
            payload["shader_id"] = Value::String(shader_id);
            payload["file_path"] = Value::String(path.display().to_string());
        } else if op == "create_prefab" {
            let name = sp("name").with_context(|| "missing name")?;
            let entity_id = sp("entity_id").with_context(|| "missing entity_id")?;
            let prefab_id = sp("prefab_id").unwrap_or_else(|| normalize_identifier(&name));
            let entity = ctx
                .scene
                .entities
                .iter()
                .find(|entity| entity.name == entity_id)
                .cloned()
                .with_context(|| format!("entity '{}' not found", entity_id))?;
            let components = Self::read_prefab_components(ctx, &entity_id);
            let metadata = self
                .params
                .get("metadata")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let path = ctx
                .project_root
                .join("assets")
                .join("prefabs")
                .join(format!("{}.prefab.json", sanitize_file_stem(&prefab_id)));
            let record = AssetPrefabRecord {
                prefab_id: prefab_id.clone(),
                name: name.clone(),
                source_entity_id: entity_id.clone(),
                entity: entity.clone(),
                components: components.clone(),
                metadata: metadata.clone(),
                file_path: Some(path.display().to_string()),
                last_saved_utc: chrono::Utc::now().to_rfc3339(),
            };
            self.write_json_file(
                &path,
                &json!({
                    "prefab_id": record.prefab_id,
                    "name": record.name,
                    "source_entity_id": record.source_entity_id,
                    "entity": record.entity,
                    "components": record.components,
                    "metadata": record.metadata,
                    "last_saved_utc": record.last_saved_utc
                }),
                "prefab descriptor",
            )?;
            ctx.prefabs.insert(prefab_id.clone(), record);
            payload["prefab_id"] = Value::String(prefab_id);
            payload["entity_id"] = Value::String(entity_id);
            payload["file_path"] = Value::String(path.display().to_string());
        } else if op == "save_prefab" {
            let prefab_id = sp("prefab_id").with_context(|| "missing prefab_id")?;
            let mut record = ctx
                .prefabs
                .get(&prefab_id)
                .cloned()
                .with_context(|| format!("prefab '{}' not found", prefab_id))?;

            if let Some(entity) = ctx
                .scene
                .entities
                .iter()
                .find(|entity| entity.name == record.source_entity_id)
            {
                record.entity = entity.clone();
                record.components = Self::read_prefab_components(ctx, &record.source_entity_id);
            }
            record.last_saved_utc = chrono::Utc::now().to_rfc3339();
            let path = record
                .file_path
                .as_ref()
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    ctx.project_root
                        .join("assets")
                        .join("prefabs")
                        .join(format!("{}.prefab.json", sanitize_file_stem(&prefab_id)))
                });
            record.file_path = Some(path.display().to_string());
            self.write_json_file(
                &path,
                &json!({
                    "prefab_id": record.prefab_id,
                    "name": record.name,
                    "source_entity_id": record.source_entity_id,
                    "entity": record.entity,
                    "components": record.components,
                    "metadata": record.metadata,
                    "last_saved_utc": record.last_saved_utc
                }),
                "prefab descriptor",
            )?;
            ctx.prefabs.insert(prefab_id.clone(), record.clone());
            payload["prefab_id"] = Value::String(prefab_id);
            payload["saved_at"] = Value::String(record.last_saved_utc);
            payload["file_path"] = Value::String(path.display().to_string());
        } else if op == "rebuild_import" {
            let asset_id = sp("asset_id").with_context(|| "missing asset_id")?;
            if !ctx.imported_assets.contains_key(&asset_id) {
                let path = resolve_project_path(&ctx.project_root, Path::new(&asset_id));
                let kind = path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .unwrap_or("unknown")
                    .to_ascii_lowercase();
                ctx.imported_assets.insert(
                    asset_id.clone(),
                    ImportedAssetRecord {
                        asset_id: asset_id.clone(),
                        source_path: path.display().to_string(),
                        imported_path: path.display().to_string(),
                        kind,
                    },
                );
            }
            let params = self
                .params
                .get("params")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let rebuild = AssetRebuildRecord {
                asset_id: asset_id.clone(),
                params: params.clone(),
                timestamp_utc: chrono::Utc::now().to_rfc3339(),
            };
            ctx.asset_pipeline.rebuilds.push(rebuild.clone());
            if ctx.asset_pipeline.rebuilds.len() > 128 {
                let overflow = ctx.asset_pipeline.rebuilds.len() - 128;
                ctx.asset_pipeline.rebuilds.drain(0..overflow);
            }
            payload["asset_id"] = Value::String(asset_id);
            payload["rebuild"] = serde_json::to_value(rebuild)?;
        } else if op == "generate_lods" {
            let mesh_id = sp("mesh_id").with_context(|| "missing mesh_id")?;
            let levels = self
                .params
                .get("levels")
                .or_else(|| self.params.get("lod_levels"))
                .and_then(Value::as_u64)
                .unwrap_or(3)
                .clamp(1, 8) as u32;
            let reduction = self
                .params
                .get("reduction")
                .and_then(Value::as_f64)
                .map(|value| value as f32)
                .unwrap_or(0.5)
                .clamp(0.05, 1.0);
            let lod = AssetLodRecord {
                mesh_id: mesh_id.clone(),
                levels,
                reduction,
                params: self
                    .params
                    .get("params")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
                timestamp_utc: chrono::Utc::now().to_rfc3339(),
            };
            ctx.asset_pipeline.lods.insert(mesh_id.clone(), lod.clone());
            payload["mesh_id"] = Value::String(mesh_id);
            payload["lod"] = serde_json::to_value(lod)?;
        } else if op == "mesh_optimize" {
            let mesh_id = sp("mesh_id").with_context(|| "missing mesh_id")?;
            let profile = sp("profile").unwrap_or_else(|| "balanced".to_string());
            let optimization = AssetMeshOptimizationRecord {
                mesh_id: mesh_id.clone(),
                profile: profile.clone(),
                params: self
                    .params
                    .get("params")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
                timestamp_utc: chrono::Utc::now().to_rfc3339(),
            };
            ctx.asset_pipeline
                .mesh_optimizations
                .insert(mesh_id.clone(), optimization.clone());
            payload["mesh_id"] = Value::String(mesh_id);
            payload["optimization"] = serde_json::to_value(optimization)?;
        } else if op == "compress_textures" {
            let asset_id = sp("asset_id").with_context(|| "missing asset_id")?;
            let format = sp("format").unwrap_or_else(|| "bc7".to_string());
            let quality = sp("quality").unwrap_or_else(|| "balanced".to_string());
            let compression = AssetTextureCompressionRecord {
                asset_id: asset_id.clone(),
                format: format.clone(),
                quality: quality.clone(),
                params: self
                    .params
                    .get("params")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
                timestamp_utc: chrono::Utc::now().to_rfc3339(),
            };
            ctx.asset_pipeline
                .texture_compressions
                .insert(asset_id.clone(), compression.clone());
            payload["asset_id"] = Value::String(asset_id);
            payload["compression"] = serde_json::to_value(compression)?;
        } else if op == "bake_lightmaps" {
            let bake = AssetBakeRecord {
                bake_id: format!(
                    "lightmap_bake_{}",
                    ctx.asset_pipeline.lightmap_bakes.len() + 1
                ),
                params: self
                    .params
                    .get("params")
                    .cloned()
                    .unwrap_or_else(|| self.params.clone()),
                timestamp_utc: chrono::Utc::now().to_rfc3339(),
            };
            ctx.asset_pipeline.lightmap_bakes.push(bake.clone());
            if ctx.asset_pipeline.lightmap_bakes.len() > 64 {
                let overflow = ctx.asset_pipeline.lightmap_bakes.len() - 64;
                ctx.asset_pipeline.lightmap_bakes.drain(0..overflow);
            }
            payload["bake"] = serde_json::to_value(bake)?;
        } else if op == "bake_reflection_probes" {
            let bake = AssetBakeRecord {
                bake_id: format!(
                    "reflection_probe_bake_{}",
                    ctx.asset_pipeline.reflection_probe_bakes.len() + 1
                ),
                params: self
                    .params
                    .get("params")
                    .cloned()
                    .unwrap_or_else(|| self.params.clone()),
                timestamp_utc: chrono::Utc::now().to_rfc3339(),
            };
            ctx.asset_pipeline.reflection_probe_bakes.push(bake.clone());
            if ctx.asset_pipeline.reflection_probe_bakes.len() > 64 {
                let overflow = ctx.asset_pipeline.reflection_probe_bakes.len() - 64;
                ctx.asset_pipeline.reflection_probe_bakes.drain(0..overflow);
            }
            payload["bake"] = serde_json::to_value(bake)?;
        } else {
            bail!("unsupported asset pipeline operation '{}'", op);
        }

        ctx.scene_runtime.last_message = Some(format!("asset operation '{}' applied", op));
        Ok(CommandResult::new(
            format!("asset operation '{}' applied", op),
            payload,
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_imported_assets) = &self.previous_imported_assets {
            ctx.imported_assets = previous_imported_assets.clone();
        }
        if let Some(previous_textures) = &self.previous_textures {
            ctx.textures = previous_textures.clone();
        }
        if let Some(previous_shaders) = &self.previous_shaders {
            ctx.shaders = previous_shaders.clone();
        }
        if let Some(previous_prefabs) = &self.previous_prefabs {
            ctx.prefabs = previous_prefabs.clone();
        }
        if let Some(previous_pipeline) = &self.previous_pipeline {
            ctx.asset_pipeline = previous_pipeline.clone();
        }
        if let Some(previous_last_message) = &self.previous_last_message {
            ctx.scene_runtime.last_message = previous_last_message.clone();
        }
        for snapshot in self.file_undo.iter().rev() {
            if let Some(previous_contents) = &snapshot.previous_contents {
                fs::write(&snapshot.path, previous_contents).with_context(|| {
                    format!(
                        "failed to restore file '{}' during asset undo",
                        snapshot.path.display()
                    )
                })?;
            } else if snapshot.path.exists() {
                fs::remove_file(&snapshot.path).with_context(|| {
                    format!(
                        "failed to remove created file '{}' during asset undo",
                        snapshot.path.display()
                    )
                })?;
            }
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "operation": self.operation,
            "params": self.params
        })
    }
}

#[derive(Debug, Clone)]
pub struct RenderSetLightCommand {
    direction: [f32; 3],
    color: [f32; 3],
    intensity: f32,
    shadow_bias: f32,
    shadow_strength: f32,
    shadow_cascade_count: u32,
    previous_settings: Option<RenderSettings>,
}

impl RenderSetLightCommand {
    pub fn new(
        direction: [f32; 3],
        color: [f32; 3],
        intensity: f32,
        shadow_bias: f32,
        shadow_strength: f32,
        shadow_cascade_count: u32,
    ) -> Self {
        Self {
            direction,
            color,
            intensity,
            shadow_bias,
            shadow_strength,
            shadow_cascade_count,
            previous_settings: None,
        }
    }
}

impl EngineCommand for RenderSetLightCommand {
    fn name(&self) -> &'static str {
        "render.set_light_params"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.intensity <= 0.0 {
            return ValidationResult::invalid("intensity must be > 0");
        }
        if self.shadow_bias <= 0.0 {
            return ValidationResult::invalid("shadow_bias must be > 0");
        }
        if !(0.0..=1.0).contains(&self.shadow_strength) {
            return ValidationResult::invalid("shadow_strength must be in [0,1]");
        }
        if !(1..=3).contains(&self.shadow_cascade_count) {
            return ValidationResult::invalid("shadow_cascade_count must be between 1 and 3");
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_settings = Some(ctx.render_settings.clone());
        ctx.render_settings.light_direction = normalize_direction(self.direction);
        ctx.render_settings.light_color = [
            self.color[0].clamp(0.0, 32.0),
            self.color[1].clamp(0.0, 32.0),
            self.color[2].clamp(0.0, 32.0),
        ];
        ctx.render_settings.light_intensity = self.intensity.clamp(0.01, 32.0);
        ctx.render_settings.shadow_bias = self.shadow_bias.clamp(0.0001, 0.01);
        ctx.render_settings.shadow_strength = self.shadow_strength.clamp(0.0, 1.0);
        ctx.render_settings.shadow_cascade_count = self.shadow_cascade_count.clamp(1, 3);
        Ok(CommandResult::new(
            "directional light updated",
            json!({
                "direction": ctx.render_settings.light_direction,
                "color": ctx.render_settings.light_color,
                "intensity": ctx.render_settings.light_intensity,
                "shadow_bias": ctx.render_settings.shadow_bias,
                "shadow_strength": ctx.render_settings.shadow_strength,
                "shadow_cascade_count": ctx.render_settings.shadow_cascade_count
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_settings) = &self.previous_settings {
            ctx.render_settings = previous_settings.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "direction": self.direction,
            "color": self.color,
            "intensity": self.intensity,
            "shadow_bias": self.shadow_bias,
            "shadow_strength": self.shadow_strength,
            "shadow_cascade_count": self.shadow_cascade_count
        })
    }
}

#[derive(Debug, Clone)]
pub struct RenderSetIblCommand {
    sky_color: [f32; 3],
    ground_color: [f32; 3],
    intensity: f32,
    previous_settings: Option<RenderSettings>,
}

impl RenderSetIblCommand {
    pub fn new(sky_color: [f32; 3], ground_color: [f32; 3], intensity: f32) -> Self {
        Self {
            sky_color,
            ground_color,
            intensity,
            previous_settings: None,
        }
    }
}

impl EngineCommand for RenderSetIblCommand {
    fn name(&self) -> &'static str {
        "render.set_ibl"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.intensity < 0.0 {
            return ValidationResult::invalid("ibl intensity must be >= 0");
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_settings = Some(ctx.render_settings.clone());
        ctx.render_settings.ibl_intensity = self.intensity.clamp(0.0, 4.0);
        ctx.render_settings.ibl_sky_color = [
            self.sky_color[0].clamp(0.0, 4.0),
            self.sky_color[1].clamp(0.0, 4.0),
            self.sky_color[2].clamp(0.0, 4.0),
        ];
        ctx.render_settings.ibl_ground_color = [
            self.ground_color[0].clamp(0.0, 4.0),
            self.ground_color[1].clamp(0.0, 4.0),
            self.ground_color[2].clamp(0.0, 4.0),
        ];
        Ok(CommandResult::new(
            "ibl updated",
            json!({
                "ibl_intensity": ctx.render_settings.ibl_intensity,
                "ibl_sky_color": ctx.render_settings.ibl_sky_color,
                "ibl_ground_color": ctx.render_settings.ibl_ground_color
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_settings) = &self.previous_settings {
            ctx.render_settings = previous_settings.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "sky_color": self.sky_color,
            "ground_color": self.ground_color,
            "intensity": self.intensity
        })
    }
}

#[derive(Debug, Clone)]
pub struct RenderSetPostprocessCommand {
    params: RenderPostprocessParams,
    previous_settings: Option<RenderSettings>,
}

impl RenderSetPostprocessCommand {
    pub fn new(params: RenderPostprocessParams) -> Self {
        Self {
            params,
            previous_settings: None,
        }
    }
}

impl EngineCommand for RenderSetPostprocessCommand {
    fn name(&self) -> &'static str {
        "render.set_postprocess"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.params.exposure <= 0.0 {
            return ValidationResult::invalid("exposure must be > 0");
        }
        if self.params.gamma <= 0.0 {
            return ValidationResult::invalid("gamma must be > 0");
        }
        if self.params.bloom_intensity < 0.0 {
            return ValidationResult::invalid("bloom_intensity must be >= 0");
        }
        if self.params.bloom_threshold < 0.0 {
            return ValidationResult::invalid("bloom_threshold must be >= 0");
        }
        if self.params.bloom_radius <= 0.0 {
            return ValidationResult::invalid("bloom_radius must be > 0");
        }
        if self.params.fog_density < 0.0 {
            return ValidationResult::invalid("fog_density must be >= 0");
        }
        if self.params.color_grading_preset.trim().is_empty() {
            return ValidationResult::invalid("color_grading_preset cannot be empty");
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_settings = Some(ctx.render_settings.clone());
        ctx.render_settings.exposure = self.params.exposure.clamp(0.05, 16.0);
        ctx.render_settings.gamma = self.params.gamma.clamp(0.2, 4.0);
        ctx.render_settings.bloom_intensity = self.params.bloom_intensity.clamp(0.0, 4.0);
        ctx.render_settings.bloom_threshold = self.params.bloom_threshold.clamp(0.0, 8.0);
        ctx.render_settings.bloom_radius = self.params.bloom_radius.clamp(0.1, 8.0);
        ctx.render_settings.fog_density = self.params.fog_density.clamp(0.0, 1.0);
        ctx.render_settings.fog_color = [
            self.params.fog_color[0].clamp(0.0, 4.0),
            self.params.fog_color[1].clamp(0.0, 4.0),
            self.params.fog_color[2].clamp(0.0, 4.0),
        ];
        ctx.render_settings.saturation = self.params.saturation.clamp(0.0, 2.0);
        ctx.render_settings.contrast = self.params.contrast.clamp(0.5, 2.0);
        ctx.render_settings.white_balance = self.params.white_balance.clamp(-1.0, 1.0);
        ctx.render_settings.grade_tint = [
            self.params.grade_tint[0].clamp(0.0, 2.0),
            self.params.grade_tint[1].clamp(0.0, 2.0),
            self.params.grade_tint[2].clamp(0.0, 2.0),
        ];
        ctx.render_settings.color_grading_preset = self.params.color_grading_preset.clone();
        Ok(CommandResult::new(
            "postprocess updated",
            json!({
                "exposure": ctx.render_settings.exposure,
                "gamma": ctx.render_settings.gamma,
                "bloom_intensity": ctx.render_settings.bloom_intensity,
                "bloom_threshold": ctx.render_settings.bloom_threshold,
                "bloom_radius": ctx.render_settings.bloom_radius,
                "fog_density": ctx.render_settings.fog_density,
                "fog_color": ctx.render_settings.fog_color,
                "saturation": ctx.render_settings.saturation,
                "contrast": ctx.render_settings.contrast,
                "white_balance": ctx.render_settings.white_balance,
                "grade_tint": ctx.render_settings.grade_tint,
                "color_grading_preset": ctx.render_settings.color_grading_preset
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_settings) = &self.previous_settings {
            ctx.render_settings = previous_settings.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "exposure": self.params.exposure,
            "gamma": self.params.gamma,
            "bloom_intensity": self.params.bloom_intensity,
            "bloom_threshold": self.params.bloom_threshold,
            "bloom_radius": self.params.bloom_radius,
            "fog_density": self.params.fog_density,
            "fog_color": self.params.fog_color,
            "saturation": self.params.saturation,
            "contrast": self.params.contrast,
            "white_balance": self.params.white_balance,
            "grade_tint": self.params.grade_tint,
            "color_grading_preset": self.params.color_grading_preset
        })
    }
}

#[derive(Debug, Clone)]
pub struct RenderSetLodCommand {
    transition_distances: [f32; 2],
    hysteresis: f32,
    previous_settings: Option<RenderSettings>,
}

impl RenderSetLodCommand {
    pub fn new(transition_distances: [f32; 2], hysteresis: f32) -> Self {
        Self {
            transition_distances,
            hysteresis,
            previous_settings: None,
        }
    }
}

impl EngineCommand for RenderSetLodCommand {
    fn name(&self) -> &'static str {
        "render.set_lod_settings"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.transition_distances[0] <= 0.0 {
            return ValidationResult::invalid("lod near transition must be > 0");
        }
        if self.transition_distances[1] <= self.transition_distances[0] {
            return ValidationResult::invalid("lod far transition must be > near transition");
        }
        if self.hysteresis < 0.0 {
            return ValidationResult::invalid("lod hysteresis must be >= 0");
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_settings = Some(ctx.render_settings.clone());
        let near = self.transition_distances[0].clamp(1.0, 5000.0);
        let far = self.transition_distances[1].clamp(near + 0.5, 5000.0);
        let hysteresis = self.hysteresis.clamp(0.0, 64.0);
        ctx.render_settings.lod_transition_distances = [near, far];
        ctx.render_settings.lod_hysteresis = hysteresis;
        Ok(CommandResult::new(
            "lod settings updated",
            json!({
                "lod_transition_distances": ctx.render_settings.lod_transition_distances,
                "lod_hysteresis": ctx.render_settings.lod_hysteresis
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_settings) = &self.previous_settings {
            ctx.render_settings = previous_settings.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "transition_distances": self.transition_distances,
            "hysteresis": self.hysteresis
        })
    }
}

#[derive(Debug, Clone)]
pub struct LowcodeSetGraphCommand {
    graph: NodeGraphFile,
    previous_state: Option<NodeGraphRuntimeState>,
}

impl LowcodeSetGraphCommand {
    pub fn new(graph: NodeGraphFile) -> Self {
        Self {
            graph,
            previous_state: None,
        }
    }
}

impl EngineCommand for LowcodeSetGraphCommand {
    fn name(&self) -> &'static str {
        "graph.set"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.graph.graph_name.trim().is_empty() {
            return ValidationResult::invalid("graph_name cannot be empty");
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        self.previous_state = Some(ctx.node_graph.clone());
        let report = validate_node_graph(&self.graph);
        ctx.node_graph.graph = Some(self.graph.clone());
        ctx.node_graph.validation = report.clone();
        ctx.node_graph.last_execution = None;
        Ok(CommandResult::new(
            "graph updated",
            json!({
                "graph_name": self.graph.graph_name,
                "node_count": self.graph.nodes.len(),
                "edge_count": self.graph.edges.len(),
                "validation": report
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_state) = &self.previous_state {
            ctx.node_graph = previous_state.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "graph_name": self.graph.graph_name,
            "nodes": self.graph.nodes,
            "edges": self.graph.edges
        })
    }
}

#[derive(Debug, Clone)]
pub struct LowcodeApplyTemplateCommand {
    template_id: String,
    previous_scene: Option<SceneFile>,
    previous_scene_runtime: Option<SceneRuntimeSettings>,
    previous_components: Option<HashMap<String, BTreeMap<String, Value>>>,
    previous_selection: Option<Vec<String>>,
    previous_graph_state: Option<NodeGraphRuntimeState>,
}

impl LowcodeApplyTemplateCommand {
    pub fn new(template_id: impl Into<String>) -> Self {
        Self {
            template_id: template_id.into(),
            previous_scene: None,
            previous_scene_runtime: None,
            previous_components: None,
            previous_selection: None,
            previous_graph_state: None,
        }
    }
}

impl EngineCommand for LowcodeApplyTemplateCommand {
    fn name(&self) -> &'static str {
        "template.apply"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.template_id.trim().is_empty() {
            return ValidationResult::invalid("template_id cannot be empty");
        }
        if builtin_template_spec(&self.template_id).is_none() {
            return ValidationResult::invalid(format!("unknown template '{}'", self.template_id));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let template = builtin_template_spec(&self.template_id)
            .with_context(|| format!("unknown template '{}'", self.template_id))?;
        self.previous_scene = Some(ctx.scene.clone());
        self.previous_scene_runtime = Some(ctx.scene_runtime.clone());
        self.previous_components = Some(ctx.components.clone());
        self.previous_selection = Some(ctx.selection.clone());
        self.previous_graph_state = Some(ctx.node_graph.clone());

        ctx.scene = template.scene.clone();
        ctx.scene_runtime = SceneRuntimeSettings::default();
        if template
            .template_id
            .eq_ignore_ascii_case("template_medieval_island")
        {
            ctx.scene_runtime.sky_preset = "sunset_hazy".to_string();
            ctx.scene_runtime.time_of_day = 18.0;
        } else if template
            .template_id
            .eq_ignore_ascii_case("template_shooter_arena")
        {
            ctx.scene_runtime.sky_preset = "clear_day".to_string();
            ctx.scene_runtime.time_of_day = 13.0;
        } else if template
            .template_id
            .eq_ignore_ascii_case("template_platform_runner")
        {
            ctx.scene_runtime.sky_preset = "clear_day".to_string();
            ctx.scene_runtime.time_of_day = 15.0;
        }
        ctx.selection.clear();
        ctx.components.clear();

        let graph_validation = validate_node_graph(&template.graph);
        let bundle_validation = validate_template_bundle(&template.asset_bundle, &ctx.project_root);
        ctx.node_graph.active_template_id = Some(template.template_id.clone());
        ctx.node_graph.graph = Some(template.graph.clone());
        ctx.node_graph.validation = graph_validation.clone();
        ctx.node_graph.last_execution = None;
        ctx.node_graph.last_bundle_validation = Some(bundle_validation.clone());

        Ok(CommandResult::new(
            format!("template '{}' applied", template.template_id),
            json!({
                "template_id": template.template_id,
                "scene_name": ctx.scene.name,
                "entity_count": ctx.scene.entities.len(),
                "graph_nodes": template.graph.nodes.len(),
                "graph_edges": template.graph.edges.len(),
                "graph_validation": graph_validation,
                "bundle_validation": bundle_validation
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(scene) = &self.previous_scene {
            ctx.scene = scene.clone();
        }
        if let Some(scene_runtime) = &self.previous_scene_runtime {
            ctx.scene_runtime = scene_runtime.clone();
        }
        if let Some(components) = &self.previous_components {
            ctx.components = components.clone();
        }
        if let Some(selection) = &self.previous_selection {
            ctx.selection = selection.clone();
        }
        if let Some(graph_state) = &self.previous_graph_state {
            ctx.node_graph = graph_state.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "template_id": self.template_id
        })
    }
}

#[derive(Debug, Clone)]
pub struct LowcodeRunGraphCommand {
    events: Vec<GraphEvent>,
    previous_scene: Option<SceneFile>,
    previous_scene_runtime: Option<SceneRuntimeSettings>,
    previous_components: Option<HashMap<String, BTreeMap<String, Value>>>,
    previous_graph_state: Option<NodeGraphRuntimeState>,
    previous_render_settings: Option<RenderSettings>,
}

impl LowcodeRunGraphCommand {
    pub fn new(events: Vec<GraphEvent>) -> Self {
        Self {
            events,
            previous_scene: None,
            previous_scene_runtime: None,
            previous_components: None,
            previous_graph_state: None,
            previous_render_settings: None,
        }
    }
}

impl EngineCommand for LowcodeRunGraphCommand {
    fn name(&self) -> &'static str {
        "graph.run"
    }

    fn validate(&self, ctx: &CommandContext) -> ValidationResult {
        if self.events.is_empty() {
            return ValidationResult::invalid("graph.run requires at least one event");
        }
        if ctx.node_graph.graph.is_none() {
            return ValidationResult::invalid("no active node graph loaded");
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let graph = ctx
            .node_graph
            .graph
            .clone()
            .context("no active node graph loaded")?;
        let mut summary = execute_runtime_graph(&graph, &self.events)
            .context("failed to execute runtime graph deterministically")?;

        self.previous_scene = Some(ctx.scene.clone());
        self.previous_scene_runtime = Some(ctx.scene_runtime.clone());
        self.previous_components = Some(ctx.components.clone());
        self.previous_graph_state = Some(ctx.node_graph.clone());
        self.previous_render_settings = Some(ctx.render_settings.clone());

        for effect in &mut summary.side_effects {
            match effect {
                GraphSideEffect::SpawnEntity {
                    entity_name,
                    mesh,
                    translation,
                } => {
                    let final_name = unique_entity_name(&ctx.scene, entity_name);
                    *entity_name = final_name.clone();
                    ctx.scene.entities.push(SceneEntity {
                        name: final_name,
                        mesh: mesh.clone(),
                        translation: *translation,
                    });
                }
                GraphSideEffect::MoveEntity {
                    entity_name,
                    translation,
                } => {
                    if let Some(entity) = ctx
                        .scene
                        .entities
                        .iter_mut()
                        .find(|entity| entity.name.eq_ignore_ascii_case(entity_name))
                    {
                        entity.translation = *translation;
                    }
                }
                GraphSideEffect::ApplyDamage {
                    entity_name,
                    amount,
                } => {
                    let bucket = ctx.components.entry(entity_name.clone()).or_default();
                    let current = bucket
                        .get("Health")
                        .and_then(|value| value.get("value"))
                        .and_then(Value::as_f64)
                        .unwrap_or(100.0);
                    let next = (current - (*amount as f64)).max(0.0);
                    bucket.insert(
                        "Health".to_string(),
                        json!({
                            "value": next,
                            "max": current.max(100.0)
                        }),
                    );
                }
                GraphSideEffect::SetLightPreset { preset } => {
                    apply_light_preset(&mut ctx.render_settings, preset);
                }
                GraphSideEffect::SetWeather { preset } => {
                    ctx.scene_runtime.sky_preset = preset.clone();
                    if preset.to_ascii_lowercase().contains("rain") {
                        ctx.scene_runtime.fog = Some(SceneFogSettings {
                            density: 0.08,
                            color: [0.52, 0.58, 0.66],
                            start: 4.0,
                            end: 80.0,
                        });
                    }
                }
                GraphSideEffect::ShowMessage { text } => {
                    ctx.scene_runtime.last_message = Some(text.clone());
                }
                GraphSideEffect::SetObjective { text } => {
                    ctx.scene_runtime.objective = Some(text.clone());
                }
            }
        }

        ctx.node_graph.last_execution = Some(summary.clone());
        Ok(CommandResult::new(
            "graph executed",
            json!({
                "events": self.events.iter().map(|event| event.as_str()).collect::<Vec<&str>>(),
                "executed_nodes": summary.executed_node_ids,
                "side_effect_count": summary.side_effects.len(),
                "summary": summary
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_scene) = &self.previous_scene {
            ctx.scene = previous_scene.clone();
        }
        if let Some(previous_runtime) = &self.previous_scene_runtime {
            ctx.scene_runtime = previous_runtime.clone();
        }
        if let Some(previous_components) = &self.previous_components {
            ctx.components = previous_components.clone();
        }
        if let Some(previous_graph_state) = &self.previous_graph_state {
            ctx.node_graph = previous_graph_state.clone();
        }
        if let Some(previous_render_settings) = &self.previous_render_settings {
            ctx.render_settings = previous_render_settings.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "events": self.events.iter().map(|event| event.as_str()).collect::<Vec<&str>>()
        })
    }
}

#[derive(Debug, Clone)]
pub struct LowcodeValidateTemplateBundleCommand {
    template_id: String,
    previous_report: Option<Option<TemplateBundleValidationReport>>,
}

impl LowcodeValidateTemplateBundleCommand {
    pub fn new(template_id: impl Into<String>) -> Self {
        Self {
            template_id: template_id.into(),
            previous_report: None,
        }
    }
}

impl EngineCommand for LowcodeValidateTemplateBundleCommand {
    fn name(&self) -> &'static str {
        "asset.validate_template_bundle"
    }

    fn validate(&self, _ctx: &CommandContext) -> ValidationResult {
        if self.template_id.trim().is_empty() {
            return ValidationResult::invalid("template_id cannot be empty");
        }
        if builtin_template_spec(&self.template_id).is_none() {
            return ValidationResult::invalid(format!("unknown template '{}'", self.template_id));
        }
        ValidationResult::ok()
    }

    fn execute(&mut self, ctx: &mut CommandContext) -> anyhow::Result<CommandResult> {
        let template = builtin_template_spec(&self.template_id)
            .with_context(|| format!("unknown template '{}'", self.template_id))?;
        self.previous_report = Some(ctx.node_graph.last_bundle_validation.clone());
        let report = validate_template_bundle(&template.asset_bundle, &ctx.project_root);
        ctx.node_graph.last_bundle_validation = Some(report.clone());
        Ok(CommandResult::new(
            "template bundle validated",
            json!({
                "template_id": template.template_id,
                "report": report
            }),
        ))
    }

    fn undo(&mut self, ctx: &mut CommandContext) -> anyhow::Result<()> {
        if let Some(previous_report) = &self.previous_report {
            ctx.node_graph.last_bundle_validation = previous_report.clone();
        }
        Ok(())
    }

    fn serialize(&self) -> Value {
        json!({
            "template_id": self.template_id
        })
    }
}

fn normalize_direction(direction: [f32; 3]) -> [f32; 3] {
    let len_sq =
        direction[0] * direction[0] + direction[1] * direction[1] + direction[2] * direction[2];
    if len_sq <= 1e-6 {
        return [-0.5, -1.0, -0.3];
    }
    let inv_len = len_sq.sqrt().recip();
    [
        direction[0] * inv_len,
        direction[1] * inv_len,
        direction[2] * inv_len,
    ]
}

fn normalize_collider_shape(shape: impl Into<String>) -> String {
    match shape.into().trim().to_ascii_lowercase().as_str() {
        "sphere" => "sphere".to_string(),
        "capsule" => "capsule".to_string(),
        "mesh" => "mesh".to_string(),
        _ => "box".to_string(),
    }
}

fn normalize_body_type(body_type: impl Into<String>) -> String {
    match body_type.into().trim().to_ascii_lowercase().as_str() {
        "static" => "static".to_string(),
        "kinematic" => "kinematic".to_string(),
        _ => "dynamic".to_string(),
    }
}

fn normalize_character_state(state: impl Into<String>) -> String {
    match state.into().trim().to_ascii_lowercase().as_str() {
        "moving" => "moving".to_string(),
        "jumping" => "jumping".to_string(),
        "falling" => "falling".to_string(),
        "mounted" => "mounted".to_string(),
        _ => "idle".to_string(),
    }
}

fn normalize_horse_gait(gait: impl Into<String>) -> String {
    match gait.into().trim().to_ascii_lowercase().as_str() {
        "walk" => "walk".to_string(),
        "trot" => "trot".to_string(),
        "gallop" => "gallop".to_string(),
        _ => "walk".to_string(),
    }
}

fn normalize_ui_template(template_type: impl Into<String>) -> String {
    match template_type.into().trim().to_ascii_lowercase().as_str() {
        "platformer" => "platformer".to_string(),
        _ => "shooter".to_string(),
    }
}

fn normalize_prediction_mode(mode: impl Into<String>) -> String {
    match mode.into().trim().to_ascii_lowercase().as_str() {
        "client" | "client_authoritative" => "client".to_string(),
        "hybrid" => "hybrid".to_string(),
        _ => "server".to_string(),
    }
}

fn normalize_build_target(target: impl Into<String>) -> String {
    match target.into().trim().to_ascii_lowercase().as_str() {
        "win" | "windows" => "windows".to_string(),
        "linux" => "linux".to_string(),
        "mac" | "macos" => "macos".to_string(),
        "android" => "android".to_string(),
        "ios" => "ios".to_string(),
        "web" | "webgl" | "wasm" => "web".to_string(),
        other if !other.is_empty() => other.to_string(),
        _ => "windows".to_string(),
    }
}

fn normalize_feature_flag(flag: impl Into<String>) -> String {
    normalize_identifier(flag.into())
}

fn create_debug_profiler_snapshot(ctx: &CommandContext) -> DebugProfilerSnapshot {
    let entity_count = ctx.scene.entities.len();
    let collider_count = ctx.physics.colliders.len();
    let draw_call_estimate = entity_count.max(1);
    let mut notes = Vec::<String>::new();
    if entity_count > 500 {
        notes.push("high_entity_count".to_string());
    }
    if collider_count > 300 {
        notes.push("high_collider_count".to_string());
    }
    if ctx.engine_state.fps > 0.0 && ctx.engine_state.fps < 45.0 {
        notes.push("fps_below_45".to_string());
    }
    DebugProfilerSnapshot {
        timestamp_utc: chrono::Utc::now().to_rfc3339(),
        fps: ctx.engine_state.fps,
        entity_count,
        collider_count,
        draw_call_estimate,
        notes,
    }
}

fn normalize_identifier(input: impl AsRef<str>) -> String {
    let mut out = String::new();
    for ch in input.as_ref().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_ascii_whitespace() {
            out.push('_');
        }
    }
    let out = out.trim_matches('_').to_string();
    if out.is_empty() {
        "item".to_string()
    } else {
        out
    }
}

fn normalize_model_primitive_type(value: impl Into<String>) -> String {
    match value.into().trim().to_ascii_lowercase().as_str() {
        "cube" | "box" => "cube".to_string(),
        "sphere" | "uv_sphere" => "sphere".to_string(),
        "cylinder" => "cylinder".to_string(),
        "capsule" => "capsule".to_string(),
        "plane" => "plane".to_string(),
        "cone" => "cone".to_string(),
        _ => "cube".to_string(),
    }
}

fn primitive_mesh_stats(primitive_type: &str) -> (u32, u32) {
    match primitive_type {
        "sphere" => (162, 320),
        "cylinder" => (96, 160),
        "capsule" => (144, 256),
        "plane" => (4, 2),
        "cone" => (64, 96),
        _ => (24, 12),
    }
}

fn parse_optional_vec3(value: Option<&Value>) -> Option<[f32; 3]> {
    let value = value?;
    let arr = value.as_array()?;
    if arr.len() != 3 {
        return None;
    }
    let mut out = [0.0f32; 3];
    for (idx, item) in arr.iter().enumerate() {
        out[idx] = item.as_f64()? as f32;
    }
    Some(out)
}

fn parse_component_vec3(value: Option<&Value>, default: [f32; 3]) -> [f32; 3] {
    if let Some(parsed) = parse_optional_vec3(value) {
        return parsed;
    }
    default
}

fn extract_parent_id_from_component(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if let Some(parent_id) = value.as_str() {
        let parent_id = parent_id.trim();
        if !parent_id.is_empty() {
            return Some(parent_id.to_string());
        }
    }
    let obj = value.as_object()?;
    obj.get("parent_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|parent_id| !parent_id.is_empty())
        .map(str::to_string)
        .or_else(|| {
            obj.get("entity_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|parent_id| !parent_id.is_empty())
                .map(str::to_string)
        })
}

fn get_entity_parent_id(ctx: &CommandContext, entity_name: &str) -> Option<String> {
    ctx.components
        .get(entity_name)
        .and_then(|bucket| bucket.get("HierarchyParent"))
        .and_then(|value| extract_parent_id_from_component(Some(value)))
}

fn parent_would_create_cycle(ctx: &CommandContext, child_name: &str, parent_name: &str) -> bool {
    let mut seen = BTreeSet::<String>::new();
    let mut cursor = Some(parent_name.to_string());
    while let Some(current) = cursor {
        if current == child_name {
            return true;
        }
        if !seen.insert(current.clone()) {
            return true;
        }
        cursor = get_entity_parent_id(ctx, &current);
    }
    false
}

fn clear_parent_links_to_entity(ctx: &mut CommandContext, parent_name: &str) {
    for bucket in ctx.components.values_mut() {
        let should_clear = extract_parent_id_from_component(bucket.get("HierarchyParent"))
            .map(|candidate| candidate == parent_name)
            .unwrap_or(false);
        if should_clear {
            bucket.remove("HierarchyParent");
        }
    }
    ctx.components.retain(|_, bucket| !bucket.is_empty());
}

fn remap_hash_map_key<T>(map: &mut HashMap<String, T>, from: &str, to: &str) {
    if let Some(value) = map.remove(from) {
        map.insert(to.to_string(), value);
    }
}

fn remap_entity_references(ctx: &mut CommandContext, from: &str, to: &str) {
    for selected in &mut ctx.selection {
        if selected == from {
            *selected = to.to_string();
        }
    }

    remap_hash_map_key(&mut ctx.components, from, to);
    remap_hash_map_key(
        &mut ctx.scene_runtime.world_streaming.entity_to_chunk,
        from,
        to,
    );
    remap_hash_map_key(&mut ctx.physics.colliders, from, to);
    remap_hash_map_key(&mut ctx.physics.rigidbodies, from, to);
    remap_hash_map_key(&mut ctx.physics.character_controllers, from, to);
    remap_hash_map_key(&mut ctx.gameplay.attachments, from, to);
    remap_hash_map_key(&mut ctx.gameplay.triggers, from, to);
    remap_hash_map_key(&mut ctx.gameplay.pickups, from, to);
    remap_hash_map_key(&mut ctx.gameplay.inventories, from, to);
    remap_hash_map_key(&mut ctx.gameplay.interactables, from, to);
    remap_hash_map_key(&mut ctx.animation.entity_animators, from, to);
    remap_hash_map_key(&mut ctx.animation.entity_active_clips, from, to);
    remap_hash_map_key(&mut ctx.animation.entity_blends, from, to);
    remap_hash_map_key(&mut ctx.animation.ik_solvers, from, to);
    remap_hash_map_key(&mut ctx.water.buoyancy, from, to);
    remap_hash_map_key(&mut ctx.water.drag, from, to);
    remap_hash_map_key(&mut ctx.mount.rider_to_horse, from, to);
    remap_hash_map_key(&mut ctx.npc_ai.entity_agents, from, to);
    remap_hash_map_key(&mut ctx.npc_ai.blackboard, from, to);
    remap_hash_map_key(&mut ctx.networking.replication, from, to);

    for bake_job in &mut ctx.animation.bake_jobs {
        if bake_job.entity_id == from {
            bake_job.entity_id = to.to_string();
        }
    }
    for particle in ctx.vfx.particle_systems.values_mut() {
        if let Some(attached_entity) = &particle.attached_entity
            && attached_entity == from
        {
            particle.attached_entity = Some(to.to_string());
        }
    }
    for horse in ctx.mount.horses.values_mut() {
        if horse.entity_id == from {
            horse.entity_id = to.to_string();
        }
        if let Some(rider_id) = &horse.rider_id
            && rider_id == from
        {
            horse.rider_id = Some(to.to_string());
        }
    }
    for agent in ctx.npc_ai.agents.values_mut() {
        if agent.entity_id == from {
            agent.entity_id = to.to_string();
        }
    }
    for binding in ctx.ui.bindings.values_mut() {
        if binding.entity_id == from {
            binding.entity_id = to.to_string();
        }
    }
    for source in ctx.audio.sources.values_mut() {
        if let Some(entity_id) = &source.entity_id
            && entity_id == from
        {
            source.entity_id = Some(to.to_string());
        }
    }
    for bucket in ctx.components.values_mut() {
        if let Some(parent_component) = bucket.get_mut("HierarchyParent") {
            let current_parent = extract_parent_id_from_component(Some(parent_component));
            if current_parent.as_deref() == Some(from) {
                *parent_component = json!({
                    "parent_id": to
                });
            }
        }
    }
}

fn parse_vec3_array(value: &Value) -> anyhow::Result<Vec<[f32; 3]>> {
    let arr = value
        .as_array()
        .with_context(|| "value must be an array of [x,y,z] points")?;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, point) in arr.iter().enumerate() {
        let point_arr = point
            .as_array()
            .with_context(|| format!("point {} must be [x,y,z]", idx))?;
        if point_arr.len() != 3 {
            bail!("point {} must contain exactly 3 values", idx);
        }
        let mut vec = [0.0f32; 3];
        for (axis, raw) in point_arr.iter().enumerate() {
            vec[axis] = raw
                .as_f64()
                .map(|v| v as f32)
                .with_context(|| format!("point {} axis {} must be numeric", idx, axis))?;
        }
        out.push(vec);
    }
    Ok(out)
}

fn model_mesh_exists(ctx: &CommandContext, mesh_id: &str) -> bool {
    if ctx.modeling.meshes.contains_key(mesh_id) {
        return true;
    }
    ctx.scene
        .entities
        .iter()
        .any(|entity| entity.mesh.eq_ignore_ascii_case(mesh_id))
}

fn ensure_model_mesh_record(ctx: &mut CommandContext, mesh_id: &str) -> anyhow::Result<()> {
    if ctx.modeling.meshes.contains_key(mesh_id) {
        return Ok(());
    }
    if ctx
        .scene
        .entities
        .iter()
        .any(|entity| entity.mesh.eq_ignore_ascii_case(mesh_id))
    {
        ctx.modeling.meshes.insert(
            mesh_id.to_string(),
            ModelMeshRecord {
                mesh_id: mesh_id.to_string(),
                primitive_type: None,
                vertex_count: 0,
                face_count: 0,
            },
        );
        return Ok(());
    }
    bail!("mesh '{}' not found in scene/modeling state", mesh_id);
}

fn apply_model_topology_operation(mesh: &mut ModelMeshRecord, op: &str, params: &Value) {
    match op {
        "extrude" => {
            mesh.vertex_count = mesh.vertex_count.saturating_add(16);
            mesh.face_count = mesh.face_count.saturating_add(8);
        }
        "inset" => {
            mesh.vertex_count = mesh.vertex_count.saturating_add(8);
            mesh.face_count = mesh.face_count.saturating_add(4);
        }
        "bevel" => {
            mesh.vertex_count = mesh.vertex_count.saturating_add(12);
            mesh.face_count = mesh.face_count.saturating_add(6);
        }
        "loop_cut" => {
            mesh.vertex_count = mesh.vertex_count.saturating_add(20);
            mesh.face_count = mesh.face_count.saturating_add(10);
        }
        "knife" => {
            mesh.vertex_count = mesh.vertex_count.saturating_add(10);
            mesh.face_count = mesh.face_count.saturating_add(10);
        }
        "merge" => {
            mesh.vertex_count = mesh.vertex_count.saturating_sub(4);
            mesh.face_count = mesh.face_count.saturating_sub(2);
        }
        "subdivide" => {
            mesh.vertex_count = mesh.vertex_count.saturating_mul(2).max(8);
            mesh.face_count = mesh.face_count.saturating_mul(2).max(4);
        }
        "triangulate" => {
            mesh.face_count = mesh.face_count.saturating_mul(2).max(2);
        }
        "voxel_remesh" => {
            let resolution = params
                .get("resolution")
                .and_then(Value::as_u64)
                .unwrap_or(32)
                .min(1024) as u32;
            mesh.vertex_count = resolution.saturating_mul(6).max(24);
            mesh.face_count = resolution.saturating_mul(12).max(12);
        }
        "decimate" => {
            let ratio = params
                .get("ratio")
                .and_then(Value::as_f64)
                .map(|value| value as f32)
                .unwrap_or(0.5)
                .clamp(0.01, 1.0);
            mesh.vertex_count = ((mesh.vertex_count as f32) * ratio).round().max(4.0) as u32;
            mesh.face_count = ((mesh.face_count as f32) * ratio).round().max(2.0) as u32;
        }
        "smooth" => {}
        _ => {}
    }
}

fn unique_entity_name(scene: &SceneFile, desired: &str) -> String {
    let base = desired.trim();
    let base = if base.is_empty() { "Entity" } else { base };
    if !scene
        .entities
        .iter()
        .any(|entity| entity.name.eq_ignore_ascii_case(base))
    {
        return base.to_string();
    }
    for index in 1..=2048 {
        let candidate = format!("{}_{}", base, index);
        if !scene
            .entities
            .iter()
            .any(|entity| entity.name.eq_ignore_ascii_case(&candidate))
        {
            return candidate;
        }
    }
    format!("{}_{}", base, chrono::Utc::now().timestamp_millis())
}

fn apply_light_preset(render_settings: &mut RenderSettings, preset: &str) {
    match preset.trim().to_ascii_lowercase().as_str() {
        "golden_hour" | "filmic_sunset" => {
            render_settings.light_color = [1.0, 0.72, 0.45];
            render_settings.light_intensity = 6.2;
            render_settings.exposure = 1.08;
            render_settings.color_grading_preset = "filmic_sunset".to_string();
        }
        "noir" | "noir_indoor" => {
            render_settings.light_color = [0.72, 0.8, 1.0];
            render_settings.light_intensity = 3.1;
            render_settings.exposure = 0.75;
            render_settings.color_grading_preset = "noir_indoor".to_string();
        }
        _ => {
            render_settings.light_color = [1.0, 1.0, 1.0];
            render_settings.light_intensity = 5.2;
            render_settings.exposure = 1.0;
            render_settings.color_grading_preset = "natural_day".to_string();
        }
    }
}

fn sanitize_file_stem(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else if ch.is_ascii_whitespace() {
            out.push('_');
        }
    }
    if out.trim_matches('_').is_empty() {
        "material".to_string()
    } else {
        out.trim_matches('_').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_bus_syncs_ecs_world_after_entity_commands() {
        let context = CommandContext::new(".");
        let mut bus = CommandBus::new(context);

        bus.submit(Box::new(SceneCreateCommand::new("Ecs Sync")))
            .expect("scene.create should succeed");
        bus.submit(Box::new(EntityCreateCommand::new(
            "CrateA",
            "cube",
            [1.0, 2.0, 3.0],
        )))
        .expect("entity.create should succeed");
        bus.submit(Box::new(EntitySetTransformCommand::new(
            "CrateA",
            [4.0, 5.0, 6.0],
        )))
        .expect("entity.set_transform should succeed");
        bus.submit(Box::new(EntityAddComponentCommand::new(
            "CrateA",
            "Health",
            json!({"value": 100}),
        )))
        .expect("entity.add_component should succeed");

        assert_eq!(bus.context().ecs_entity_count(), 1);
        assert_eq!(
            bus.context().entity_transform("CrateA"),
            Some([4.0, 5.0, 6.0])
        );
        assert!(
            bus.context()
                .runtime_world
                .get_dynamic_components("CrateA")
                .expect("dynamic components should exist")
                .contains_key("Health")
        );
    }

    #[test]
    fn asset_instantiate_prefab_adds_and_undoes_entity() {
        let mut context = CommandContext::new(".");
        context.imported_assets.insert(
            "assets/imported/crate.glb".to_string(),
            ImportedAssetRecord {
                asset_id: "assets/imported/crate.glb".to_string(),
                source_path: "C:/tmp/crate.glb".to_string(),
                imported_path: "./assets/imported/crate.glb".to_string(),
                kind: "glb".to_string(),
            },
        );
        let mut bus = CommandBus::new(context);

        bus.submit(Box::new(SceneCreateCommand::new("Prefab Scene")))
            .expect("scene.create should succeed");
        bus.submit(Box::new(AssetInstantiatePrefabCommand::new(
            "assets/imported/crate.glb",
            "CratePrefab",
            [0.0, 1.0, 2.0],
        )))
        .expect("asset.instantiate_prefab should succeed");

        assert!(bus.context().entity_exists("CratePrefab"));
        assert_eq!(
            bus.context().entity_transform("CratePrefab"),
            Some([0.0, 1.0, 2.0])
        );

        let undone = bus.history_undo(1).expect("undo should succeed");
        assert_eq!(undone, 1);
        assert!(!bus.context().entity_exists("CratePrefab"));
    }

    #[test]
    fn lowcode_template_apply_and_graph_run_changes_scene_state() {
        let context = CommandContext::new(".");
        let mut bus = CommandBus::new(context);
        bus.submit(Box::new(LowcodeApplyTemplateCommand::new(
            "template_shooter_arena",
        )))
        .expect("template.apply should succeed");
        let before_count = bus.context().scene.entities.len();
        assert!(before_count >= 3);
        assert!(bus.context().node_graph.graph.is_some());

        bus.submit(Box::new(LowcodeRunGraphCommand::new(vec![
            GraphEvent::OnStart,
        ])))
        .expect("graph.run should succeed");
        let after_count = bus.context().scene.entities.len();
        assert!(after_count > before_count);
        assert_eq!(
            bus.context().scene_runtime.objective.as_deref(),
            Some("Defeat enemy wave")
        );
        assert!(bus.context().node_graph.last_execution.is_some());
    }
}
