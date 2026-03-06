use crate::command_bus::{
    AnimMutationCommand, AssetCreateMaterialCommand, AssetImportFileCommand,
    AssetInstantiatePrefabCommand, AssetPipelineMutationCommand, AudioMutationCommand,
    BuildMutationCommand, CommandBus, CommandContext, DebugMutationCommand,
    EntityAddComponentCommand, EntityCloneCommand, EntityCreateCommand, EntityDeleteCommand,
    EntityParentCommand, EntityRemoveComponentCommand, EntityRenameCommand, EntityRotateCommand,
    EntityScaleCommand, EntitySetComponentCommand, EntitySetTransformCommand,
    EntityTranslateCommand, EntityUnparentCommand, GameAddHealthComponentCommand,
    GameAddInteractableCommand, GameAddInventoryCommand, GameAddPickupCommand,
    GameAddTriggerCommand, GameApplyDamageCommand, GameAttachWeaponCommand, GameBindActionCommand,
    GameCreateInputActionCommand, GameCreateWeaponCommand, GameFireWeaponCommand,
    GameSetRebindCommand, LowcodeApplyTemplateCommand, LowcodeRunGraphCommand,
    LowcodeSetGraphCommand, LowcodeValidateTemplateBundleCommand, ModelMutationCommand,
    MountMutationCommand, NetMutationCommand, NodeGraphRuntimeState, NpcAiMutationCommand,
    PhysAddCharacterControllerCommand, PhysAddColliderCommand, PhysAddRigidbodyCommand,
    PhysApplyForceCommand, PhysApplyImpulseCommand, PhysCharacterJumpCommand,
    PhysCharacterMoveCommand, PhysCharacterSetStateCommand, PhysRemoveColliderCommand,
    PhysSetGravityCommand, PhysSetRigidbodyParamsCommand, RenderPostprocessParams,
    RenderSetIblCommand, RenderSetLightCommand, RenderSetLodCommand, RenderSetPostprocessCommand,
    RenderSettings, SceneAddFogCommand, SceneAssignEntityToChunkCommand, SceneCloseCommand,
    SceneCreateCommand, SceneCreateStreamChunkCommand, SceneDuplicateCommand,
    SceneEnableWorldStreamingCommand, SceneOpenCommand, SceneSaveCommand, SceneSetSkyCommand,
    SceneSetTimeOfDayCommand, UiMutationCommand, VfxMutationCommand, WaterMutationCommand,
    resolve_project_path,
};
use anyhow::{Context, bail};
use assets::{
    NodeGraphEdge, NodeGraphFile, NodeGraphNode, SceneFile, builtin_template_bundle,
    builtin_template_specs, supported_node_type, validate_node_graph,
};
use ecs::GraphEvent;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ToolPermission {
    Read,
    Write,
    Export,
    Network,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub permissions: Vec<ToolPermission>,
    pub params_schema: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskState {
    pub id: String,
    pub title: String,
    pub progress: f32,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticEntry {
    pub level: String,
    pub message: String,
    pub source: String,
    pub timestamp_utc: String,
}

pub struct ToolRuntime {
    command_bus: CommandBus,
    schemas: HashMap<String, ToolSchema>,
    tasks: HashMap<String, TaskState>,
    next_task_id: u64,
    project_memory: Value,
    runtime_constraints: Value,
    diagnostics: Vec<DiagnosticEntry>,
    max_diagnostics: usize,
}

impl ToolRuntime {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        let context = CommandContext::new(project_root);
        let mut runtime = Self {
            command_bus: CommandBus::new(context),
            schemas: HashMap::new(),
            tasks: HashMap::new(),
            next_task_id: 1,
            project_memory: json!({
                "style": null,
                "target_platform": null,
                "target_fps": null,
                "notes": [],
                "tags": []
            }),
            runtime_constraints: json!({
                "target_fps": null,
                "resolution": null,
                "allow_external_assets": true,
                "max_gpu_memory_mb": null,
                "max_system_memory_mb": null,
                "notes": []
            }),
            diagnostics: Vec::new(),
            max_diagnostics: 256,
        };
        runtime.register_default_tools();
        runtime
    }

    pub fn set_ai_mode(&mut self, mode: &str) {
        self.command_bus.set_ai_mode(mode);
    }

    pub fn set_frame_stats(&mut self, fps: f32) {
        self.command_bus.set_frame_stats(fps);
    }

    pub fn command_bus(&self) -> &CommandBus {
        &self.command_bus
    }

    pub fn command_bus_mut(&mut self) -> &mut CommandBus {
        &mut self.command_bus
    }

    pub fn scene_snapshot(&self) -> SceneFile {
        self.command_bus.scene_snapshot()
    }

    pub fn scene_revision(&self) -> u64 {
        self.command_bus.scene_revision()
    }

    pub fn render_settings(&self) -> RenderSettings {
        self.command_bus.render_settings()
    }

    pub fn lowcode_state(&self) -> NodeGraphRuntimeState {
        self.command_bus.context().node_graph.clone()
    }

    pub fn sync_scene_from_editor(
        &mut self,
        scene: SceneFile,
        open_scene_path: Option<PathBuf>,
    ) -> anyhow::Result<()> {
        self.command_bus
            .replace_scene_from_editor(scene, open_scene_path)
    }

    pub fn list_tools(&self) -> Vec<ToolSchema> {
        let mut tools = self.schemas.values().cloned().collect::<Vec<ToolSchema>>();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        tools
    }

    fn push_diagnostic(
        &mut self,
        level: impl Into<String>,
        message: impl Into<String>,
        source: impl Into<String>,
    ) {
        self.diagnostics.push(DiagnosticEntry {
            level: level.into(),
            message: message.into(),
            source: source.into(),
            timestamp_utc: chrono::Utc::now().to_rfc3339(),
        });
        if self.diagnostics.len() > self.max_diagnostics {
            let overflow = self.diagnostics.len() - self.max_diagnostics;
            self.diagnostics.drain(0..overflow);
        }
    }

    fn diagnostics_tail(&self, last_n: usize, level: Option<&str>) -> Vec<DiagnosticEntry> {
        let requested = last_n.max(1);
        let normalized_level = level
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_lowercase());
        let mut out = self
            .diagnostics
            .iter()
            .filter(|entry| {
                normalized_level
                    .as_ref()
                    .map(|needle| entry.level.eq_ignore_ascii_case(needle))
                    .unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<DiagnosticEntry>>();
        if out.len() > requested {
            out.drain(0..(out.len() - requested));
        }
        out
    }

    pub fn execute(&mut self, tool_name: &str, params: Value) -> anyhow::Result<Value> {
        let result = match tool_name {
            "tool.get_engine_state" => self.tool_get_engine_state(),
            "tool.get_cycle_context" => self.tool_get_cycle_context(&params),
            "tool.get_project_tree" => self.tool_get_project_tree(&params),
            "tool.search_assets" => self.tool_search_assets(&params),
            "tool.read_asset_metadata" => self.tool_read_asset_metadata(&params),
            "tool.get_selection" => self.tool_get_selection(),
            "tool.set_selection" => self.tool_set_selection(&params),
            "tool.get_viewport_camera" => self.tool_get_viewport_camera(),
            "tool.set_viewport_camera" => self.tool_set_viewport_camera(&params),
            "tool.get_rules" => self.tool_get_rules(),
            "tool.get_project_memory" => self.tool_get_project_memory(),
            "tool.set_project_memory" => self.tool_set_project_memory(&params),
            "tool.get_constraints" => self.tool_get_constraints(),
            "tool.set_constraints" => self.tool_set_constraints(&params),
            "tool.set_objective" => self.tool_set_objective(&params),
            "tool.get_diagnostics" => self.tool_get_diagnostics(&params),
            "tool.clear_diagnostics" => self.tool_clear_diagnostics(&params),
            "tool.begin_transaction" => self.tool_begin_transaction(&params),
            "tool.commit_transaction" => self.tool_commit_transaction(),
            "tool.rollback_transaction" => self.tool_rollback_transaction(),
            "tool.create_checkpoint" => self.tool_create_checkpoint(&params),
            "tool.rollback_to_checkpoint" => self.tool_rollback_to_checkpoint(&params),
            "tool.log" => self.tool_log(&params),
            "tool.open_task" => self.tool_open_task(&params),
            "tool.update_task" => self.tool_update_task(&params),
            "tool.close_task" => self.tool_close_task(&params),
            "scene.create" => self.scene_create(&params),
            "scene.open" => self.scene_open(&params),
            "scene.save" => self.scene_save(),
            "scene.save_as" => self.scene_save_as(&params),
            "scene.duplicate" => self.scene_duplicate(&params),
            "scene.close" => self.scene_close(),
            "scene.set_sky" => self.scene_set_sky(&params),
            "scene.set_time_of_day" => self.scene_set_time_of_day(&params),
            "scene.add_fog" => self.scene_add_fog(&params),
            "scene.enable_world_streaming" => self.scene_enable_world_streaming(&params),
            "scene.create_stream_chunk" => self.scene_create_stream_chunk(&params),
            "scene.assign_entity_to_chunk" => self.scene_assign_entity_to_chunk(&params),
            "scene.add_postprocess" => self.render_set_postprocess(&params),
            "entity.create" => self.entity_create(&params),
            "entity.clone" => self.entity_clone(&params),
            "entity.delete" => self.entity_delete(&params),
            "entity.rename" => self.entity_rename(&params),
            "entity.parent" => self.entity_parent(&params),
            "entity.unparent" => self.entity_unparent(&params),
            "entity.find_by_name" => self.entity_find_by_name(&params),
            "entity.find_by_tag" => self.entity_find_by_tag(&params),
            "entity.set_transform" => self.entity_set_transform(&params),
            "entity.translate" => self.entity_translate(&params),
            "entity.rotate" => self.entity_rotate(&params),
            "entity.scale" => self.entity_scale(&params),
            "entity.get_transform" => self.entity_get_transform(&params),
            "entity.add_component" => self.entity_add_component(&params),
            "entity.remove_component" => self.entity_remove_component(&params),
            "entity.get_component" => self.entity_get_component(&params),
            "entity.set_component" => self.entity_set_component(&params),
            "asset.import_file" => self.asset_import_file(&params),
            "asset.import_url" => self.asset_import_url(&params),
            "asset.create_material" => self.asset_create_material(&params),
            "asset.create_texture" => self.asset_create_texture(&params),
            "asset.create_shader" => self.asset_create_shader(&params),
            "asset.create_prefab" => self.asset_create_prefab(&params),
            "asset.save_prefab" => self.asset_save_prefab(&params),
            "asset.rebuild_import" => self.asset_rebuild_import(&params),
            "asset.generate_lods" => self.asset_generate_lods(&params),
            "asset.mesh_optimize" => self.asset_mesh_optimize(&params),
            "asset.compress_textures" => self.asset_compress_textures(&params),
            "asset.bake_lightmaps" => self.asset_bake_lightmaps(&params),
            "asset.bake_reflection_probes" => self.asset_bake_reflection_probes(&params),
            "asset.assign_material" => self.asset_assign_material(&params),
            "asset.instantiate_prefab" => self.asset_instantiate_prefab(&params),
            "render.create_light" => self.render_create_light(&params),
            "render.set_light_params" => self.render_set_light_params(&params),
            "render.set_ibl" => self.render_set_ibl(&params),
            "render.set_postprocess" => self.render_set_postprocess(&params),
            "render.set_lod_settings" => self.render_set_lod_settings(&params),
            "render.assign_material" => self.render_assign_material(&params),
            "graph.create" => self.graph_create(&params),
            "graph.add_node" => self.graph_add_node(&params),
            "graph.connect" => self.graph_connect(&params),
            "graph.delete_node" => self.graph_delete_node(&params),
            "graph.delete_edge" => self.graph_delete_edge(&params),
            "graph.set_node_params" => self.graph_set_node_params(&params),
            "graph.validate" => self.graph_validate(),
            "graph.run" => self.graph_run(&params),
            "template.list" => self.template_list(),
            "template.apply" => self.template_apply(&params),
            "asset.get_template_bundle" => self.asset_get_template_bundle(&params),
            "asset.validate_template_bundle" => self.asset_validate_template_bundle(&params),
            "phys.add_collider" => self.phys_add_collider(&params),
            "phys.set_collider" => self.phys_set_collider(&params),
            "phys.remove_collider" => self.phys_remove_collider(&params),
            "phys.add_rigidbody" => self.phys_add_rigidbody(&params),
            "phys.set_mass" => self.phys_set_mass(&params),
            "phys.set_friction" => self.phys_set_friction(&params),
            "phys.set_restitution" => self.phys_set_restitution(&params),
            "phys.apply_force" => self.phys_apply_force(&params),
            "phys.apply_impulse" => self.phys_apply_impulse(&params),
            "phys.set_gravity" => self.phys_set_gravity(&params),
            "phys.raycast" => self.phys_raycast(&params),
            "phys.overlap" => self.phys_overlap(&params),
            "phys.add_character_controller" => self.phys_add_character_controller(&params),
            "phys.character_move" => self.phys_character_move(&params),
            "phys.character_jump" => self.phys_character_jump(&params),
            "phys.character_set_state" => self.phys_character_set_state(&params),
            "game.create_input_action" => self.game_create_input_action(&params),
            "game.bind_action" => self.game_bind_action(&params),
            "game.set_rebind" => self.game_set_rebind(&params),
            "game.create_weapon" => self.game_create_weapon(&params),
            "game.attach_weapon" => self.game_attach_weapon(&params),
            "game.fire_weapon" => self.game_fire_weapon(&params),
            "game.apply_damage" => self.game_apply_damage(&params),
            "game.add_health_component" => self.game_add_health_component(&params),
            "game.add_trigger" => self.game_add_trigger(&params),
            "game.add_pickup" => self.game_add_pickup(&params),
            "game.add_inventory" => self.game_add_inventory(&params),
            "game.add_interactable" => self.game_add_interactable(&params),
            "anim.add_animator" => self.anim_add_animator(&params),
            "anim.create_state_machine" => self.anim_create_state_machine(&params),
            "anim.add_state" => self.anim_add_state(&params),
            "anim.add_transition" => self.anim_add_transition(&params),
            "anim.set_parameter" => self.anim_set_parameter(&params),
            "anim.play" => self.anim_play(&params),
            "anim.blend" => self.anim_blend(&params),
            "anim.add_ik" => self.anim_add_ik(&params),
            "anim.retarget" => self.anim_retarget(&params),
            "anim.bake_animation" => self.anim_bake_animation(&params),
            "model.create_primitive" => self.model_create_primitive(&params),
            "model.enter_edit_mode" => self.model_enter_edit_mode(&params),
            "model.exit_edit_mode" => self.model_exit_edit_mode(&params),
            "model.select" => self.model_select(&params),
            "model.extrude" => self.model_extrude(&params),
            "model.inset" => self.model_inset(&params),
            "model.bevel" => self.model_bevel(&params),
            "model.loop_cut" => self.model_loop_cut(&params),
            "model.knife" => self.model_knife(&params),
            "model.merge" => self.model_merge(&params),
            "model.subdivide" => self.model_subdivide(&params),
            "model.triangulate" => self.model_triangulate(&params),
            "model.add_modifier" => self.model_add_modifier(&params),
            "model.set_modifier" => self.model_set_modifier(&params),
            "model.apply_modifier" => self.model_apply_modifier(&params),
            "model.remove_modifier" => self.model_remove_modifier(&params),
            "model.unwrap_uv" => self.model_unwrap_uv(&params),
            "model.pack_uv" => self.model_pack_uv(&params),
            "model.generate_lightmap_uv" => self.model_generate_lightmap_uv(&params),
            "model.voxel_remesh" => self.model_voxel_remesh(&params),
            "model.decimate" => self.model_decimate(&params),
            "model.smooth" => self.model_smooth(&params),
            "model.sculpt_brush" => self.model_sculpt_brush(&params),
            "model.sculpt_mask" => self.model_sculpt_mask(&params),
            "vfx.create_particle_system" => self.vfx_create_particle_system(&params),
            "vfx.set_emitter" => self.vfx_set_emitter(&params),
            "vfx.set_forces" => self.vfx_set_forces(&params),
            "vfx.set_collision" => self.vfx_set_collision(&params),
            "vfx.set_renderer" => self.vfx_set_renderer(&params),
            "vfx.attach_to_entity" => self.vfx_attach_to_entity(&params),
            "vfx.create_graph" => self.vfx_create_graph(&params),
            "vfx.add_node" => self.vfx_add_node(&params),
            "vfx.connect" => self.vfx_connect(&params),
            "vfx.compile_graph" => self.vfx_compile_graph(&params),
            "water.create_ocean" => self.water_create_ocean(&params),
            "water.create_river" => self.water_create_river(&params),
            "water.create_waterfall" => self.water_create_waterfall(&params),
            "water.set_waves" => self.water_set_waves(&params),
            "water.enable_foam" => self.water_enable_foam(&params),
            "water.enable_refraction" => self.water_enable_refraction(&params),
            "water.enable_caustics" => self.water_enable_caustics(&params),
            "water.add_buoyancy" => self.water_add_buoyancy(&params),
            "water.add_drag" => self.water_add_drag(&params),
            "water.sample_height" => self.water_sample_height(&params),
            "mount.create_horse_template" => self.mount_create_horse_template(&params),
            "mount.spawn_horse" => self.mount_spawn_horse(&params),
            "mount.mount_rider" => self.mount_mount_rider(&params),
            "mount.dismount" => self.mount_dismount(&params),
            "mount.set_gait" => self.mount_set_gait(&params),
            "mount.set_path_follow" => self.mount_set_path_follow(&params),
            "ai.create_navmesh" => self.ai_create_navmesh(&params),
            "ai.bake_navmesh" => self.ai_bake_navmesh(&params),
            "ai.add_agent" => self.ai_add_agent(&params),
            "ai.set_destination" => self.ai_set_destination(&params),
            "ai.create_behavior_tree" => self.ai_create_behavior_tree(&params),
            "ai.bt_add_node" => self.ai_bt_add_node(&params),
            "ai.bt_connect" => self.ai_bt_connect(&params),
            "ai.assign_behavior" => self.ai_assign_behavior(&params),
            "ai.set_blackboard" => self.ai_set_blackboard(&params),
            "ui.create_canvas" => self.ui_create_canvas(&params),
            "ui.add_panel" => self.ui_add_panel(&params),
            "ui.add_text" => self.ui_add_text(&params),
            "ui.add_button" => self.ui_add_button(&params),
            "ui.bind_to_data" => self.ui_bind_to_data(&params),
            "ui.create_hud_template" => self.ui_create_hud_template(&params),
            "audio.import_clip" => self.audio_import_clip(&params),
            "audio.create_source" => self.audio_create_source(&params),
            "audio.play" => self.audio_play(&params),
            "audio.set_spatial" => self.audio_set_spatial(&params),
            "audio.create_mixer" => self.audio_create_mixer(&params),
            "audio.route" => self.audio_route(&params),
            "net.create_server" => self.net_create_server(&params),
            "net.connect_client" => self.net_connect_client(&params),
            "net.enable_replication" => self.net_enable_replication(&params),
            "net.set_prediction" => self.net_set_prediction(&params),
            "net.set_rollback" => self.net_set_rollback(&params),
            "build.set_target" => self.build_set_target(&params),
            "build.set_bundle_id" => self.build_set_bundle_id(&params),
            "build.set_version" => self.build_set_version(&params),
            "build.enable_feature" => self.build_enable_feature(&params),
            "build.export_project" => self.build_export_project(&params),
            "build.generate_installer" => self.build_generate_installer(&params),
            "history.undo" => self.history_undo(&params),
            "history.redo" => self.history_redo(&params),
            "history.mark" => self.history_mark(&params),
            "history.jump_to" => self.history_jump_to(&params),
            "gen.create_game_from_template" => self.gen_create_game_from_template(&params),
            "gen.create_platformer_level" => self.gen_create_platformer_level(&params),
            "gen.create_shooter_arena" => self.gen_create_shooter_arena(&params),
            "gen.create_island_adventure" => self.gen_create_island_adventure(&params),
            "gen.plan_from_prompt" => self.gen_plan_from_prompt(&params),
            "gen.execute_plan" => self.gen_execute_plan(&params),
            "gen.validate_gameplay" => self.gen_validate_gameplay(&params),
            "gen.package_demo_build" => self.gen_package_demo_build(&params),
            "build.build_and_run" => self.build_build_and_run(&params),
            "debug.show_colliders" => self.debug_show_colliders(&params),
            "debug.show_navmesh" => self.debug_show_navmesh(&params),
            "debug.toggle_wireframe" => self.debug_toggle_wireframe(&params),
            "debug.capture_frame" => self.debug_capture_frame(&params),
            "debug.get_profiler_snapshot" => self.debug_get_profiler_snapshot(&params),
            "debug.find_performance_hotspots" => self.debug_find_performance_hotspots(&params),
            _ => bail!("tool '{}' is not registered", tool_name),
        };
        if let Err(err) = &result {
            self.push_diagnostic(
                "error",
                format!("tool '{}' failed: {}", tool_name, err),
                "tool.execute",
            );
        }
        result
    }

    fn register_default_tools(&mut self) {
        self.register_tool(
            "tool.get_engine_state",
            "Returns engine/runtime snapshot (version, fps, memory, open scene).",
            vec![ToolPermission::Read],
            json!({"type":"object","properties":{}}),
        );
        self.register_tool(
            "tool.get_project_tree",
            "Lists project directories/files with optional max_entries limit.",
            vec![ToolPermission::Read],
            json!({"type":"object","properties":{"max_entries":{"type":"integer","minimum":32}}}),
        );
        self.register_tool(
            "tool.search_assets",
            "Searches assets/samples by filename query.",
            vec![ToolPermission::Read],
            json!({"type":"object","required":["query"],"properties":{"query":{"type":"string"}}}),
        );
        self.register_tool(
            "tool.read_asset_metadata",
            "Reads metadata for an asset path.",
            vec![ToolPermission::Read],
            json!({"type":"object","required":["asset_id"],"properties":{"asset_id":{"type":"string"}}}),
        );
        self.register_tool(
            "tool.get_selection",
            "Returns selected entities in editor context.",
            vec![ToolPermission::Read],
            json!({"type":"object","properties":{}}),
        );
        self.register_tool(
            "tool.set_selection",
            "Sets selected entities in editor context.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_ids"],"properties":{"entity_ids":{"type":"array","items":{"type":"string"}}}}),
        );
        self.register_tool(
            "tool.get_viewport_camera",
            "Returns viewport camera state.",
            vec![ToolPermission::Read],
            json!({"type":"object","properties":{}}),
        );
        self.register_tool(
            "tool.set_viewport_camera",
            "Updates viewport camera state.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{"position":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"target":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"fov_y_deg":{"type":"number"}}}),
        );
        self.register_tool(
            "tool.get_cycle_context",
            "Returns condensed cycle context (scene summary, selection, resources, diagnostics, objective, constraints, memory).",
            vec![ToolPermission::Read],
            json!({"type":"object","properties":{
                "max_entities":{"type":"integer","minimum":1},
                "recent_commands":{"type":"integer","minimum":1},
                "diagnostics_last_n":{"type":"integer","minimum":1}
            }}),
        );
        self.register_tool(
            "tool.get_rules",
            "Returns runtime rules/limits and tool permission index.",
            vec![ToolPermission::Read],
            json!({"type":"object","properties":{}}),
        );
        self.register_tool(
            "tool.get_project_memory",
            "Reads project memory (style/targets/notes) used by planning loops.",
            vec![ToolPermission::Read],
            json!({"type":"object","properties":{}}),
        );
        self.register_tool(
            "tool.set_project_memory",
            "Updates project memory (merge/replace modes).",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "memory":{"type":"object"},
                "merge":{"type":"boolean"},
                "style":{"type":"string"},
                "target_platform":{"type":"string"},
                "target_fps":{"type":"number"},
                "notes":{"type":"array","items":{"type":"string"}},
                "tags":{"type":"array","items":{"type":"string"}}
            }}),
        );
        self.register_tool(
            "tool.get_constraints",
            "Reads active generation/runtime constraints.",
            vec![ToolPermission::Read],
            json!({"type":"object","properties":{}}),
        );
        self.register_tool(
            "tool.set_constraints",
            "Updates constraints (merge/replace modes).",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "constraints":{"type":"object"},
                "merge":{"type":"boolean"},
                "target_fps":{"type":"number"},
                "resolution":{"type":"string"},
                "allow_external_assets":{"type":"boolean"},
                "max_gpu_memory_mb":{"type":"integer","minimum":1},
                "max_system_memory_mb":{"type":"integer","minimum":1},
                "notes":{"type":"array","items":{"type":"string"}}
            }}),
        );
        self.register_tool(
            "tool.set_objective",
            "Sets current scene objective text for the AI cycle.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["objective"],"properties":{"objective":{"type":"string"}}}),
        );
        self.register_tool(
            "tool.get_diagnostics",
            "Returns recent diagnostics (warnings/errors/info).",
            vec![ToolPermission::Read],
            json!({"type":"object","properties":{
                "last_n":{"type":"integer","minimum":1},
                "level":{"type":"string"}
            }}),
        );
        self.register_tool(
            "tool.clear_diagnostics",
            "Clears diagnostics, optionally filtered by level.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{"level":{"type":"string"}}}),
        );
        self.register_tool(
            "tool.begin_transaction",
            "Starts a transaction buffer for command execution.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["name"],"properties":{"name":{"type":"string"}}}),
        );
        self.register_tool(
            "tool.commit_transaction",
            "Commits active transaction commands into history.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{}}),
        );
        self.register_tool(
            "tool.rollback_transaction",
            "Rolls back active transaction and undoes buffered commands.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{}}),
        );
        self.register_tool(
            "tool.create_checkpoint",
            "Creates a rollback checkpoint in active transaction.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["label"],"properties":{"label":{"type":"string"}}}),
        );
        self.register_tool(
            "tool.rollback_to_checkpoint",
            "Rolls back active transaction to a checkpoint label.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["label"],"properties":{"label":{"type":"string"}}}),
        );
        self.register_tool(
            "tool.log",
            "Writes an engine log line (trace/debug/info/warn/error).",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["message"],"properties":{"message":{"type":"string"},"level":{"type":"string"}}}),
        );
        self.register_tool(
            "tool.open_task",
            "Creates a task record for long-running flows.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["title"],"properties":{"title":{"type":"string"}}}),
        );
        self.register_tool(
            "tool.update_task",
            "Updates task progress and optional status.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["task_id"],"properties":{"task_id":{"type":"string"},"progress":{"type":"number"},"status":{"type":"string"}}}),
        );
        self.register_tool(
            "tool.close_task",
            "Closes an open task with a final status.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["task_id","status"],"properties":{"task_id":{"type":"string"},"status":{"type":"string"}}}),
        );
        self.register_tool(
            "scene.create",
            "Creates a new empty scene in memory.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["name"],"properties":{"name":{"type":"string"}}}),
        );
        self.register_tool(
            "scene.open",
            "Opens a scene JSON from disk.",
            vec![ToolPermission::Read, ToolPermission::Write],
            json!({"type":"object","required":["scene_id"],"properties":{"scene_id":{"type":"string"}}}),
        );
        self.register_tool(
            "scene.save",
            "Saves current scene to its bound path.",
            vec![ToolPermission::Export],
            json!({"type":"object","properties":{}}),
        );
        self.register_tool(
            "scene.save_as",
            "Saves current scene to a new path/name.",
            vec![ToolPermission::Export],
            json!({"type":"object","required":["name"],"properties":{"name":{"type":"string"}}}),
        );
        self.register_tool(
            "scene.duplicate",
            "Duplicates a scene JSON into a new target file.",
            vec![ToolPermission::Read, ToolPermission::Write, ToolPermission::Export],
            json!({"type":"object","required":["scene_id"],"properties":{"scene_id":{"type":"string"},"name":{"type":"string"},"target_scene_name":{"type":"string"}}}),
        );
        self.register_tool(
            "scene.close",
            "Closes active scene and resets to empty scene.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{}}),
        );
        self.register_tool(
            "scene.set_sky",
            "Sets active scene sky preset.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["preset"],"properties":{"preset":{"type":"string"}}}),
        );
        self.register_tool(
            "scene.set_time_of_day",
            "Sets active scene time of day (0-24).",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["value"],"properties":{"value":{"type":"number"}}}),
        );
        self.register_tool(
            "scene.add_fog",
            "Configures active scene fog settings.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{"density":{"type":"number"},"color":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"start":{"type":"number"},"end":{"type":"number"}}}),
        );
        self.register_tool(
            "scene.enable_world_streaming",
            "Enables world streaming for the active scene.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{"chunksize":{"type":"number"},"chunk_size":{"type":"number"},"range":{"type":"integer","minimum":1}}}),
        );
        self.register_tool(
            "scene.create_stream_chunk",
            "Creates or updates a world streaming chunk.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{"chunk_id":{"type":"string"},"center":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"radius":{"type":"number"}}}),
        );
        self.register_tool(
            "scene.assign_entity_to_chunk",
            "Assigns an entity to a world streaming chunk.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","chunk_id"],"properties":{"entity_id":{"type":"string"},"chunk_id":{"type":"string"}}}),
        );
        self.register_tool(
            "scene.add_postprocess",
            "Updates postprocess controls (exposure/gamma/bloom/fog/color grading).",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "preset":{"type":"string"},
                "exposure":{"type":"number"},
                "gamma":{"type":"number"},
                "bloom_intensity":{"type":"number"},
                "bloom_threshold":{"type":"number"},
                "bloom_radius":{"type":"number"},
                "fog_density":{"type":"number"},
                "fog_color":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "saturation":{"type":"number"},
                "contrast":{"type":"number"},
                "white_balance":{"type":"number"},
                "grade_tint":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}
            }}),
        );
        self.register_tool(
            "entity.create",
            "Creates an entity in current scene.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["name"],"properties":{"name":{"type":"string"},"mesh":{"type":"string"},"translation":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}}}),
        );
        self.register_tool(
            "entity.clone",
            "Clones an existing entity, optionally copying components/hierarchy.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{
                "entity_id":{"type":"string"},
                "name":{"type":"string"},
                "translation_offset":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "copy_components":{"type":"boolean"},
                "copy_parent":{"type":"boolean"}
            }}),
        );
        self.register_tool(
            "entity.delete",
            "Deletes an entity from current scene.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{"entity_id":{"type":"string"}}}),
        );
        self.register_tool(
            "entity.rename",
            "Renames an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","name"],"properties":{"entity_id":{"type":"string"},"name":{"type":"string"}}}),
        );
        self.register_tool(
            "entity.parent",
            "Parents a child entity under a parent entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["child_id","parent_id"],"properties":{"child_id":{"type":"string"},"parent_id":{"type":"string"}}}),
        );
        self.register_tool(
            "entity.unparent",
            "Removes parent relationship for an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["child_id"],"properties":{"child_id":{"type":"string"},"entity_id":{"type":"string"}}}),
        );
        self.register_tool(
            "entity.find_by_name",
            "Finds entities by exact name or substring query.",
            vec![ToolPermission::Read],
            json!({"type":"object","properties":{"name":{"type":"string"},"query":{"type":"string"},"exact":{"type":"boolean"}}}),
        );
        self.register_tool(
            "entity.find_by_tag",
            "Finds entities by tag from Tag/Tags components.",
            vec![ToolPermission::Read],
            json!({"type":"object","required":["tag"],"properties":{"tag":{"type":"string"}}}),
        );
        self.register_tool(
            "entity.set_transform",
            "Updates entity position.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{"entity_id":{"type":"string"},"pos":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"translation":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}}}),
        );
        self.register_tool(
            "entity.translate",
            "Adds delta translation to an entity transform.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{"entity_id":{"type":"string"},"delta":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}}}),
        );
        self.register_tool(
            "entity.rotate",
            "Adds Euler rotation delta (stored in TransformRotation component).",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{"entity_id":{"type":"string"},"delta":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}}}),
        );
        self.register_tool(
            "entity.scale",
            "Scales an entity by scalar or vec3 factor (stored in TransformScale component).",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{"entity_id":{"type":"string"},"factor":{"oneOf":[{"type":"number"},{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}]}}}),
        );
        self.register_tool(
            "entity.get_transform",
            "Returns entity transform.",
            vec![ToolPermission::Read],
            json!({"type":"object","required":["entity_id"],"properties":{"entity_id":{"type":"string"}}}),
        );
        self.register_tool(
            "entity.add_component",
            "Adds or updates a component payload for an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","component_type","data"],"properties":{"entity_id":{"type":"string"},"component_type":{"type":"string"},"data":{"type":"object"}}}),
        );
        self.register_tool(
            "entity.remove_component",
            "Removes a component payload from an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","component_type"],"properties":{"entity_id":{"type":"string"},"component_type":{"type":"string"}}}),
        );
        self.register_tool(
            "entity.get_component",
            "Returns a specific component payload for an entity.",
            vec![ToolPermission::Read],
            json!({"type":"object","required":["entity_id","component_type"],"properties":{"entity_id":{"type":"string"},"component_type":{"type":"string"}}}),
        );
        self.register_tool(
            "entity.set_component",
            "Sets or replaces a component payload for an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","component_type","data"],"properties":{"entity_id":{"type":"string"},"component_type":{"type":"string"},"data":{}}}),
        );
        self.register_tool(
            "asset.import_file",
            "Imports an external file into project assets/imported.",
            vec![ToolPermission::Write, ToolPermission::Export],
            json!({"type":"object","required":["path"],"properties":{"path":{"type":"string"},"target_subdir":{"type":"string"},"options":{"type":"object"}}}),
        );
        self.register_tool(
            "asset.import_url",
            "Imports an asset from URL into project assets/imported.",
            vec![ToolPermission::Write, ToolPermission::Export, ToolPermission::Network],
            json!({"type":"object","required":["url"],"properties":{"url":{"type":"string"},"target_subdir":{"type":"string"},"file_name":{"type":"string"},"options":{"type":"object"}}}),
        );
        self.register_tool(
            "asset.create_material",
            "Creates a material descriptor JSON under assets/materials.",
            vec![ToolPermission::Write, ToolPermission::Export],
            json!({"type":"object","required":["name"],"properties":{"name":{"type":"string"},"preset":{"type":"string"},"params":{"type":"object"}}}),
        );
        self.register_tool(
            "asset.create_texture",
            "Creates a texture descriptor under assets/textures.",
            vec![ToolPermission::Write, ToolPermission::Export],
            json!({"type":"object","required":["name","width","height","format"],"properties":{"name":{"type":"string"},"texture_id":{"type":"string"},"width":{"type":"integer","minimum":1},"height":{"type":"integer","minimum":1},"format":{"type":"string"},"params":{"type":"object"}}}),
        );
        self.register_tool(
            "asset.create_shader",
            "Creates a shader descriptor under assets/shaders.",
            vec![ToolPermission::Write, ToolPermission::Export],
            json!({"type":"object","required":["name","template"],"properties":{"name":{"type":"string"},"shader_id":{"type":"string"},"template":{"type":"string"},"params":{"type":"object"}}}),
        );
        self.register_tool(
            "asset.create_prefab",
            "Creates a prefab snapshot from an entity under assets/prefabs.",
            vec![ToolPermission::Write, ToolPermission::Export],
            json!({"type":"object","required":["name","entity_id"],"properties":{"name":{"type":"string"},"entity_id":{"type":"string"},"prefab_id":{"type":"string"},"metadata":{"type":"object"}}}),
        );
        self.register_tool(
            "asset.save_prefab",
            "Saves an existing prefab snapshot to disk.",
            vec![ToolPermission::Write, ToolPermission::Export],
            json!({"type":"object","required":["prefab_id"],"properties":{"prefab_id":{"type":"string"}}}),
        );
        self.register_tool(
            "asset.rebuild_import",
            "Rebuilds/reprocesses imported asset metadata.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["asset_id"],"properties":{"asset_id":{"type":"string"},"params":{"type":"object"}}}),
        );
        self.register_tool(
            "asset.generate_lods",
            "Generates runtime LOD metadata for a mesh.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["mesh_id"],"properties":{"mesh_id":{"type":"string"},"levels":{"type":"integer","minimum":1},"reduction":{"type":"number"},"params":{"type":"object"}}}),
        );
        self.register_tool(
            "asset.mesh_optimize",
            "Runs mesh optimization metadata pass for a mesh.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["mesh_id"],"properties":{"mesh_id":{"type":"string"},"profile":{"type":"string"},"params":{"type":"object"}}}),
        );
        self.register_tool(
            "asset.compress_textures",
            "Runs texture compression metadata pass for a texture/imported asset.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["asset_id"],"properties":{"asset_id":{"type":"string"},"format":{"type":"string"},"quality":{"type":"string"},"params":{"type":"object"}}}),
        );
        self.register_tool(
            "asset.bake_lightmaps",
            "Triggers lightmap bake metadata job.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{"params":{"type":"object"}}}),
        );
        self.register_tool(
            "asset.bake_reflection_probes",
            "Triggers reflection probe bake metadata job.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{"params":{"type":"object"}}}),
        );
        self.register_tool(
            "asset.assign_material",
            "Assigns a material to an entity (slot-aware material override component).",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","material_id"],"properties":{
                "entity_id":{"type":"string"},
                "material_id":{"type":"string"},
                "slot":{"type":"string"}
            }}),
        );
        self.register_tool(
            "asset.instantiate_prefab",
            "Instantiates an imported asset/prefab into the open scene.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["prefab_id"],"properties":{"prefab_id":{"type":"string"},"asset_id":{"type":"string"},"entity_name":{"type":"string"},"translation":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"transform":{"type":"object"}}}),
        );
        self.register_tool(
            "render.create_light",
            "Creates/sets the directional key light.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{"type":{"type":"string"},"direction":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"color":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},"intensity":{"type":"number"}}}),
        );
        self.register_tool(
            "render.set_light_params",
            "Updates directional light direction/color/intensity.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "direction":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "color":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "intensity":{"type":"number"},
                "shadow_bias":{"type":"number"},
                "shadow_strength":{"type":"number"},
                "shadow_cascade_count":{"type":"integer","minimum":1,"maximum":3}
            }}),
        );
        self.register_tool(
            "render.set_ibl",
            "Updates IBL baseline colors and intensity.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "sky_color":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "ground_color":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "intensity":{"type":"number"}
            }}),
        );
        self.register_tool(
            "render.set_postprocess",
            "Updates postprocess settings (exposure/gamma/bloom/fog/color grading).",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "preset":{"type":"string"},
                "exposure":{"type":"number"},
                "gamma":{"type":"number"},
                "bloom_intensity":{"type":"number"},
                "bloom_threshold":{"type":"number"},
                "bloom_radius":{"type":"number"},
                "fog_density":{"type":"number"},
                "fog_color":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "saturation":{"type":"number"},
                "contrast":{"type":"number"},
                "white_balance":{"type":"number"},
                "grade_tint":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}
            }}),
        );
        self.register_tool(
            "render.set_lod_settings",
            "Updates automatic LOD transition distances and hysteresis.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "transition_distances":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2},
                "near_distance":{"type":"number"},
                "far_distance":{"type":"number"},
                "hysteresis":{"type":"number"}
            }}),
        );
        self.register_tool(
            "render.assign_material",
            "Alias for asset.assign_material to bind materials from render workflows.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","material_id"],"properties":{
                "entity_id":{"type":"string"},
                "material_id":{"type":"string"},
                "slot":{"type":"string"}
            }}),
        );
        self.register_tool(
            "graph.create",
            "Creates a new node graph.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["graph_name"],"properties":{"graph_name":{"type":"string"}}}),
        );
        self.register_tool(
            "graph.add_node",
            "Adds a node to the active graph.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["id","type"],"properties":{
                "id":{"type":"string"},
                "type":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "graph.connect",
            "Creates a directed edge between two graph nodes.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["from","to"],"properties":{
                "from":{"type":"string"},
                "to":{"type":"string"},
                "pin":{"type":"string"}
            }}),
        );
        self.register_tool(
            "graph.delete_node",
            "Deletes a node (and connected edges) from active graph.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}),
        );
        self.register_tool(
            "graph.delete_edge",
            "Deletes an edge from active graph.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["from","to"],"properties":{
                "from":{"type":"string"},
                "to":{"type":"string"},
                "pin":{"type":"string"}
            }}),
        );
        self.register_tool(
            "graph.set_node_params",
            "Updates node params on active graph.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["id","params"],"properties":{
                "id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "graph.validate",
            "Validates active node graph (node contracts, edges, DAG).",
            vec![ToolPermission::Read],
            json!({"type":"object","properties":{}}),
        );
        self.register_tool(
            "graph.run",
            "Executes active graph deterministically for selected events.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{"events":{"type":"array","items":{"type":"string"}}}}),
        );
        self.register_tool(
            "template.list",
            "Lists built-in low-code templates.",
            vec![ToolPermission::Read],
            json!({"type":"object","properties":{}}),
        );
        self.register_tool(
            "template.apply",
            "Applies a built-in template scene+graph bundle.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["template_id"],"properties":{"template_id":{"type":"string"}}}),
        );
        self.register_tool(
            "asset.get_template_bundle",
            "Returns declared assets for a template bundle.",
            vec![ToolPermission::Read],
            json!({"type":"object","required":["template_id"],"properties":{"template_id":{"type":"string"}}}),
        );
        self.register_tool(
            "asset.validate_template_bundle",
            "Validates availability of template bundle assets.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["template_id"],"properties":{"template_id":{"type":"string"}}}),
        );
        self.register_tool(
            "phys.add_collider",
            "Adds or updates collider settings on an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{
                "entity_id":{"type":"string"},
                "shape":{"type":"string"},
                "size":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "is_trigger":{"type":"boolean"}
            }}),
        );
        self.register_tool(
            "phys.set_collider",
            "Updates collider settings on an entity (alias of add/update).",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{
                "entity_id":{"type":"string"},
                "shape":{"type":"string"},
                "size":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "is_trigger":{"type":"boolean"}
            }}),
        );
        self.register_tool(
            "phys.remove_collider",
            "Removes collider component/state from an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{
                "entity_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "phys.add_rigidbody",
            "Adds or updates rigidbody settings on an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{
                "entity_id":{"type":"string"},
                "type":{"type":"string"},
                "mass":{"type":"number"},
                "friction":{"type":"number"},
                "restitution":{"type":"number"}
            }}),
        );
        self.register_tool(
            "phys.set_mass",
            "Sets rigidbody mass for an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","mass"],"properties":{
                "entity_id":{"type":"string"},
                "mass":{"type":"number"}
            }}),
        );
        self.register_tool(
            "phys.set_friction",
            "Sets rigidbody friction for an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","value"],"properties":{
                "entity_id":{"type":"string"},
                "value":{"type":"number"}
            }}),
        );
        self.register_tool(
            "phys.set_restitution",
            "Sets rigidbody restitution for an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","value"],"properties":{
                "entity_id":{"type":"string"},
                "value":{"type":"number"}
            }}),
        );
        self.register_tool(
            "phys.apply_force",
            "Applies force vector over dt to an entity rigidbody.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","force"],"properties":{
                "entity_id":{"type":"string"},
                "force":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "dt":{"type":"number"}
            }}),
        );
        self.register_tool(
            "phys.apply_impulse",
            "Applies an impulse vector to an entity rigidbody.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","impulse"],"properties":{
                "entity_id":{"type":"string"},
                "impulse":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}
            }}),
        );
        self.register_tool(
            "phys.set_gravity",
            "Sets world gravity vector used by physics tools.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["gravity"],"properties":{
                "gravity":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}
            }}),
        );
        self.register_tool(
            "phys.raycast",
            "Runs a lightweight raycast against entity colliders.",
            vec![ToolPermission::Read],
            json!({"type":"object","required":["origin","dir"],"properties":{
                "origin":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "dir":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "maxdist":{"type":"number"}
            }}),
        );
        self.register_tool(
            "phys.overlap",
            "Queries entities overlapping a box/sphere volume.",
            vec![ToolPermission::Read],
            json!({"type":"object","required":["shape","center"],"properties":{
                "shape":{"type":"string"},
                "center":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "size":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "radius":{"type":"number"}
            }}),
        );
        self.register_tool(
            "phys.add_character_controller",
            "Adds character controller settings to an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{
                "entity_id":{"type":"string"},
                "radius":{"type":"number"},
                "height":{"type":"number"},
                "speed":{"type":"number"},
                "jump_strength":{"type":"number"}
            }}),
        );
        self.register_tool(
            "phys.character_move",
            "Moves character controller from input vector and dt.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","input"],"properties":{
                "entity_id":{"type":"string"},
                "input":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "dt":{"type":"number"}
            }}),
        );
        self.register_tool(
            "phys.character_jump",
            "Triggers a character controller jump.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{
                "entity_id":{"type":"string"},
                "strength":{"type":"number"}
            }}),
        );
        self.register_tool(
            "phys.character_set_state",
            "Sets a character controller state label.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","state"],"properties":{
                "entity_id":{"type":"string"},
                "state":{"type":"string"}
            }}),
        );
        self.register_tool(
            "game.create_input_action",
            "Creates an input action with binding list.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["name","bindings"],"properties":{
                "name":{"type":"string"},
                "bindings":{"type":"array","items":{"type":"string"}}
            }}),
        );
        self.register_tool(
            "game.bind_action",
            "Binds input action to a script event name.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["name","target_script_event"],"properties":{
                "name":{"type":"string"},
                "target_script_event":{"type":"string"}
            }}),
        );
        self.register_tool(
            "game.set_rebind",
            "Rebinds an existing input action to a new primary binding.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["action","binding"],"properties":{
                "action":{"type":"string"},
                "binding":{"type":"string"}
            }}),
        );
        self.register_tool(
            "game.create_weapon",
            "Creates or updates a weapon definition.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["weapon_id"],"properties":{
                "weapon_id":{"type":"string"},
                "name":{"type":"string"},
                "rate":{"type":"number"},
                "recoil":{"type":"number"},
                "spread":{"type":"number"},
                "ammo_capacity":{"type":"integer","minimum":1}
            }}),
        );
        self.register_tool(
            "game.attach_weapon",
            "Attaches an existing weapon to an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["character_id","weapon_id"],"properties":{
                "character_id":{"type":"string"},
                "entity_id":{"type":"string"},
                "weapon_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "game.fire_weapon",
            "Fires weapon once and consumes ammo.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "weapon_id":{"type":"string"},
                "character_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "game.apply_damage",
            "Applies damage to target health state.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["target_id","amount"],"properties":{
                "target_id":{"type":"string"},
                "amount":{"type":"number"},
                "damage_type":{"type":"string"}
            }}),
        );
        self.register_tool(
            "game.add_health_component",
            "Adds canonical health component to entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{
                "entity_id":{"type":"string"},
                "max_health":{"type":"number"},
                "current_health":{"type":"number"}
            }}),
        );
        self.register_tool(
            "game.add_trigger",
            "Adds trigger metadata/component to an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{
                "entity_id":{"type":"string"},
                "shape":{"type":"string"},
                "radius":{"type":"number"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "game.add_pickup",
            "Adds pickup metadata/component to an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","item_data"],"properties":{
                "entity_id":{"type":"string"},
                "item_data":{"type":"object"}
            }}),
        );
        self.register_tool(
            "game.add_inventory",
            "Adds inventory metadata/component to an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{
                "entity_id":{"type":"string"},
                "capacity":{"type":"integer","minimum":1},
                "items":{"type":"array","items":{"type":"string"}}
            }}),
        );
        self.register_tool(
            "game.add_interactable",
            "Adds interactable metadata/component to an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","prompt"],"properties":{
                "entity_id":{"type":"string"},
                "prompt":{"type":"string"},
                "actions":{"type":"array","items":{"type":"string"}}
            }}),
        );
        self.register_tool(
            "anim.add_animator",
            "Assigns an animation controller to an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","controller_id"],"properties":{
                "entity_id":{"type":"string"},
                "controller_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "anim.create_state_machine",
            "Creates or updates an animation state machine.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["name"],"properties":{
                "name":{"type":"string"},
                "controller_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "anim.add_state",
            "Adds a state and clip to a controller.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["controller_id","state_name","clip_id"],"properties":{
                "controller_id":{"type":"string"},
                "state_name":{"type":"string"},
                "clip_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "anim.add_transition",
            "Adds transition conditions between two states.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["controller_id","from","to"],"properties":{
                "controller_id":{"type":"string"},
                "from":{"type":"string"},
                "to":{"type":"string"},
                "conditions":{"type":"object"}
            }}),
        );
        self.register_tool(
            "anim.set_parameter",
            "Sets a runtime parameter on a controller.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["controller_id","key","value"],"properties":{
                "controller_id":{"type":"string"},
                "key":{"type":"string"},
                "value":{}
            }}),
        );
        self.register_tool(
            "anim.play",
            "Plays an animation clip on an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","clip_id"],"properties":{
                "entity_id":{"type":"string"},
                "clip_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "anim.blend",
            "Blends two animation clips on an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","clip_a","clip_b"],"properties":{
                "entity_id":{"type":"string"},
                "clip_a":{"type":"string"},
                "clip_b":{"type":"string"},
                "weight":{"type":"number"}
            }}),
        );
        self.register_tool(
            "anim.add_ik",
            "Adds IK chain configuration to an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","chain"],"properties":{
                "entity_id":{"type":"string"},
                "chain":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "anim.retarget",
            "Queues a retarget operation from source rig to target rig.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["source_rig","target_rig"],"properties":{
                "source_rig":{"type":"string"},
                "target_rig":{"type":"string"},
                "mapping":{"type":"object"}
            }}),
        );
        self.register_tool(
            "anim.bake_animation",
            "Queues bake operation for entity animation.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{
                "entity_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "model.create_primitive",
            "Creates a primitive mesh as a scene entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["type","name"],"properties":{
                "type":{"type":"string"},
                "name":{"type":"string"},
                "mesh_id":{"type":"string"},
                "translation":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}
            }}),
        );
        self.register_tool(
            "model.enter_edit_mode",
            "Enters edit mode for a mesh.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["mesh_id"],"properties":{"mesh_id":{"type":"string"}}}),
        );
        self.register_tool(
            "model.exit_edit_mode",
            "Exits edit mode for a mesh.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["mesh_id"],"properties":{"mesh_id":{"type":"string"}}}),
        );
        self.register_tool(
            "model.select",
            "Updates mesh edit selection.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["mesh_id","mode"],"properties":{
                "mesh_id":{"type":"string"},
                "mode":{"type":"string"},
                "selector":{}
            }}),
        );
        for tool in [
            "model.extrude",
            "model.inset",
            "model.bevel",
            "model.loop_cut",
            "model.knife",
            "model.merge",
            "model.subdivide",
            "model.triangulate",
            "model.voxel_remesh",
            "model.decimate",
            "model.smooth",
        ] {
            self.register_tool(
                tool,
                "Applies a mesh topology operation.",
                vec![ToolPermission::Write],
                json!({"type":"object","required":["mesh_id"],"properties":{
                    "mesh_id":{"type":"string"},
                    "params":{"type":"object"},
                    "resolution":{"type":"integer"},
                    "ratio":{"type":"number"},
                    "iterations":{"type":"integer"},
                    "path":{"type":"array"}
                }}),
            );
        }
        self.register_tool(
            "model.add_modifier",
            "Adds a mesh modifier to stack.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["mesh_id","type"],"properties":{
                "mesh_id":{"type":"string"},
                "type":{"type":"string"},
                "modifier_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "model.set_modifier",
            "Updates modifier params.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["mesh_id","modifier_id"],"properties":{
                "mesh_id":{"type":"string"},
                "modifier_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "model.apply_modifier",
            "Applies a modifier to mesh.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["mesh_id","modifier_id"],"properties":{
                "mesh_id":{"type":"string"},
                "modifier_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "model.remove_modifier",
            "Removes a modifier from stack.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["mesh_id","modifier_id"],"properties":{
                "mesh_id":{"type":"string"},
                "modifier_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "model.unwrap_uv",
            "Unwraps UV map for mesh.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["mesh_id"],"properties":{
                "mesh_id":{"type":"string"},
                "method":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "model.pack_uv",
            "Packs UV islands for mesh.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["mesh_id"],"properties":{
                "mesh_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "model.generate_lightmap_uv",
            "Generates UV2 for lightmaps.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["mesh_id"],"properties":{
                "mesh_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "model.sculpt_brush",
            "Applies sculpt brush operation.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["mesh_id","brush_type"],"properties":{
                "mesh_id":{"type":"string"},
                "brush_type":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "model.sculpt_mask",
            "Applies sculpt mask operation.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["mesh_id"],"properties":{
                "mesh_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "vfx.create_particle_system",
            "Creates a particle system record.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["name"],"properties":{
                "name":{"type":"string"},
                "particle_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "vfx.set_emitter",
            "Updates emitter parameters for a particle system.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["particle_id"],"properties":{
                "particle_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "vfx.set_forces",
            "Updates force parameters for a particle system.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["particle_id"],"properties":{
                "particle_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "vfx.set_collision",
            "Updates collision parameters for a particle system.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["particle_id"],"properties":{
                "particle_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "vfx.set_renderer",
            "Updates renderer parameters for a particle system.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["particle_id"],"properties":{
                "particle_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "vfx.attach_to_entity",
            "Attaches a particle system to an entity/socket.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["particle_id","entity_id"],"properties":{
                "particle_id":{"type":"string"},
                "entity_id":{"type":"string"},
                "socket":{"type":"string"}
            }}),
        );
        self.register_tool(
            "vfx.create_graph",
            "Creates a VFX graph record.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["name"],"properties":{
                "name":{"type":"string"},
                "graph_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "vfx.add_node",
            "Adds a node to a VFX graph.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["graph_id","node_type"],"properties":{
                "graph_id":{"type":"string"},
                "node_type":{"type":"string"},
                "node_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "vfx.connect",
            "Connects two VFX graph nodes.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["graph_id","out_node","in_node"],"properties":{
                "graph_id":{"type":"string"},
                "out_node":{"type":"string"},
                "in_node":{"type":"string"}
            }}),
        );
        self.register_tool(
            "vfx.compile_graph",
            "Compiles a VFX graph and stores report.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["graph_id"],"properties":{
                "graph_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "water.create_ocean",
            "Creates an ocean water body.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "ocean_id":{"type":"string"},
                "size":{"type":"number"},
                "waves":{"type":"object"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "water.create_river",
            "Creates a river path water body.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["path"],"properties":{
                "river_id":{"type":"string"},
                "path":{"type":"array","items":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "water.create_waterfall",
            "Creates a waterfall record.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "waterfall_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "water.set_waves",
            "Updates ocean waves parameters.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["ocean_id"],"properties":{
                "ocean_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "water.enable_foam",
            "Enables foam params for an ocean.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["ocean_id"],"properties":{
                "ocean_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "water.enable_refraction",
            "Enables refraction params for an ocean.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["ocean_id"],"properties":{
                "ocean_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "water.enable_caustics",
            "Enables caustics params for an ocean.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["ocean_id"],"properties":{
                "ocean_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "water.add_buoyancy",
            "Adds buoyancy settings to an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{
                "entity_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "water.add_drag",
            "Adds water drag settings to an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{
                "entity_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "water.sample_height",
            "Samples ocean height at world position.",
            vec![ToolPermission::Read],
            json!({"type":"object","required":["ocean_id","position"],"properties":{
                "ocean_id":{"type":"string"},
                "position":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}
            }}),
        );
        self.register_tool(
            "mount.create_horse_template",
            "Creates a horse mount template record.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "template_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "mount.spawn_horse",
            "Spawns a horse entity from template.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["template_id"],"properties":{
                "template_id":{"type":"string"},
                "horse_id":{"type":"string"},
                "entity_id":{"type":"string"},
                "translation":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "mount.mount_rider",
            "Mounts a rider entity onto a horse.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["horse_id","rider_id"],"properties":{
                "horse_id":{"type":"string"},
                "rider_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "mount.dismount",
            "Dismounts a rider from current horse.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["rider_id"],"properties":{
                "rider_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "mount.set_gait",
            "Sets horse gait (walk/trot/gallop).",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["horse_id","gait"],"properties":{
                "horse_id":{"type":"string"},
                "gait":{"type":"string"}
            }}),
        );
        self.register_tool(
            "mount.set_path_follow",
            "Assigns a path id for horse path-follow behavior.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["horse_id","path_id"],"properties":{
                "horse_id":{"type":"string"},
                "path_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "ai.create_navmesh",
            "Creates a navmesh record.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "navmesh_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "ai.bake_navmesh",
            "Marks a navmesh as baked.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "navmesh_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "ai.add_agent",
            "Adds an AI agent for an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id"],"properties":{
                "entity_id":{"type":"string"},
                "agent_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "ai.set_destination",
            "Sets navigation destination for an AI agent.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["agent_id","position"],"properties":{
                "agent_id":{"type":"string"},
                "position":{"type":"array","items":{"type":"number"},"minItems":3,"maxItems":3}
            }}),
        );
        self.register_tool(
            "ai.create_behavior_tree",
            "Creates an AI behavior tree.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["name"],"properties":{
                "name":{"type":"string"},
                "tree_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "ai.bt_add_node",
            "Adds a behavior tree node.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["tree_id","node_type"],"properties":{
                "tree_id":{"type":"string"},
                "node_type":{"type":"string"},
                "node_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "ai.bt_connect",
            "Connects behavior tree parent/child nodes.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["tree_id","parent","child"],"properties":{
                "tree_id":{"type":"string"},
                "parent":{"type":"string"},
                "child":{"type":"string"}
            }}),
        );
        self.register_tool(
            "ai.assign_behavior",
            "Assigns a behavior tree to an entity agent.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","tree_id"],"properties":{
                "entity_id":{"type":"string"},
                "tree_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "ai.set_blackboard",
            "Sets a blackboard key/value for entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["entity_id","key","value"],"properties":{
                "entity_id":{"type":"string"},
                "key":{"type":"string"},
                "value":{}
            }}),
        );
        self.register_tool(
            "ui.create_canvas",
            "Creates a UI canvas.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["name"],"properties":{
                "name":{"type":"string"},
                "canvas_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "ui.add_panel",
            "Adds a panel element to a canvas.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["canvas_id"],"properties":{
                "canvas_id":{"type":"string"},
                "ui_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "ui.add_text",
            "Adds a text element to a canvas.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["canvas_id"],"properties":{
                "canvas_id":{"type":"string"},
                "ui_id":{"type":"string"},
                "text":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "ui.add_button",
            "Adds a button element to a canvas.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["canvas_id"],"properties":{
                "canvas_id":{"type":"string"},
                "ui_id":{"type":"string"},
                "label":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "ui.bind_to_data",
            "Binds a UI element to an entity component field.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["ui_id","entity_id","component_field"],"properties":{
                "ui_id":{"type":"string"},
                "entity_id":{"type":"string"},
                "component_field":{"type":"string"}
            }}),
        );
        self.register_tool(
            "ui.create_hud_template",
            "Creates a built-in HUD template (shooter/platformer).",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "type":{"type":"string"},
                "template_type":{"type":"string"}
            }}),
        );
        self.register_tool(
            "audio.import_clip",
            "Imports/registers an audio clip path.",
            vec![ToolPermission::Read, ToolPermission::Write],
            json!({"type":"object","required":["path"],"properties":{
                "path":{"type":"string"},
                "clip_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "audio.create_source",
            "Creates an audio source, optionally attached to an entity.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "source_id":{"type":"string"},
                "entity_id":{"type":"string"},
                "params":{"type":"object"},
                "spatial":{"type":"object"}
            }}),
        );
        self.register_tool(
            "audio.play",
            "Plays a clip on a source.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["source_id","clip_id"],"properties":{
                "source_id":{"type":"string"},
                "clip_id":{"type":"string"}
            }}),
        );
        self.register_tool(
            "audio.set_spatial",
            "Updates spatial audio parameters for a source.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["source_id"],"properties":{
                "source_id":{"type":"string"},
                "params":{"type":"object"},
                "spatial":{"type":"object"}
            }}),
        );
        self.register_tool(
            "audio.create_mixer",
            "Creates an audio mixer bus.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "bus_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "audio.route",
            "Routes a source to a mixer bus.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["source_id","mixer_bus"],"properties":{
                "source_id":{"type":"string"},
                "mixer_bus":{"type":"string"}
            }}),
        );
        self.register_tool(
            "net.create_server",
            "Creates/updates networking server baseline settings.",
            vec![ToolPermission::Write, ToolPermission::Network],
            json!({"type":"object","properties":{
                "server_id":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "net.connect_client",
            "Connects/registers a network client endpoint.",
            vec![ToolPermission::Write, ToolPermission::Network],
            json!({"type":"object","properties":{
                "client_id":{"type":"string"},
                "endpoint":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "net.enable_replication",
            "Enables component replication for an entity.",
            vec![ToolPermission::Write, ToolPermission::Network],
            json!({"type":"object","required":["entity_id","components"],"properties":{
                "entity_id":{"type":"string"},
                "components":{"type":"array","minItems":1,"items":{"type":"string"}}
            }}),
        );
        self.register_tool(
            "net.set_prediction",
            "Sets network prediction mode (server/client/hybrid).",
            vec![ToolPermission::Write, ToolPermission::Network],
            json!({"type":"object","properties":{"mode":{"type":"string"}}}),
        );
        self.register_tool(
            "net.set_rollback",
            "Sets rollback networking parameters.",
            vec![ToolPermission::Write, ToolPermission::Network],
            json!({"type":"object","properties":{"params":{"type":"object"}}}),
        );
        self.register_tool(
            "build.set_target",
            "Sets export build target platform.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "platform":{"type":"string"},
                "target":{"type":"string"}
            }}),
        );
        self.register_tool(
            "build.set_bundle_id",
            "Sets bundle/application identifier.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}),
        );
        self.register_tool(
            "build.set_version",
            "Sets project export version.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["version"],"properties":{"version":{"type":"string"}}}),
        );
        self.register_tool(
            "build.enable_feature",
            "Enables a build/export feature flag.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["flag"],"properties":{"flag":{"type":"string"}}}),
        );
        self.register_tool(
            "build.export_project",
            "Exports a project manifest to the target folder.",
            vec![ToolPermission::Write, ToolPermission::Export],
            json!({"type":"object","required":["path"],"properties":{"path":{"type":"string"}}}),
        );
        self.register_tool(
            "build.generate_installer",
            "Generates an installer manifest file.",
            vec![ToolPermission::Write, ToolPermission::Export],
            json!({"type":"object","required":["path"],"properties":{
                "path":{"type":"string"},
                "params":{"type":"object"}
            }}),
        );
        self.register_tool(
            "history.undo",
            "Undo one or more committed commands.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{"steps":{"type":"integer","minimum":1}}}),
        );
        self.register_tool(
            "history.redo",
            "Redo one or more previously undone commands.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{"steps":{"type":"integer","minimum":1}}}),
        );
        self.register_tool(
            "history.mark",
            "Stores a named mark at current history cursor.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["label"],"properties":{"label":{"type":"string"}}}),
        );
        self.register_tool(
            "history.jump_to",
            "Moves history cursor to a named mark using undo/redo.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["label"],"properties":{"label":{"type":"string"}}}),
        );
        self.register_tool(
            "gen.plan_from_prompt",
            "Builds a minimal task graph from prompt.",
            vec![ToolPermission::Read],
            json!({"type":"object","required":["prompt"],"properties":{"prompt":{"type":"string"}}}),
        );
        self.register_tool(
            "gen.execute_plan",
            "Executes a task graph step-by-step using registered tools.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["task_graph"],"properties":{"task_graph":{"type":"object"}}}),
        );
        self.register_tool(
            "gen.validate_gameplay",
            "Validates scene/gameplay against minimum requirements.",
            vec![ToolPermission::Read],
            json!({"type":"object","properties":{"min_requirements":{"type":"object"}}}),
        );
        self.register_tool(
            "gen.create_game_from_template",
            "Creates a game baseline from a named low-code template and executes it.",
            vec![ToolPermission::Write],
            json!({"type":"object","required":["template_id"],"properties":{
                "template_id":{"type":"string"},
                "auto_transaction":{"type":"boolean"},
                "postprocess":{"type":"object"},
                "postprocess_preset":{"type":"string"},
                "save_as":{"type":"string"}
            }}),
        );
        self.register_tool(
            "gen.create_platformer_level",
            "Macro-tool to generate a platformer level baseline.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "auto_transaction":{"type":"boolean"},
                "save_as":{"type":"string"}
            }}),
        );
        self.register_tool(
            "gen.create_shooter_arena",
            "Macro-tool to generate a shooter arena baseline.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "auto_transaction":{"type":"boolean"},
                "save_as":{"type":"string"}
            }}),
        );
        self.register_tool(
            "gen.create_island_adventure",
            "Macro-tool to generate a medieval/island adventure baseline.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{
                "auto_transaction":{"type":"boolean"},
                "save_as":{"type":"string"}
            }}),
        );
        self.register_tool(
            "gen.package_demo_build",
            "Macro-tool to configure and package a demo build/export pipeline.",
            vec![ToolPermission::Write, ToolPermission::Export],
            json!({"type":"object","properties":{
                "target":{"type":"string"},
                "bundle_id":{"type":"string"},
                "version":{"type":"string"},
                "features":{"type":"array","items":{"type":"string"}},
                "export_path":{"type":"string"},
                "installer_path":{"type":"string"},
                "run_build":{"type":"boolean"},
                "run_target":{"type":"string"},
                "profile":{"type":"string"},
                "run_binary":{"type":"boolean"},
                "dry_run":{"type":"boolean"},
                "wait_for_run":{"type":"boolean"}
            }}),
        );
        self.register_tool(
            "build.build_and_run",
            "Builds project target and optionally runs it.",
            vec![ToolPermission::Export],
            json!({"type":"object","properties":{"target":{"type":"string"},"profile":{"type":"string"},"run":{"type":"boolean"},"dry_run":{"type":"boolean"},"wait_for_run":{"type":"boolean"}}}),
        );
        self.register_tool(
            "debug.show_colliders",
            "Toggles collider debug visualization flag.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{"on":{"type":"boolean"}}}),
        );
        self.register_tool(
            "debug.show_navmesh",
            "Toggles navmesh debug visualization flag.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{"on":{"type":"boolean"}}}),
        );
        self.register_tool(
            "debug.toggle_wireframe",
            "Toggles wireframe debug rendering flag.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{"on":{"type":"boolean"}}}),
        );
        self.register_tool(
            "debug.capture_frame",
            "Captures a profiler/debug frame snapshot.",
            vec![ToolPermission::Write],
            json!({"type":"object","properties":{}}),
        );
        self.register_tool(
            "debug.get_profiler_snapshot",
            "Returns the latest profiler snapshot or a recent window.",
            vec![ToolPermission::Read],
            json!({"type":"object","properties":{"last_n":{"type":"integer","minimum":1}}}),
        );
        self.register_tool(
            "debug.find_performance_hotspots",
            "Summarizes likely performance hotspots from profiler snapshots.",
            vec![ToolPermission::Read],
            json!({"type":"object","properties":{"last_n":{"type":"integer","minimum":1}}}),
        );
    }

    fn register_tool(
        &mut self,
        name: &str,
        description: &str,
        permissions: Vec<ToolPermission>,
        params_schema: Value,
    ) {
        self.schemas.insert(
            name.to_string(),
            ToolSchema {
                name: name.to_string(),
                description: description.to_string(),
                permissions,
                params_schema,
            },
        );
    }

    fn tool_get_engine_state(&self) -> anyhow::Result<Value> {
        let context = self.command_bus.context();
        let assets_state = json!({
            "imported_assets": context.imported_assets,
            "materials": context.materials,
            "textures": context.textures,
            "shaders": context.shaders,
            "prefabs": context.prefabs,
            "pipeline": context.asset_pipeline
        });
        Ok(json!({
            "version": context.engine_state.version,
            "ai_mode": context.engine_state.ai_mode,
            "fps": context.engine_state.fps,
            "gpu_memory_mb": context.engine_state.gpu_memory_mb,
            "system_memory_mb": context.engine_state.system_memory_mb,
            "open_scene": context.open_scene_label(),
            "ecs_entities": context.ecs_entity_count(),
            "scene_revision": context.revision,
            "active_transaction": self.command_bus.current_transaction_name(),
            "history_len": self.command_bus.history_len(),
            "redo_len": self.command_bus.redo_len(),
            "imported_assets": context.imported_assets.len(),
            "materials": context.materials.len(),
            "textures": context.textures.len(),
            "shaders": context.shaders.len(),
            "prefabs": context.prefabs.len(),
            "assets": assets_state,
            "scene": {
                "name": context.scene.name,
                "entity_count": context.scene.entities.len(),
                "sky_preset": context.scene_runtime.sky_preset,
                "time_of_day": context.scene_runtime.time_of_day,
                "fog": context.scene_runtime.fog,
                "world_streaming": context.scene_runtime.world_streaming,
                "objective": context.scene_runtime.objective,
                "last_message": context.scene_runtime.last_message
            },
            "render": {
                "light_direction": context.render_settings.light_direction,
                "light_color": context.render_settings.light_color,
                "light_intensity": context.render_settings.light_intensity,
                "shadow_bias": context.render_settings.shadow_bias,
                "shadow_strength": context.render_settings.shadow_strength,
                "shadow_cascade_count": context.render_settings.shadow_cascade_count,
                "lod_transition_distances": context.render_settings.lod_transition_distances,
                "lod_hysteresis": context.render_settings.lod_hysteresis,
                "ibl_intensity": context.render_settings.ibl_intensity,
                "ibl_sky_color": context.render_settings.ibl_sky_color,
                "ibl_ground_color": context.render_settings.ibl_ground_color,
                "exposure": context.render_settings.exposure,
                "gamma": context.render_settings.gamma,
                "bloom_intensity": context.render_settings.bloom_intensity,
                "bloom_threshold": context.render_settings.bloom_threshold,
                "bloom_radius": context.render_settings.bloom_radius,
                "fog_density": context.render_settings.fog_density,
                "fog_color": context.render_settings.fog_color,
                "saturation": context.render_settings.saturation,
                "contrast": context.render_settings.contrast,
                "white_balance": context.render_settings.white_balance,
                "grade_tint": context.render_settings.grade_tint,
                "color_grading_preset": context.render_settings.color_grading_preset
            },
            "lowcode": {
                "active_template_id": context.node_graph.active_template_id,
                "graph": context.node_graph.graph,
                "graph_validation": context.node_graph.validation,
                "last_execution": context.node_graph.last_execution,
                "last_bundle_validation": context.node_graph.last_bundle_validation
            },
            "physics": {
                "gravity": context.physics.gravity,
                "collider_count": context.physics.colliders.len(),
                "rigidbody_count": context.physics.rigidbodies.len(),
                "character_controller_count": context.physics.character_controllers.len(),
                "colliders": context.physics.colliders,
                "rigidbodies": context.physics.rigidbodies,
                "character_controllers": context.physics.character_controllers
            },
            "gameplay": {
                "weapon_count": context.gameplay.weapons.len(),
                "attachment_count": context.gameplay.attachments.len(),
                "input_action_count": context.gameplay.input_actions.len(),
                "trigger_count": context.gameplay.triggers.len(),
                "pickup_count": context.gameplay.pickups.len(),
                "inventory_count": context.gameplay.inventories.len(),
                "interactable_count": context.gameplay.interactables.len(),
                "weapons": context.gameplay.weapons,
                "attachments": context.gameplay.attachments,
                "input_actions": context.gameplay.input_actions,
                "triggers": context.gameplay.triggers,
                "pickups": context.gameplay.pickups,
                "inventories": context.gameplay.inventories,
                "interactables": context.gameplay.interactables,
                "fire_events": context.gameplay.fire_events,
                "total_damage_applied": context.gameplay.total_damage_applied
            },
            "animation": {
                "state_machine_count": context.animation.state_machines.len(),
                "animator_count": context.animation.entity_animators.len(),
                "active_clip_count": context.animation.entity_active_clips.len(),
                "blend_count": context.animation.entity_blends.len(),
                "ik_count": context.animation.ik_solvers.len(),
                "retarget_jobs": context.animation.retarget_jobs,
                "bake_jobs": context.animation.bake_jobs,
                "state_machines": context.animation.state_machines,
                "entity_animators": context.animation.entity_animators,
                "entity_active_clips": context.animation.entity_active_clips,
                "entity_blends": context.animation.entity_blends,
                "ik_solvers": context.animation.ik_solvers
            },
            "modeling": {
                "mesh_count": context.modeling.meshes.len(),
                "edit_mode_count": context.modeling.edit_modes.len(),
                "selection_count": context.modeling.selections.len(),
                "modifier_stack_count": context.modeling.modifiers.len(),
                "uv_count": context.modeling.uv.len(),
                "sculpt_mask_count": context.modeling.sculpt_masks.len(),
                "operation_log_len": context.modeling.operation_log.len(),
                "meshes": context.modeling.meshes,
                "edit_modes": context.modeling.edit_modes,
                "selections": context.modeling.selections,
                "modifiers": context.modeling.modifiers,
                "uv": context.modeling.uv,
                "sculpt_masks": context.modeling.sculpt_masks,
                "operation_log": context.modeling.operation_log
            },
            "vfx": {
                "particle_system_count": context.vfx.particle_systems.len(),
                "graph_count": context.vfx.graphs.len(),
                "particle_systems": context.vfx.particle_systems,
                "graphs": context.vfx.graphs
            },
            "water": {
                "ocean_count": context.water.oceans.len(),
                "river_count": context.water.rivers.len(),
                "waterfall_count": context.water.waterfalls.len(),
                "buoyancy_count": context.water.buoyancy.len(),
                "drag_count": context.water.drag.len(),
                "oceans": context.water.oceans,
                "rivers": context.water.rivers,
                "waterfalls": context.water.waterfalls,
                "buoyancy": context.water.buoyancy,
                "drag": context.water.drag
            },
            "mount": {
                "template_count": context.mount.horse_templates.len(),
                "horse_count": context.mount.horses.len(),
                "mounted_rider_count": context.mount.rider_to_horse.len(),
                "horse_templates": context.mount.horse_templates,
                "horses": context.mount.horses,
                "rider_to_horse": context.mount.rider_to_horse
            },
            "ai": {
                "navmesh_count": context.npc_ai.navmeshes.len(),
                "agent_count": context.npc_ai.agents.len(),
                "behavior_tree_count": context.npc_ai.behavior_trees.len(),
                "blackboard_entity_count": context.npc_ai.blackboard.len(),
                "active_navmesh_id": context.npc_ai.active_navmesh_id,
                "navmeshes": context.npc_ai.navmeshes,
                "agents": context.npc_ai.agents,
                "entity_agents": context.npc_ai.entity_agents,
                "behavior_trees": context.npc_ai.behavior_trees,
                "blackboard": context.npc_ai.blackboard
            },
            "ui": {
                "canvas_count": context.ui.canvases.len(),
                "element_count": context.ui.elements.len(),
                "binding_count": context.ui.bindings.len(),
                "active_hud_template": context.ui.active_hud_template,
                "canvases": context.ui.canvases,
                "elements": context.ui.elements,
                "bindings": context.ui.bindings
            },
            "audio": {
                "clip_count": context.audio.clips.len(),
                "source_count": context.audio.sources.len(),
                "mixer_count": context.audio.mixers.len(),
                "play_events": context.audio.play_events,
                "clips": context.audio.clips,
                "sources": context.audio.sources,
                "mixers": context.audio.mixers
            },
            "networking": {
                "has_server": context.networking.server.is_some(),
                "client_count": context.networking.clients.len(),
                "replication_count": context.networking.replication.len(),
                "prediction_mode": context.networking.prediction_mode,
                "server": context.networking.server,
                "clients": context.networking.clients,
                "replication": context.networking.replication,
                "rollback": context.networking.rollback
            },
            "build": {
                "target": context.build.target,
                "bundle_id": context.build.bundle_id,
                "version": context.build.version,
                "enabled_feature_count": context.build.enabled_features.len(),
                "enabled_features": context.build.enabled_features,
                "last_export_path": context.build.last_export_path,
                "last_installer_path": context.build.last_installer_path
            },
            "debug": {
                "show_colliders": context.debug.show_colliders,
                "show_navmesh": context.debug.show_navmesh,
                "wireframe": context.debug.wireframe,
                "captured_frames": context.debug.captured_frames,
                "profiler_snapshot_count": context.debug.profiler_snapshots.len(),
                "latest_profiler_snapshot": context.debug.profiler_snapshots.last(),
                "profiler_snapshots": context.debug.profiler_snapshots
            },
            "project_memory": self.project_memory.clone(),
            "constraints": self.runtime_constraints.clone(),
            "diagnostics": {
                "count": self.diagnostics.len(),
                "warning_count": self
                    .diagnostics
                    .iter()
                    .filter(|entry| entry.level.eq_ignore_ascii_case("warn") || entry.level.eq_ignore_ascii_case("warning"))
                    .count(),
                "error_count": self
                    .diagnostics
                    .iter()
                    .filter(|entry| entry.level.eq_ignore_ascii_case("error"))
                    .count(),
                "latest": self.diagnostics.last()
            }
        }))
    }

    fn tool_get_cycle_context(&self, params: &Value) -> anyhow::Result<Value> {
        let context = self.command_bus.context();
        let max_entities = optional_usize(params, "max_entities").unwrap_or(24).max(1);
        let recent_commands = optional_usize(params, "recent_commands")
            .unwrap_or(8)
            .max(1);
        let diagnostics_last_n = optional_usize(params, "diagnostics_last_n")
            .unwrap_or(16)
            .max(1);
        let scene_entities = context
            .scene
            .entities
            .iter()
            .take(max_entities)
            .map(|entity| {
                json!({
                    "name": entity.name,
                    "mesh": entity.mesh,
                    "translation": entity.translation
                })
            })
            .collect::<Vec<Value>>();
        let replay_tail = self
            .command_bus
            .replay_log()
            .iter()
            .rev()
            .take(recent_commands)
            .map(|entry| {
                json!({
                    "command_id": entry.command_id,
                    "command_name": entry.command_name,
                    "status": entry.status,
                    "summary": entry.result_summary
                })
            })
            .collect::<Vec<Value>>();
        let diagnostics = self.diagnostics_tail(diagnostics_last_n, None);
        let warning_count = diagnostics
            .iter()
            .filter(|entry| {
                entry.level.eq_ignore_ascii_case("warn")
                    || entry.level.eq_ignore_ascii_case("warning")
            })
            .count();
        let error_count = diagnostics
            .iter()
            .filter(|entry| entry.level.eq_ignore_ascii_case("error"))
            .count();

        Ok(json!({
            "scene": {
                "name": context.scene.name,
                "open_scene": context.open_scene_label(),
                "entity_count": context.scene.entities.len(),
                "entity_hierarchy_summary": scene_entities,
                "hierarchy_truncated": context.scene.entities.len() > max_entities
            },
            "selection": {
                "entity_ids": context.selection,
                "count": context.selection.len()
            },
            "resources": {
                "fps": context.engine_state.fps,
                "gpu_memory_mb": context.engine_state.gpu_memory_mb,
                "system_memory_mb": context.engine_state.system_memory_mb
            },
            "objective": context.scene_runtime.objective,
            "constraints": self.runtime_constraints.clone(),
            "project_memory": self.project_memory.clone(),
            "feedback": {
                "last_message": context.scene_runtime.last_message,
                "recent_commands": replay_tail
            },
            "diagnostics": {
                "warning_count": warning_count,
                "error_count": error_count,
                "entries": diagnostics
            }
        }))
    }

    fn tool_get_rules(&self) -> anyhow::Result<Value> {
        let mut read_tools = Vec::<String>::new();
        let mut write_tools = Vec::<String>::new();
        let mut export_tools = Vec::<String>::new();
        let mut network_tools = Vec::<String>::new();
        for schema in self.list_tools() {
            if schema.permissions.contains(&ToolPermission::Read) {
                read_tools.push(schema.name.clone());
            }
            if schema.permissions.contains(&ToolPermission::Write) {
                write_tools.push(schema.name.clone());
            }
            if schema.permissions.contains(&ToolPermission::Export) {
                export_tools.push(schema.name.clone());
            }
            if schema.permissions.contains(&ToolPermission::Network) {
                network_tools.push(schema.name.clone());
            }
        }
        Ok(json!({
            "rules": {
                "tool_only_mutation": true,
                "transaction_support": true,
                "rollback_supported": true,
                "delete_requires_confirmation": true,
                "notes": [
                    "All scene/runtime mutations are expected to route via registered tools.",
                    "Use begin/commit/rollback transaction tools for multi-step edits.",
                    "build.build_and_run is non-rollback-safe in auto transactions."
                ]
            },
            "permissions": {
                "read": read_tools,
                "write": write_tools,
                "export": export_tools,
                "network": network_tools
            }
        }))
    }

    fn tool_get_project_memory(&self) -> anyhow::Result<Value> {
        Ok(json!({
            "project_memory": self.project_memory.clone()
        }))
    }

    fn tool_set_project_memory(&mut self, params: &Value) -> anyhow::Result<Value> {
        let merge = optional_bool(params, "merge").unwrap_or(true);
        let mut incoming = params.get("memory").cloned().unwrap_or_else(|| json!({}));
        if !incoming.is_object() {
            bail!("tool.set_project_memory expects 'memory' to be an object");
        }

        let incoming_obj = incoming
            .as_object_mut()
            .with_context(|| "tool.set_project_memory expects object payload")?;
        if let Some(style) = optional_string(params, "style") {
            incoming_obj.insert("style".to_string(), Value::String(style));
        }
        if let Some(platform) = optional_string(params, "target_platform") {
            incoming_obj.insert("target_platform".to_string(), Value::String(platform));
        }
        if let Some(target_fps) = optional_f32(params, "target_fps") {
            incoming_obj.insert("target_fps".to_string(), json!(target_fps));
        }
        if params.get("notes").is_some() {
            incoming_obj.insert(
                "notes".to_string(),
                json!(optional_string_array(params, "notes")),
            );
        }
        if params.get("tags").is_some() {
            incoming_obj.insert(
                "tags".to_string(),
                json!(optional_string_array(params, "tags")),
            );
        }

        if merge {
            let existing = self
                .project_memory
                .as_object_mut()
                .with_context(|| "internal project_memory must be object")?;
            let incoming_obj = incoming
                .as_object()
                .with_context(|| "tool.set_project_memory incoming payload must be object")?;
            for (key, value) in incoming_obj {
                existing.insert(key.clone(), value.clone());
            }
        } else {
            self.project_memory = incoming;
        }
        Ok(json!({
            "project_memory": self.project_memory.clone(),
            "merge": merge
        }))
    }

    fn tool_get_constraints(&self) -> anyhow::Result<Value> {
        Ok(json!({
            "constraints": self.runtime_constraints.clone()
        }))
    }

    fn tool_set_constraints(&mut self, params: &Value) -> anyhow::Result<Value> {
        let merge = optional_bool(params, "merge").unwrap_or(true);
        let mut incoming = params
            .get("constraints")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !incoming.is_object() {
            bail!("tool.set_constraints expects 'constraints' to be an object");
        }
        let incoming_obj = incoming
            .as_object_mut()
            .with_context(|| "tool.set_constraints expects object payload")?;
        if let Some(target_fps) = optional_f32(params, "target_fps") {
            incoming_obj.insert("target_fps".to_string(), json!(target_fps));
        }
        if let Some(resolution) = optional_string(params, "resolution") {
            incoming_obj.insert("resolution".to_string(), Value::String(resolution));
        }
        if let Some(allow_external_assets) = optional_bool(params, "allow_external_assets") {
            incoming_obj.insert(
                "allow_external_assets".to_string(),
                Value::Bool(allow_external_assets),
            );
        }
        if let Some(max_gpu_memory_mb) = params.get("max_gpu_memory_mb").and_then(Value::as_u64) {
            incoming_obj.insert("max_gpu_memory_mb".to_string(), json!(max_gpu_memory_mb));
        }
        if let Some(max_system_memory_mb) =
            params.get("max_system_memory_mb").and_then(Value::as_u64)
        {
            incoming_obj.insert(
                "max_system_memory_mb".to_string(),
                json!(max_system_memory_mb),
            );
        }
        if params.get("notes").is_some() {
            incoming_obj.insert(
                "notes".to_string(),
                json!(optional_string_array(params, "notes")),
            );
        }

        if merge {
            let existing = self
                .runtime_constraints
                .as_object_mut()
                .with_context(|| "internal runtime_constraints must be object")?;
            let incoming_obj = incoming
                .as_object()
                .with_context(|| "tool.set_constraints incoming payload must be object")?;
            for (key, value) in incoming_obj {
                existing.insert(key.clone(), value.clone());
            }
        } else {
            self.runtime_constraints = incoming;
        }
        Ok(json!({
            "constraints": self.runtime_constraints.clone(),
            "merge": merge
        }))
    }

    fn tool_set_objective(&mut self, params: &Value) -> anyhow::Result<Value> {
        let objective = required_string(params, "objective")?;
        self.command_bus.context_mut().scene_runtime.objective = Some(objective.clone());
        Ok(json!({
            "objective": objective
        }))
    }

    fn tool_get_diagnostics(&self, params: &Value) -> anyhow::Result<Value> {
        let last_n = optional_usize(params, "last_n").unwrap_or(32).max(1);
        let level = optional_string(params, "level");
        let entries = self.diagnostics_tail(last_n, level.as_deref());
        Ok(json!({
            "count": entries.len(),
            "entries": entries
        }))
    }

    fn tool_clear_diagnostics(&mut self, params: &Value) -> anyhow::Result<Value> {
        let level = optional_string(params, "level");
        let previous = self.diagnostics.len();
        if let Some(level) = level {
            self.diagnostics
                .retain(|entry| !entry.level.eq_ignore_ascii_case(&level));
        } else {
            self.diagnostics.clear();
        }
        Ok(json!({
            "cleared": previous.saturating_sub(self.diagnostics.len()),
            "remaining": self.diagnostics.len()
        }))
    }

    fn tool_get_project_tree(&self, params: &Value) -> anyhow::Result<Value> {
        let max_entries = optional_usize(params, "max_entries").unwrap_or(512).max(32);
        collect_project_tree(&self.command_bus.context().project_root, max_entries)
    }

    fn tool_search_assets(&self, params: &Value) -> anyhow::Result<Value> {
        let query = required_string(params, "query")?;
        let query_normalized = query.to_ascii_lowercase();
        let project_root = &self.command_bus.context().project_root;
        let mut results = Vec::new();
        for root_name in ["assets", "samples"] {
            let root = project_root.join(root_name);
            collect_matching_files(project_root, &root, &query_normalized, 128, &mut results);
        }
        results.sort();
        results.dedup();
        Ok(json!({
            "query": query,
            "matches": results,
            "count": results.len()
        }))
    }

    fn tool_read_asset_metadata(&self, params: &Value) -> anyhow::Result<Value> {
        let asset_id = required_string(params, "asset_id")?;
        let project_root = &self.command_bus.context().project_root;
        let asset_path = resolve_project_path(project_root, Path::new(&asset_id));
        let metadata = fs::metadata(&asset_path)
            .with_context(|| format!("failed to read metadata for '{}'", asset_path.display()))?;
        let modified = metadata
            .modified()
            .ok()
            .map(|timestamp| chrono::DateTime::<chrono::Utc>::from(timestamp).to_rfc3339());
        Ok(json!({
            "asset_id": asset_id,
            "path": asset_path.display().to_string(),
            "exists": true,
            "is_dir": metadata.is_dir(),
            "size_bytes": metadata.len(),
            "modified_utc": modified,
            "extension": asset_path.extension().and_then(|ext| ext.to_str())
        }))
    }

    fn tool_get_selection(&self) -> anyhow::Result<Value> {
        Ok(json!({
            "entity_ids": self.command_bus.context().selection
        }))
    }

    fn tool_set_selection(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_ids = required_string_array(params, "entity_ids")?;
        self.command_bus.context_mut().selection = entity_ids.clone();
        Ok(json!({
            "entity_ids": entity_ids,
            "count": entity_ids.len()
        }))
    }

    fn tool_get_viewport_camera(&self) -> anyhow::Result<Value> {
        let camera = &self.command_bus.context().viewport_camera;
        Ok(json!({
            "position": camera.position,
            "target": camera.target,
            "fov_y_deg": camera.fov_y_deg
        }))
    }

    fn tool_set_viewport_camera(&mut self, params: &Value) -> anyhow::Result<Value> {
        if let Some(position) = optional_vec3(params, "position")? {
            self.command_bus.context_mut().viewport_camera.position = position;
        }
        if let Some(target) = optional_vec3(params, "target")? {
            self.command_bus.context_mut().viewport_camera.target = target;
        }
        if let Some(fov_y_deg) = optional_f32(params, "fov_y_deg") {
            self.command_bus.context_mut().viewport_camera.fov_y_deg = fov_y_deg.clamp(10.0, 150.0);
        }
        self.tool_get_viewport_camera()
    }

    fn tool_begin_transaction(&mut self, params: &Value) -> anyhow::Result<Value> {
        let name = required_string(params, "name")?;
        self.command_bus.begin_transaction(name.clone())?;
        Ok(json!({
            "transaction": name,
            "status": "active"
        }))
    }

    fn tool_commit_transaction(&mut self) -> anyhow::Result<Value> {
        let committed_commands = self.command_bus.commit_transaction()?;
        Ok(json!({
            "status": "committed",
            "committed_commands": committed_commands
        }))
    }

    fn tool_rollback_transaction(&mut self) -> anyhow::Result<Value> {
        let rolled_back_commands = self.command_bus.rollback_transaction()?;
        Ok(json!({
            "status": "rolled_back",
            "rolled_back_commands": rolled_back_commands
        }))
    }

    fn tool_create_checkpoint(&mut self, params: &Value) -> anyhow::Result<Value> {
        let label = required_string(params, "label")?;
        self.command_bus.transaction_checkpoint(label.clone())?;
        Ok(json!({
            "checkpoint": label,
            "status": "created"
        }))
    }

    fn tool_rollback_to_checkpoint(&mut self, params: &Value) -> anyhow::Result<Value> {
        let label = required_string(params, "label")?;
        let rolled_back_commands = self.command_bus.transaction_rollback_to(&label)?;
        Ok(json!({
            "checkpoint": label,
            "rolled_back_commands": rolled_back_commands
        }))
    }

    fn tool_log(&mut self, params: &Value) -> anyhow::Result<Value> {
        let message = required_string(params, "message")?;
        let level = optional_string(params, "level").unwrap_or_else(|| "info".to_string());
        let normalized_level = level.to_ascii_lowercase();
        match normalized_level.as_str() {
            "trace" => log::trace!("{message}"),
            "debug" => log::debug!("{message}"),
            "warn" | "warning" => log::warn!("{message}"),
            "error" => log::error!("{message}"),
            _ => log::info!("{message}"),
        }
        if matches!(normalized_level.as_str(), "warn" | "warning" | "error") {
            self.push_diagnostic(normalized_level, message.clone(), "tool.log");
        }
        Ok(json!({
            "message": message,
            "level": level
        }))
    }

    fn tool_open_task(&mut self, params: &Value) -> anyhow::Result<Value> {
        let title = required_string(params, "title")?;
        let task_id = format!("task-{}", self.next_task_id);
        self.next_task_id += 1;
        let state = TaskState {
            id: task_id.clone(),
            title,
            progress: 0.0,
            status: "open".to_string(),
        };
        self.tasks.insert(task_id.clone(), state.clone());
        Ok(json!({
            "task": state
        }))
    }

    fn tool_update_task(&mut self, params: &Value) -> anyhow::Result<Value> {
        let task_id = required_string(params, "task_id")?;
        let progress = optional_f32(params, "progress");
        let status = optional_string(params, "status");
        let task = self
            .tasks
            .get_mut(&task_id)
            .with_context(|| format!("task '{}' not found", task_id))?;
        if let Some(progress) = progress {
            task.progress = progress.clamp(0.0, 1.0);
        }
        if let Some(status) = status {
            task.status = status;
        }
        Ok(json!({
            "task": task
        }))
    }

    fn tool_close_task(&mut self, params: &Value) -> anyhow::Result<Value> {
        let task_id = required_string(params, "task_id")?;
        let status = required_string(params, "status")?;
        let task = self
            .tasks
            .get_mut(&task_id)
            .with_context(|| format!("task '{}' not found", task_id))?;
        task.status = status;
        task.progress = 1.0;
        Ok(json!({
            "task": task
        }))
    }

    fn scene_create(&mut self, params: &Value) -> anyhow::Result<Value> {
        let name = required_string(params, "name")?;
        let receipt = self
            .command_bus
            .submit(Box::new(SceneCreateCommand::new(name)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn scene_open(&mut self, params: &Value) -> anyhow::Result<Value> {
        let scene_id = required_string(params, "scene_id")?;
        let scene_path = normalize_scene_path(&scene_id);
        let receipt = self
            .command_bus
            .submit(Box::new(SceneOpenCommand::new(scene_path)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn scene_save(&mut self) -> anyhow::Result<Value> {
        let receipt = self
            .command_bus
            .submit(Box::new(SceneSaveCommand::new(None)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn scene_save_as(&mut self, params: &Value) -> anyhow::Result<Value> {
        let name = required_string(params, "name")?;
        let path = normalize_scene_path(&name);
        let receipt = self
            .command_bus
            .submit(Box::new(SceneSaveCommand::new(Some(path))))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn scene_duplicate(&mut self, params: &Value) -> anyhow::Result<Value> {
        let scene_id = required_string(params, "scene_id")?;
        let source_path = normalize_scene_path(&scene_id);
        let target_path = if let Some(name) = optional_string(params, "name") {
            normalize_scene_path(&name)
        } else {
            default_scene_duplicate_path(&source_path)
        };
        let target_scene_name = optional_string(params, "target_scene_name");
        let receipt = self
            .command_bus
            .submit(Box::new(SceneDuplicateCommand::new(
                source_path,
                target_path,
                target_scene_name,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn scene_close(&mut self) -> anyhow::Result<Value> {
        let receipt = self
            .command_bus
            .submit(Box::new(SceneCloseCommand::new()))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn scene_set_sky(&mut self, params: &Value) -> anyhow::Result<Value> {
        let preset = required_string(params, "preset")?;
        let receipt = self
            .command_bus
            .submit(Box::new(SceneSetSkyCommand::new(preset)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn scene_set_time_of_day(&mut self, params: &Value) -> anyhow::Result<Value> {
        let value = optional_f32(params, "value")
            .or_else(|| optional_f32(params, "time_of_day"))
            .with_context(|| "scene.set_time_of_day requires numeric 'value' in range [0,24]")?;
        let receipt = self
            .command_bus
            .submit(Box::new(SceneSetTimeOfDayCommand::new(value)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn scene_add_fog(&mut self, params: &Value) -> anyhow::Result<Value> {
        let current_fog = self.command_bus.context().scene_runtime.fog.clone();
        let density = optional_f32(params, "density")
            .or_else(|| current_fog.as_ref().map(|fog| fog.density))
            .unwrap_or(0.02);
        let color = optional_vec3(params, "color")?
            .or_else(|| current_fog.as_ref().map(|fog| fog.color))
            .unwrap_or([0.72, 0.76, 0.84]);
        let start = optional_f32(params, "start")
            .or_else(|| current_fog.as_ref().map(|fog| fog.start))
            .unwrap_or(5.0);
        let end = optional_f32(params, "end")
            .or_else(|| current_fog.as_ref().map(|fog| fog.end))
            .unwrap_or(80.0);
        let receipt = self.command_bus.submit(Box::new(SceneAddFogCommand::new(
            density, color, start, end,
        )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn scene_enable_world_streaming(&mut self, params: &Value) -> anyhow::Result<Value> {
        let chunk_size = optional_f32(params, "chunksize")
            .or_else(|| optional_f32(params, "chunk_size"))
            .unwrap_or(64.0);
        let range = optional_usize(params, "range").unwrap_or(4).max(1);
        let receipt = self
            .command_bus
            .submit(Box::new(SceneEnableWorldStreamingCommand::new(
                chunk_size,
                range as u32,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn scene_create_stream_chunk(&mut self, params: &Value) -> anyhow::Result<Value> {
        let default_chunk_id = format!(
            "chunk_{}",
            self.command_bus
                .context()
                .scene_runtime
                .world_streaming
                .chunks
                .len()
                + 1
        );
        let chunk_id = optional_string(params, "chunk_id").unwrap_or(default_chunk_id);
        let center = optional_vec3(params, "center")?
            .or(optional_vec3(params, "position")?)
            .unwrap_or([0.0, 0.0, 0.0]);
        let default_radius = (self
            .command_bus
            .context()
            .scene_runtime
            .world_streaming
            .chunk_size
            * 0.5)
            .max(1.0);
        let radius = optional_f32(params, "radius").unwrap_or(default_radius);
        let receipt = self
            .command_bus
            .submit(Box::new(SceneCreateStreamChunkCommand::new(
                chunk_id, center, radius,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn scene_assign_entity_to_chunk(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let chunk_id = required_string(params, "chunk_id")?;
        let receipt = self
            .command_bus
            .submit(Box::new(SceneAssignEntityToChunkCommand::new(
                entity_id, chunk_id,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn entity_create(&mut self, params: &Value) -> anyhow::Result<Value> {
        let name = required_string(params, "name")?;
        let mesh = optional_string(params, "mesh").unwrap_or_else(|| "cube".to_string());
        let translation = optional_vec3(params, "translation")?.unwrap_or([0.0, 0.0, 0.0]);
        let receipt =
            self.command_bus
                .submit(Box::new(EntityCreateCommand::new(name, mesh, translation)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn entity_clone(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let name = optional_string(params, "name");
        let translation_offset =
            optional_vec3(params, "translation_offset")?.unwrap_or([0.0, 0.0, 0.0]);
        let copy_components = optional_bool(params, "copy_components").unwrap_or(true);
        let copy_parent = optional_bool(params, "copy_parent").unwrap_or(false);
        let receipt = self.command_bus.submit(Box::new(EntityCloneCommand::new(
            entity_id,
            name,
            translation_offset,
            copy_components,
            copy_parent,
        )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn entity_delete(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let receipt = self
            .command_bus
            .submit(Box::new(EntityDeleteCommand::new(entity_id)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn entity_rename(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let name = required_string(params, "name")?;
        let receipt = self
            .command_bus
            .submit(Box::new(EntityRenameCommand::new(entity_id, name)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn entity_parent(&mut self, params: &Value) -> anyhow::Result<Value> {
        let child_id = required_string(params, "child_id")
            .or_else(|_| required_string(params, "entity_id"))?;
        let parent_id = required_string(params, "parent_id")?;
        let receipt = self
            .command_bus
            .submit(Box::new(EntityParentCommand::new(child_id, parent_id)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn entity_unparent(&mut self, params: &Value) -> anyhow::Result<Value> {
        let child_id = required_string(params, "child_id")
            .or_else(|_| required_string(params, "entity_id"))?;
        let receipt = self
            .command_bus
            .submit(Box::new(EntityUnparentCommand::new(child_id)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn entity_find_by_name(&self, params: &Value) -> anyhow::Result<Value> {
        let query = optional_string(params, "name")
            .or_else(|| optional_string(params, "query"))
            .unwrap_or_default();
        let exact = optional_bool(params, "exact").unwrap_or(false);
        let query_lower = query.trim().to_ascii_lowercase();
        let matches = self
            .command_bus
            .context()
            .scene
            .entities
            .iter()
            .filter(|entity| {
                let name = entity.name.as_str();
                if query_lower.is_empty() {
                    return true;
                }
                if exact {
                    name.eq_ignore_ascii_case(&query_lower)
                } else {
                    name.to_ascii_lowercase().contains(&query_lower)
                }
            })
            .map(|entity| entity.name.clone())
            .collect::<Vec<String>>();
        Ok(json!({
            "query": query,
            "exact": exact,
            "count": matches.len(),
            "entity_ids": matches
        }))
    }

    fn entity_find_by_tag(&self, params: &Value) -> anyhow::Result<Value> {
        let tag = required_string(params, "tag")?;
        let tag_normalized = tag.trim().to_ascii_lowercase();
        let mut matches = Vec::<String>::new();
        for entity in &self.command_bus.context().scene.entities {
            let Some(bucket) = self.command_bus.context().components.get(&entity.name) else {
                continue;
            };
            let mut entity_tags = Vec::<String>::new();
            if let Some(value) = bucket.get("Tag") {
                if let Some(tag_value) = value.as_str() {
                    entity_tags.push(tag_value.to_string());
                } else if let Some(obj) = value.as_object()
                    && let Some(tag_value) = obj.get("value").and_then(Value::as_str)
                {
                    entity_tags.push(tag_value.to_string());
                }
            }
            if let Some(value) = bucket.get("Tags") {
                if let Some(items) = value.as_array() {
                    for item in items {
                        if let Some(tag_value) = item.as_str() {
                            entity_tags.push(tag_value.to_string());
                        }
                    }
                } else if let Some(obj) = value.as_object()
                    && let Some(items) = obj.get("values").and_then(Value::as_array)
                {
                    for item in items {
                        if let Some(tag_value) = item.as_str() {
                            entity_tags.push(tag_value.to_string());
                        }
                    }
                }
            }
            let has_tag = entity_tags
                .iter()
                .any(|candidate| candidate.trim().eq_ignore_ascii_case(&tag_normalized));
            if has_tag {
                matches.push(entity.name.clone());
            }
        }
        Ok(json!({
            "tag": tag,
            "count": matches.len(),
            "entity_ids": matches
        }))
    }

    fn entity_set_transform(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let translation = optional_vec3(params, "translation")?
            .or(optional_vec3(params, "pos")?)
            .context("entity.set_transform requires 'translation' or 'pos' [x,y,z]")?;
        let receipt = self
            .command_bus
            .submit(Box::new(EntitySetTransformCommand::new(
                entity_id,
                translation,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn entity_translate(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let delta = optional_vec3(params, "delta")?
            .or(optional_vec3(params, "translation")?)
            .or(optional_vec3(params, "offset")?)
            .context("entity.translate requires 'delta' [x,y,z]")?;
        let receipt = self
            .command_bus
            .submit(Box::new(EntityTranslateCommand::new(entity_id, delta)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn entity_rotate(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let delta = optional_vec3(params, "delta")?
            .or(optional_vec3(params, "rotation")?)
            .context("entity.rotate requires 'delta' [x,y,z]")?;
        let receipt = self
            .command_bus
            .submit(Box::new(EntityRotateCommand::new(entity_id, delta)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn entity_scale(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let factor = if let Some(factor_scalar) =
            optional_f32(params, "factor").or_else(|| optional_f32(params, "value"))
        {
            [factor_scalar, factor_scalar, factor_scalar]
        } else {
            optional_vec3(params, "factor")?
                .or(optional_vec3(params, "scale")?)
                .context("entity.scale requires numeric 'factor' or vec3 'factor'/'scale'")?
        };
        let receipt = self
            .command_bus
            .submit(Box::new(EntityScaleCommand::new(entity_id, factor)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn entity_get_transform(&self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let components = self
            .command_bus
            .context()
            .components
            .get(&entity_id)
            .cloned()
            .unwrap_or_default();
        let rotation = components
            .get("TransformRotation")
            .and_then(|value| value.as_array())
            .and_then(|arr| {
                if arr.len() == 3 {
                    Some([
                        arr[0].as_f64()? as f32,
                        arr[1].as_f64()? as f32,
                        arr[2].as_f64()? as f32,
                    ])
                } else {
                    None
                }
            })
            .unwrap_or([0.0, 0.0, 0.0]);
        let scale = components
            .get("TransformScale")
            .and_then(|value| value.as_array())
            .and_then(|arr| {
                if arr.len() == 3 {
                    Some([
                        arr[0].as_f64()? as f32,
                        arr[1].as_f64()? as f32,
                        arr[2].as_f64()? as f32,
                    ])
                } else {
                    None
                }
            })
            .unwrap_or([1.0, 1.0, 1.0]);
        let translation = self
            .command_bus
            .context()
            .entity_transform(&entity_id)
            .with_context(|| format!("entity '{}' not found", entity_id))?;
        Ok(json!({
            "entity_id": entity_id,
            "translation": translation,
            "rotation": rotation,
            "scale": scale
        }))
    }

    fn entity_add_component(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let component_type = required_string(params, "component_type")?;
        let data = required_value(params, "data")?.clone();
        let receipt = self
            .command_bus
            .submit(Box::new(EntityAddComponentCommand::new(
                entity_id,
                component_type,
                data,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn entity_remove_component(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let component_type = required_string(params, "component_type")?;
        let receipt = self
            .command_bus
            .submit(Box::new(EntityRemoveComponentCommand::new(
                entity_id,
                component_type,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn entity_get_component(&self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let component_type = required_string(params, "component_type")?;
        let data = self
            .command_bus
            .context()
            .components
            .get(&entity_id)
            .and_then(|bucket| bucket.get(&component_type))
            .cloned();
        Ok(json!({
            "entity_id": entity_id,
            "component_type": component_type,
            "exists": data.is_some(),
            "data": data
        }))
    }

    fn entity_set_component(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let component_type = required_string(params, "component_type")?;
        let data = required_value(params, "data")?.clone();
        let receipt = self
            .command_bus
            .submit(Box::new(EntitySetComponentCommand::new(
                entity_id,
                component_type,
                data,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn phys_add_collider(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let shape = optional_string(params, "shape").unwrap_or_else(|| "box".to_string());
        let size = optional_vec3(params, "size")?.unwrap_or([1.0, 1.0, 1.0]);
        let is_trigger = optional_bool(params, "is_trigger").unwrap_or(false);

        let receipt = self
            .command_bus
            .submit(Box::new(PhysAddColliderCommand::new(
                entity_id, shape, size, is_trigger,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn phys_set_collider(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let existing = self
            .command_bus
            .context()
            .physics
            .colliders
            .get(&entity_id)
            .cloned();
        let shape = optional_string(params, "shape")
            .or_else(|| existing.as_ref().map(|collider| collider.shape.clone()))
            .unwrap_or_else(|| "box".to_string());
        let size = optional_vec3(params, "size")?
            .or_else(|| existing.as_ref().map(|collider| collider.size))
            .unwrap_or([1.0, 1.0, 1.0]);
        let is_trigger = optional_bool(params, "is_trigger")
            .or_else(|| existing.as_ref().map(|collider| collider.is_trigger))
            .unwrap_or(false);
        let receipt = self
            .command_bus
            .submit(Box::new(PhysAddColliderCommand::new(
                entity_id, shape, size, is_trigger,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn phys_remove_collider(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let receipt = self
            .command_bus
            .submit(Box::new(PhysRemoveColliderCommand::new(entity_id)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn phys_add_rigidbody(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let body_type = optional_string(params, "type")
            .or_else(|| optional_string(params, "body_type"))
            .unwrap_or_else(|| "dynamic".to_string());
        let mass = optional_f32(params, "mass").unwrap_or(1.0);
        let friction = optional_f32(params, "friction").unwrap_or(0.5);
        let restitution = optional_f32(params, "restitution").unwrap_or(0.0);

        let receipt = self
            .command_bus
            .submit(Box::new(PhysAddRigidbodyCommand::new(
                entity_id,
                body_type,
                mass,
                friction,
                restitution,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn phys_set_mass(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let mass = optional_f32(params, "mass")
            .or_else(|| optional_f32(params, "value"))
            .with_context(|| "phys.set_mass requires numeric 'mass' or 'value'")?;
        let receipt = self
            .command_bus
            .submit(Box::new(PhysSetRigidbodyParamsCommand::new(
                entity_id,
                Some(mass),
                None,
                None,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn phys_set_friction(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let friction = optional_f32(params, "friction")
            .or_else(|| optional_f32(params, "value"))
            .with_context(|| "phys.set_friction requires numeric 'friction' or 'value'")?;
        let receipt = self
            .command_bus
            .submit(Box::new(PhysSetRigidbodyParamsCommand::new(
                entity_id,
                None,
                Some(friction),
                None,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn phys_set_restitution(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let restitution = optional_f32(params, "restitution")
            .or_else(|| optional_f32(params, "value"))
            .with_context(|| "phys.set_restitution requires numeric 'restitution' or 'value'")?;
        let receipt = self
            .command_bus
            .submit(Box::new(PhysSetRigidbodyParamsCommand::new(
                entity_id,
                None,
                None,
                Some(restitution),
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn phys_apply_force(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let force = optional_vec3(params, "force")?
            .or(optional_vec3(params, "impulse")?)
            .context("phys.apply_force requires 'force' [x,y,z]")?;
        let dt = optional_f32(params, "dt").unwrap_or(1.0 / 60.0);
        let receipt = self
            .command_bus
            .submit(Box::new(PhysApplyForceCommand::new(entity_id, force, dt)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn phys_apply_impulse(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let impulse = optional_vec3(params, "impulse")?
            .or(optional_vec3(params, "force")?)
            .context("phys.apply_impulse requires 'impulse' [x,y,z]")?;

        let receipt = self
            .command_bus
            .submit(Box::new(PhysApplyImpulseCommand::new(entity_id, impulse)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn phys_set_gravity(&mut self, params: &Value) -> anyhow::Result<Value> {
        let gravity = optional_vec3(params, "gravity")?
            .or(optional_vec3(params, "value")?)
            .context("phys.set_gravity requires 'gravity' [x,y,z]")?;

        let receipt = self
            .command_bus
            .submit(Box::new(PhysSetGravityCommand::new(gravity)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn phys_raycast(&self, params: &Value) -> anyhow::Result<Value> {
        let origin =
            optional_vec3(params, "origin")?.context("phys.raycast requires 'origin' [x,y,z]")?;
        let dir = optional_vec3(params, "dir")?
            .or(optional_vec3(params, "direction")?)
            .context("phys.raycast requires 'dir' [x,y,z]")?;
        let normalized_dir =
            normalize_vec3(dir).with_context(|| "phys.raycast requires non-zero direction")?;
        let maxdist = optional_f32(params, "maxdist")
            .or_else(|| optional_f32(params, "max_distance"))
            .unwrap_or(100.0)
            .max(0.0);

        let context = self.command_bus.context();
        let mut hits = Vec::<Value>::new();
        for (entity_id, collider) in &context.physics.colliders {
            let Some(center) = context.entity_transform(entity_id) else {
                continue;
            };
            let half_extents = [
                collider.size[0].abs() * 0.5,
                collider.size[1].abs() * 0.5,
                collider.size[2].abs() * 0.5,
            ];
            let Some(distance) =
                ray_aabb_hit_distance(origin, normalized_dir, center, half_extents)
            else {
                continue;
            };
            if distance > maxdist {
                continue;
            }
            let point = [
                origin[0] + normalized_dir[0] * distance,
                origin[1] + normalized_dir[1] * distance,
                origin[2] + normalized_dir[2] * distance,
            ];
            hits.push(json!({
                "entity_id": entity_id,
                "distance": distance,
                "point": point,
                "shape": collider.shape,
                "is_trigger": collider.is_trigger
            }));
        }
        hits.sort_by(|left, right| {
            let left_distance = left
                .get("distance")
                .and_then(Value::as_f64)
                .unwrap_or(f64::INFINITY);
            let right_distance = right
                .get("distance")
                .and_then(Value::as_f64)
                .unwrap_or(f64::INFINITY);
            left_distance
                .partial_cmp(&right_distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(json!({
            "origin": origin,
            "dir": normalized_dir,
            "maxdist": maxdist,
            "hit": !hits.is_empty(),
            "hit_count": hits.len(),
            "closest_hit": hits.first().cloned(),
            "hits": hits
        }))
    }

    fn phys_overlap(&self, params: &Value) -> anyhow::Result<Value> {
        let shape = optional_string(params, "shape").unwrap_or_else(|| "box".to_string());
        let center =
            optional_vec3(params, "center")?.context("phys.overlap requires 'center' [x,y,z]")?;
        let context = self.command_bus.context();
        let mut overlaps = Vec::<Value>::new();

        if shape.eq_ignore_ascii_case("sphere") {
            let radius = optional_f32(params, "radius").unwrap_or(1.0).max(0.01);
            for (entity_id, collider) in &context.physics.colliders {
                let Some(collider_center) = context.entity_transform(entity_id) else {
                    continue;
                };
                let half = [
                    collider.size[0].abs() * 0.5,
                    collider.size[1].abs() * 0.5,
                    collider.size[2].abs() * 0.5,
                ];
                if sphere_aabb_intersects(center, radius, collider_center, half) {
                    overlaps.push(json!({
                        "entity_id": entity_id,
                        "shape": collider.shape,
                        "is_trigger": collider.is_trigger
                    }));
                }
            }
            return Ok(json!({
                "shape": "sphere",
                "center": center,
                "radius": radius,
                "count": overlaps.len(),
                "overlaps": overlaps
            }));
        }

        let size = optional_vec3(params, "size")?.unwrap_or([1.0, 1.0, 1.0]);
        let query_half = [
            size[0].abs() * 0.5,
            size[1].abs() * 0.5,
            size[2].abs() * 0.5,
        ];
        for (entity_id, collider) in &context.physics.colliders {
            let Some(collider_center) = context.entity_transform(entity_id) else {
                continue;
            };
            let collider_half = [
                collider.size[0].abs() * 0.5,
                collider.size[1].abs() * 0.5,
                collider.size[2].abs() * 0.5,
            ];
            if aabb_overlap(center, query_half, collider_center, collider_half) {
                overlaps.push(json!({
                    "entity_id": entity_id,
                    "shape": collider.shape,
                    "is_trigger": collider.is_trigger
                }));
            }
        }
        Ok(json!({
            "shape": "box",
            "center": center,
            "size": size,
            "count": overlaps.len(),
            "overlaps": overlaps
        }))
    }

    fn phys_add_character_controller(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let radius = optional_f32(params, "radius").unwrap_or(0.4);
        let height = optional_f32(params, "height").unwrap_or(1.8);
        let speed = optional_f32(params, "speed").unwrap_or(5.0);
        let jump_strength = optional_f32(params, "jump_strength").unwrap_or(6.0);
        let receipt = self
            .command_bus
            .submit(Box::new(PhysAddCharacterControllerCommand::new(
                entity_id,
                radius,
                height,
                speed,
                jump_strength,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn phys_character_move(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let input = optional_vec3(params, "input")?
            .or(optional_vec3(params, "move")?)
            .context("phys.character_move requires 'input' [x,y,z]")?;
        let dt = optional_f32(params, "dt").unwrap_or(1.0 / 60.0);
        let receipt = self
            .command_bus
            .submit(Box::new(PhysCharacterMoveCommand::new(
                entity_id, input, dt,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn phys_character_jump(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let strength = optional_f32(params, "strength");
        let receipt = self
            .command_bus
            .submit(Box::new(PhysCharacterJumpCommand::new(entity_id, strength)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn phys_character_set_state(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let state = required_string(params, "state")?;
        let receipt = self
            .command_bus
            .submit(Box::new(PhysCharacterSetStateCommand::new(
                entity_id, state,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn game_create_input_action(&mut self, params: &Value) -> anyhow::Result<Value> {
        let name = required_string(params, "name")?;
        let bindings = required_string_array(params, "bindings")?;
        let receipt = self
            .command_bus
            .submit(Box::new(GameCreateInputActionCommand::new(name, bindings)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn game_bind_action(&mut self, params: &Value) -> anyhow::Result<Value> {
        let name = optional_string(params, "name")
            .or_else(|| optional_string(params, "action"))
            .with_context(|| "game.bind_action requires 'name' or 'action'")?;
        let target_script_event = required_string(params, "target_script_event")?;
        let receipt = self
            .command_bus
            .submit(Box::new(GameBindActionCommand::new(
                name,
                target_script_event,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn game_set_rebind(&mut self, params: &Value) -> anyhow::Result<Value> {
        let action = optional_string(params, "action")
            .or_else(|| optional_string(params, "name"))
            .with_context(|| "game.set_rebind requires 'action' or 'name'")?;
        let binding = required_string(params, "binding")?;
        let receipt = self
            .command_bus
            .submit(Box::new(GameSetRebindCommand::new(action, binding)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn game_create_weapon(&mut self, params: &Value) -> anyhow::Result<Value> {
        let weapon_id = required_string(params, "weapon_id")?;
        let rate = optional_f32(params, "rate").unwrap_or(8.0);
        let recoil = optional_f32(params, "recoil").unwrap_or(1.0);
        let spread = optional_f32(params, "spread").unwrap_or(0.08);
        let ammo_capacity = optional_usize(params, "ammo_capacity")
            .or_else(|| optional_usize(params, "ammo"))
            .unwrap_or(30)
            .min(u32::MAX as usize) as u32;

        let receipt = self
            .command_bus
            .submit(Box::new(GameCreateWeaponCommand::new(
                weapon_id,
                rate,
                recoil,
                spread,
                ammo_capacity,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn game_attach_weapon(&mut self, params: &Value) -> anyhow::Result<Value> {
        let character_id = optional_string(params, "character_id")
            .or_else(|| optional_string(params, "entity_id"))
            .with_context(|| "game.attach_weapon requires 'character_id' or 'entity_id'")?;
        let weapon_id = required_string(params, "weapon_id")?;

        let receipt = self
            .command_bus
            .submit(Box::new(GameAttachWeaponCommand::new(
                character_id,
                weapon_id,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn game_fire_weapon(&mut self, params: &Value) -> anyhow::Result<Value> {
        let character_id = optional_string(params, "character_id")
            .or_else(|| optional_string(params, "entity_id"));
        let weapon_id = if let Some(weapon_id) = optional_string(params, "weapon_id") {
            weapon_id
        } else if let Some(character_id) = character_id.clone() {
            self.command_bus
                .context()
                .gameplay
                .attachments
                .get(&character_id)
                .cloned()
                .with_context(|| format!("character '{}' has no weapon attachment", character_id))?
        } else {
            bail!("game.fire_weapon requires 'weapon_id' or 'character_id'");
        };

        let receipt = self
            .command_bus
            .submit(Box::new(GameFireWeaponCommand::new(weapon_id.clone())))?;
        let mut result = command_receipt_to_json(receipt);
        if let Some(character_id) = character_id {
            result["character_id"] = Value::String(character_id);
        }
        result["weapon_id"] = Value::String(weapon_id);
        Ok(result)
    }

    fn game_apply_damage(&mut self, params: &Value) -> anyhow::Result<Value> {
        let target_id = required_string(params, "target_id")?;
        let amount = params
            .get("amount")
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .with_context(|| "game.apply_damage requires numeric 'amount'")?;
        let damage_type =
            optional_string(params, "damage_type").unwrap_or_else(|| "generic".to_string());

        let receipt = self
            .command_bus
            .submit(Box::new(GameApplyDamageCommand::new(
                target_id,
                amount,
                damage_type,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn game_add_health_component(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let max_health = optional_f32(params, "max_health").unwrap_or(100.0);
        let current_health = optional_f32(params, "current_health").unwrap_or(max_health);

        let receipt = self
            .command_bus
            .submit(Box::new(GameAddHealthComponentCommand::new(
                entity_id,
                max_health,
                current_health,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn game_add_trigger(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let shape = optional_string(params, "shape").unwrap_or_else(|| "sphere".to_string());
        let radius = optional_f32(params, "radius").unwrap_or(1.5);
        let data = params.get("params").cloned().unwrap_or_else(|| json!({}));
        let receipt = self
            .command_bus
            .submit(Box::new(GameAddTriggerCommand::new(
                entity_id, shape, radius, data,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn game_add_pickup(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let item_data = required_value(params, "item_data")?.clone();
        let receipt = self
            .command_bus
            .submit(Box::new(GameAddPickupCommand::new(entity_id, item_data)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn game_add_inventory(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let capacity = optional_usize(params, "capacity")
            .unwrap_or(8)
            .min(u32::MAX as usize) as u32;
        let items = optional_string_array(params, "items");
        let receipt = self
            .command_bus
            .submit(Box::new(GameAddInventoryCommand::new(
                entity_id, capacity, items,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn game_add_interactable(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let prompt = required_string(params, "prompt")?;
        let actions = optional_string_array(params, "actions");
        let receipt = self
            .command_bus
            .submit(Box::new(GameAddInteractableCommand::new(
                entity_id, prompt, actions,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn anim_mutation(&mut self, operation: &str, params: Value) -> anyhow::Result<Value> {
        let receipt = self
            .command_bus
            .submit(Box::new(AnimMutationCommand::new(operation, params)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn anim_add_animator(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let controller_id = required_string(params, "controller_id")?;
        self.anim_mutation(
            "add_animator",
            json!({
                "entity_id": entity_id,
                "controller_id": controller_id
            }),
        )
    }

    fn anim_create_state_machine(&mut self, params: &Value) -> anyhow::Result<Value> {
        let name = required_string(params, "name")?;
        let controller_id = optional_string(params, "controller_id");
        self.anim_mutation(
            "create_state_machine",
            json!({
                "name": name,
                "controller_id": controller_id
            }),
        )
    }

    fn anim_add_state(&mut self, params: &Value) -> anyhow::Result<Value> {
        let controller_id = required_string(params, "controller_id")?;
        let state_name = required_string(params, "state_name")?;
        let clip_id = required_string(params, "clip_id")?;
        self.anim_mutation(
            "add_state",
            json!({
                "controller_id": controller_id,
                "state_name": state_name,
                "clip_id": clip_id
            }),
        )
    }

    fn anim_add_transition(&mut self, params: &Value) -> anyhow::Result<Value> {
        let controller_id = required_string(params, "controller_id")?;
        let from = required_string(params, "from")?;
        let to = required_string(params, "to")?;
        let conditions = params
            .get("conditions")
            .cloned()
            .unwrap_or_else(|| json!({}));
        self.anim_mutation(
            "add_transition",
            json!({
                "controller_id": controller_id,
                "from": from,
                "to": to,
                "conditions": conditions
            }),
        )
    }

    fn anim_set_parameter(&mut self, params: &Value) -> anyhow::Result<Value> {
        let controller_id = required_string(params, "controller_id")?;
        let key = required_string(params, "key")?;
        let value = required_value(params, "value")?.clone();
        self.anim_mutation(
            "set_parameter",
            json!({
                "controller_id": controller_id,
                "key": key,
                "value": value
            }),
        )
    }

    fn anim_play(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let clip_id = required_string(params, "clip_id")?;
        self.anim_mutation(
            "play",
            json!({
                "entity_id": entity_id,
                "clip_id": clip_id
            }),
        )
    }

    fn anim_blend(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let clip_a = required_string(params, "clip_a")?;
        let clip_b = required_string(params, "clip_b")?;
        let weight = optional_f32(params, "weight").unwrap_or(0.5);
        self.anim_mutation(
            "blend",
            json!({
                "entity_id": entity_id,
                "clip_a": clip_a,
                "clip_b": clip_b,
                "weight": weight
            }),
        )
    }

    fn anim_add_ik(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let chain = required_string(params, "chain")?;
        let ik_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.anim_mutation(
            "add_ik",
            json!({
                "entity_id": entity_id,
                "chain": chain,
                "params": ik_params
            }),
        )
    }

    fn anim_retarget(&mut self, params: &Value) -> anyhow::Result<Value> {
        let source_rig = required_string(params, "source_rig")?;
        let target_rig = required_string(params, "target_rig")?;
        let mapping = params.get("mapping").cloned().unwrap_or_else(|| json!({}));
        self.anim_mutation(
            "retarget",
            json!({
                "source_rig": source_rig,
                "target_rig": target_rig,
                "mapping": mapping
            }),
        )
    }

    fn anim_bake_animation(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let bake_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.anim_mutation(
            "bake_animation",
            json!({
                "entity_id": entity_id,
                "params": bake_params
            }),
        )
    }

    fn model_mutation(&mut self, operation: &str, params: Value) -> anyhow::Result<Value> {
        let receipt = self
            .command_bus
            .submit(Box::new(ModelMutationCommand::new(operation, params)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn model_create_primitive(&mut self, params: &Value) -> anyhow::Result<Value> {
        let primitive_type = required_string(params, "type")?;
        let name = required_string(params, "name")?;
        let mesh_id = optional_string(params, "mesh_id");
        let translation = optional_vec3(params, "translation")?;
        self.model_mutation(
            "create_primitive",
            json!({
                "type": primitive_type,
                "name": name,
                "mesh_id": mesh_id,
                "translation": translation
            }),
        )
    }

    fn model_enter_edit_mode(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mesh_id = required_string(params, "mesh_id")?;
        self.model_mutation("enter_edit_mode", json!({ "mesh_id": mesh_id }))
    }

    fn model_exit_edit_mode(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mesh_id = required_string(params, "mesh_id")?;
        self.model_mutation("exit_edit_mode", json!({ "mesh_id": mesh_id }))
    }

    fn model_select(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mesh_id = required_string(params, "mesh_id")?;
        let mode = required_string(params, "mode")?;
        let selector = params.get("selector").cloned().unwrap_or(Value::Null);
        self.model_mutation(
            "select",
            json!({
                "mesh_id": mesh_id,
                "mode": mode,
                "selector": selector
            }),
        )
    }

    fn model_mesh_op(&mut self, operation: &str, params: &Value) -> anyhow::Result<Value> {
        let mesh_id = required_string(params, "mesh_id")?;
        let mut payload = json!({ "mesh_id": mesh_id });
        if let Some(params_obj) = params.get("params") {
            payload["params"] = params_obj.clone();
        }
        for key in ["resolution", "ratio", "iterations", "path"] {
            if let Some(value) = params.get(key) {
                payload[key] = value.clone();
            }
        }
        self.model_mutation(operation, payload)
    }

    fn model_extrude(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.model_mesh_op("extrude", params)
    }

    fn model_inset(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.model_mesh_op("inset", params)
    }

    fn model_bevel(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.model_mesh_op("bevel", params)
    }

    fn model_loop_cut(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.model_mesh_op("loop_cut", params)
    }

    fn model_knife(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.model_mesh_op("knife", params)
    }

    fn model_merge(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.model_mesh_op("merge", params)
    }

    fn model_subdivide(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.model_mesh_op("subdivide", params)
    }

    fn model_triangulate(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.model_mesh_op("triangulate", params)
    }

    fn model_add_modifier(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mesh_id = required_string(params, "mesh_id")?;
        let modifier_type = required_string(params, "type")?;
        let modifier_id = optional_string(params, "modifier_id");
        let modifier_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.model_mutation(
            "add_modifier",
            json!({
                "mesh_id": mesh_id,
                "type": modifier_type,
                "modifier_id": modifier_id,
                "params": modifier_params
            }),
        )
    }

    fn model_set_modifier(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mesh_id = required_string(params, "mesh_id")?;
        let modifier_id = required_string(params, "modifier_id")?;
        let modifier_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.model_mutation(
            "set_modifier",
            json!({
                "mesh_id": mesh_id,
                "modifier_id": modifier_id,
                "params": modifier_params
            }),
        )
    }

    fn model_apply_modifier(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mesh_id = required_string(params, "mesh_id")?;
        let modifier_id = required_string(params, "modifier_id")?;
        self.model_mutation(
            "apply_modifier",
            json!({
                "mesh_id": mesh_id,
                "modifier_id": modifier_id
            }),
        )
    }

    fn model_remove_modifier(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mesh_id = required_string(params, "mesh_id")?;
        let modifier_id = required_string(params, "modifier_id")?;
        self.model_mutation(
            "remove_modifier",
            json!({
                "mesh_id": mesh_id,
                "modifier_id": modifier_id
            }),
        )
    }

    fn model_unwrap_uv(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mesh_id = required_string(params, "mesh_id")?;
        let method = optional_string(params, "method");
        let uv_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.model_mutation(
            "unwrap_uv",
            json!({
                "mesh_id": mesh_id,
                "method": method,
                "params": uv_params
            }),
        )
    }

    fn model_pack_uv(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mesh_id = required_string(params, "mesh_id")?;
        let uv_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.model_mutation(
            "pack_uv",
            json!({
                "mesh_id": mesh_id,
                "params": uv_params
            }),
        )
    }

    fn model_generate_lightmap_uv(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mesh_id = required_string(params, "mesh_id")?;
        let uv_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.model_mutation(
            "generate_lightmap_uv",
            json!({
                "mesh_id": mesh_id,
                "params": uv_params
            }),
        )
    }

    fn model_voxel_remesh(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.model_mesh_op("voxel_remesh", params)
    }

    fn model_decimate(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.model_mesh_op("decimate", params)
    }

    fn model_smooth(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.model_mesh_op("smooth", params)
    }

    fn model_sculpt_brush(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mesh_id = required_string(params, "mesh_id")?;
        let brush_type = required_string(params, "brush_type")?;
        let sculpt_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.model_mutation(
            "sculpt_brush",
            json!({
                "mesh_id": mesh_id,
                "brush_type": brush_type,
                "params": sculpt_params
            }),
        )
    }

    fn model_sculpt_mask(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mesh_id = required_string(params, "mesh_id")?;
        let sculpt_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.model_mutation(
            "sculpt_mask",
            json!({
                "mesh_id": mesh_id,
                "params": sculpt_params
            }),
        )
    }

    fn vfx_mutation(&mut self, operation: &str, params: Value) -> anyhow::Result<Value> {
        let receipt = self
            .command_bus
            .submit(Box::new(VfxMutationCommand::new(operation, params)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn vfx_create_particle_system(&mut self, params: &Value) -> anyhow::Result<Value> {
        let name = required_string(params, "name")?;
        let particle_id = optional_string(params, "particle_id");
        let particle_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.vfx_mutation(
            "create_particle_system",
            json!({
                "name": name,
                "particle_id": particle_id,
                "params": particle_params
            }),
        )
    }

    fn vfx_set_emitter(&mut self, params: &Value) -> anyhow::Result<Value> {
        let particle_id = required_string(params, "particle_id")?;
        let emitter_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.vfx_mutation(
            "set_emitter",
            json!({
                "particle_id": particle_id,
                "params": emitter_params
            }),
        )
    }

    fn vfx_set_forces(&mut self, params: &Value) -> anyhow::Result<Value> {
        let particle_id = required_string(params, "particle_id")?;
        let force_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.vfx_mutation(
            "set_forces",
            json!({
                "particle_id": particle_id,
                "params": force_params
            }),
        )
    }

    fn vfx_set_collision(&mut self, params: &Value) -> anyhow::Result<Value> {
        let particle_id = required_string(params, "particle_id")?;
        let collision_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.vfx_mutation(
            "set_collision",
            json!({
                "particle_id": particle_id,
                "params": collision_params
            }),
        )
    }

    fn vfx_set_renderer(&mut self, params: &Value) -> anyhow::Result<Value> {
        let particle_id = required_string(params, "particle_id")?;
        let renderer_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.vfx_mutation(
            "set_renderer",
            json!({
                "particle_id": particle_id,
                "params": renderer_params
            }),
        )
    }

    fn vfx_attach_to_entity(&mut self, params: &Value) -> anyhow::Result<Value> {
        let particle_id = required_string(params, "particle_id")?;
        let entity_id = required_string(params, "entity_id")?;
        let socket = optional_string(params, "socket");
        self.vfx_mutation(
            "attach_to_entity",
            json!({
                "particle_id": particle_id,
                "entity_id": entity_id,
                "socket": socket
            }),
        )
    }

    fn vfx_create_graph(&mut self, params: &Value) -> anyhow::Result<Value> {
        let name = required_string(params, "name")?;
        let graph_id = optional_string(params, "graph_id");
        self.vfx_mutation(
            "create_graph",
            json!({
                "name": name,
                "graph_id": graph_id
            }),
        )
    }

    fn vfx_add_node(&mut self, params: &Value) -> anyhow::Result<Value> {
        let graph_id = required_string(params, "graph_id")?;
        let node_type = required_string(params, "node_type")?;
        let node_id = optional_string(params, "node_id");
        let node_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.vfx_mutation(
            "add_node",
            json!({
                "graph_id": graph_id,
                "node_type": node_type,
                "node_id": node_id,
                "params": node_params
            }),
        )
    }

    fn vfx_connect(&mut self, params: &Value) -> anyhow::Result<Value> {
        let graph_id = required_string(params, "graph_id")?;
        let out_node = required_string(params, "out_node")?;
        let in_node = required_string(params, "in_node")?;
        self.vfx_mutation(
            "connect",
            json!({
                "graph_id": graph_id,
                "out_node": out_node,
                "in_node": in_node
            }),
        )
    }

    fn vfx_compile_graph(&mut self, params: &Value) -> anyhow::Result<Value> {
        let graph_id = required_string(params, "graph_id")?;
        self.vfx_mutation("compile_graph", json!({ "graph_id": graph_id }))
    }

    fn water_mutation(&mut self, operation: &str, params: Value) -> anyhow::Result<Value> {
        let receipt = self
            .command_bus
            .submit(Box::new(WaterMutationCommand::new(operation, params)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn water_create_ocean(&mut self, params: &Value) -> anyhow::Result<Value> {
        let ocean_id = optional_string(params, "ocean_id");
        let size = optional_f32(params, "size");
        let waves = params.get("waves").cloned().unwrap_or_else(|| json!({}));
        let water_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.water_mutation(
            "create_ocean",
            json!({
                "ocean_id": ocean_id,
                "size": size,
                "waves": waves,
                "params": water_params
            }),
        )
    }

    fn water_create_river(&mut self, params: &Value) -> anyhow::Result<Value> {
        let river_id = optional_string(params, "river_id");
        let path = required_value(params, "path")?.clone();
        let river_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.water_mutation(
            "create_river",
            json!({
                "river_id": river_id,
                "path": path,
                "params": river_params
            }),
        )
    }

    fn water_create_waterfall(&mut self, params: &Value) -> anyhow::Result<Value> {
        let waterfall_id = optional_string(params, "waterfall_id");
        let waterfall_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.water_mutation(
            "create_waterfall",
            json!({
                "waterfall_id": waterfall_id,
                "params": waterfall_params
            }),
        )
    }

    fn water_set_waves(&mut self, params: &Value) -> anyhow::Result<Value> {
        let ocean_id = required_string(params, "ocean_id")?;
        let wave_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.water_mutation(
            "set_waves",
            json!({
                "ocean_id": ocean_id,
                "params": wave_params
            }),
        )
    }

    fn water_enable_foam(&mut self, params: &Value) -> anyhow::Result<Value> {
        let ocean_id = required_string(params, "ocean_id")?;
        let foam_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.water_mutation(
            "enable_foam",
            json!({
                "ocean_id": ocean_id,
                "params": foam_params
            }),
        )
    }

    fn water_enable_refraction(&mut self, params: &Value) -> anyhow::Result<Value> {
        let ocean_id = required_string(params, "ocean_id")?;
        let refraction_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.water_mutation(
            "enable_refraction",
            json!({
                "ocean_id": ocean_id,
                "params": refraction_params
            }),
        )
    }

    fn water_enable_caustics(&mut self, params: &Value) -> anyhow::Result<Value> {
        let ocean_id = required_string(params, "ocean_id")?;
        let caustics_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.water_mutation(
            "enable_caustics",
            json!({
                "ocean_id": ocean_id,
                "params": caustics_params
            }),
        )
    }

    fn water_add_buoyancy(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let buoyancy_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.water_mutation(
            "add_buoyancy",
            json!({
                "entity_id": entity_id,
                "params": buoyancy_params
            }),
        )
    }

    fn water_add_drag(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let drag_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.water_mutation(
            "add_drag",
            json!({
                "entity_id": entity_id,
                "params": drag_params
            }),
        )
    }

    fn water_sample_height(&self, params: &Value) -> anyhow::Result<Value> {
        let ocean_id = required_string(params, "ocean_id")?;
        let position = optional_vec3(params, "position")?
            .context("water.sample_height requires 'position' [x,y,z]")?;
        let ocean = self
            .command_bus
            .context()
            .water
            .oceans
            .get(&ocean_id)
            .with_context(|| format!("ocean '{}' not found", ocean_id))?;
        let base = ocean
            .params
            .get("base_height")
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(0.0);
        let amplitude = ocean
            .waves
            .get("amplitude")
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(1.0);
        let frequency = ocean
            .waves
            .get("frequency")
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(0.2);
        let phase = ocean
            .waves
            .get("phase")
            .and_then(Value::as_f64)
            .map(|value| value as f32)
            .unwrap_or(0.0);
        let height = base + amplitude * ((position[0] + position[2]) * frequency + phase).sin();
        Ok(json!({
            "ocean_id": ocean_id,
            "position": position,
            "height": height
        }))
    }

    fn mount_mutation(&mut self, operation: &str, params: Value) -> anyhow::Result<Value> {
        let receipt = self
            .command_bus
            .submit(Box::new(MountMutationCommand::new(operation, params)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn mount_create_horse_template(&mut self, params: &Value) -> anyhow::Result<Value> {
        let template_id = optional_string(params, "template_id");
        let template_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.mount_mutation(
            "create_horse_template",
            json!({
                "template_id": template_id,
                "params": template_params
            }),
        )
    }

    fn mount_spawn_horse(&mut self, params: &Value) -> anyhow::Result<Value> {
        let template_id = required_string(params, "template_id")?;
        let horse_id = optional_string(params, "horse_id");
        let entity_id = optional_string(params, "entity_id");
        let translation = optional_vec3(params, "translation")?;
        let spawn_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.mount_mutation(
            "spawn_horse",
            json!({
                "template_id": template_id,
                "horse_id": horse_id,
                "entity_id": entity_id,
                "translation": translation,
                "params": spawn_params
            }),
        )
    }

    fn mount_mount_rider(&mut self, params: &Value) -> anyhow::Result<Value> {
        let horse_id = required_string(params, "horse_id")?;
        let rider_id = required_string(params, "rider_id")?;
        self.mount_mutation(
            "mount_rider",
            json!({
                "horse_id": horse_id,
                "rider_id": rider_id
            }),
        )
    }

    fn mount_dismount(&mut self, params: &Value) -> anyhow::Result<Value> {
        let rider_id = required_string(params, "rider_id")?;
        self.mount_mutation("dismount", json!({ "rider_id": rider_id }))
    }

    fn mount_set_gait(&mut self, params: &Value) -> anyhow::Result<Value> {
        let horse_id = required_string(params, "horse_id")?;
        let gait = required_string(params, "gait")?;
        self.mount_mutation(
            "set_gait",
            json!({
                "horse_id": horse_id,
                "gait": gait
            }),
        )
    }

    fn mount_set_path_follow(&mut self, params: &Value) -> anyhow::Result<Value> {
        let horse_id = required_string(params, "horse_id")?;
        let path_id = required_string(params, "path_id")?;
        self.mount_mutation(
            "set_path_follow",
            json!({
                "horse_id": horse_id,
                "path_id": path_id
            }),
        )
    }

    fn ai_mutation(&mut self, operation: &str, params: Value) -> anyhow::Result<Value> {
        let receipt = self
            .command_bus
            .submit(Box::new(NpcAiMutationCommand::new(operation, params)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn ai_create_navmesh(&mut self, params: &Value) -> anyhow::Result<Value> {
        let navmesh_id = optional_string(params, "navmesh_id");
        let navmesh_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.ai_mutation(
            "create_navmesh",
            json!({
                "navmesh_id": navmesh_id,
                "params": navmesh_params
            }),
        )
    }

    fn ai_bake_navmesh(&mut self, params: &Value) -> anyhow::Result<Value> {
        let navmesh_id = optional_string(params, "navmesh_id");
        self.ai_mutation("bake_navmesh", json!({ "navmesh_id": navmesh_id }))
    }

    fn ai_add_agent(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let agent_id = optional_string(params, "agent_id");
        let agent_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.ai_mutation(
            "add_agent",
            json!({
                "entity_id": entity_id,
                "agent_id": agent_id,
                "params": agent_params
            }),
        )
    }

    fn ai_set_destination(&mut self, params: &Value) -> anyhow::Result<Value> {
        let agent_id = required_string(params, "agent_id")?;
        let position = optional_vec3(params, "position")?
            .context("ai.set_destination requires 'position' [x,y,z]")?;
        self.ai_mutation(
            "set_destination",
            json!({
                "agent_id": agent_id,
                "position": position
            }),
        )
    }

    fn ai_create_behavior_tree(&mut self, params: &Value) -> anyhow::Result<Value> {
        let name = required_string(params, "name")?;
        let tree_id = optional_string(params, "tree_id");
        self.ai_mutation(
            "create_behavior_tree",
            json!({
                "name": name,
                "tree_id": tree_id
            }),
        )
    }

    fn ai_bt_add_node(&mut self, params: &Value) -> anyhow::Result<Value> {
        let tree_id = required_string(params, "tree_id")?;
        let node_type = required_string(params, "node_type")?;
        let node_id = optional_string(params, "node_id");
        let node_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.ai_mutation(
            "bt_add_node",
            json!({
                "tree_id": tree_id,
                "node_type": node_type,
                "node_id": node_id,
                "params": node_params
            }),
        )
    }

    fn ai_bt_connect(&mut self, params: &Value) -> anyhow::Result<Value> {
        let tree_id = required_string(params, "tree_id")?;
        let parent = required_string(params, "parent")?;
        let child = required_string(params, "child")?;
        self.ai_mutation(
            "bt_connect",
            json!({
                "tree_id": tree_id,
                "parent": parent,
                "child": child
            }),
        )
    }

    fn ai_assign_behavior(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let tree_id = required_string(params, "tree_id")?;
        self.ai_mutation(
            "assign_behavior",
            json!({
                "entity_id": entity_id,
                "tree_id": tree_id
            }),
        )
    }

    fn ai_set_blackboard(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let key = required_string(params, "key")?;
        let value = required_value(params, "value")?.clone();
        self.ai_mutation(
            "set_blackboard",
            json!({
                "entity_id": entity_id,
                "key": key,
                "value": value
            }),
        )
    }

    fn ui_mutation(&mut self, operation: &str, params: Value) -> anyhow::Result<Value> {
        let receipt = self
            .command_bus
            .submit(Box::new(UiMutationCommand::new(operation, params)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn ui_create_canvas(&mut self, params: &Value) -> anyhow::Result<Value> {
        let name = required_string(params, "name")?;
        let canvas_id = optional_string(params, "canvas_id");
        let canvas_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.ui_mutation(
            "create_canvas",
            json!({
                "name": name,
                "canvas_id": canvas_id,
                "params": canvas_params
            }),
        )
    }

    fn ui_add_panel(&mut self, params: &Value) -> anyhow::Result<Value> {
        let canvas_id = required_string(params, "canvas_id")?;
        let ui_id = optional_string(params, "ui_id");
        let element_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.ui_mutation(
            "add_panel",
            json!({
                "canvas_id": canvas_id,
                "ui_id": ui_id,
                "params": element_params
            }),
        )
    }

    fn ui_add_text(&mut self, params: &Value) -> anyhow::Result<Value> {
        let canvas_id = required_string(params, "canvas_id")?;
        let ui_id = optional_string(params, "ui_id");
        let text = optional_string(params, "text");
        let element_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.ui_mutation(
            "add_text",
            json!({
                "canvas_id": canvas_id,
                "ui_id": ui_id,
                "text": text,
                "params": element_params
            }),
        )
    }

    fn ui_add_button(&mut self, params: &Value) -> anyhow::Result<Value> {
        let canvas_id = required_string(params, "canvas_id")?;
        let ui_id = optional_string(params, "ui_id");
        let label = optional_string(params, "label");
        let element_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.ui_mutation(
            "add_button",
            json!({
                "canvas_id": canvas_id,
                "ui_id": ui_id,
                "label": label,
                "params": element_params
            }),
        )
    }

    fn ui_bind_to_data(&mut self, params: &Value) -> anyhow::Result<Value> {
        let ui_id = required_string(params, "ui_id")?;
        let entity_id = required_string(params, "entity_id")?;
        let component_field = required_string(params, "component_field")?;
        self.ui_mutation(
            "bind_to_data",
            json!({
                "ui_id": ui_id,
                "entity_id": entity_id,
                "component_field": component_field
            }),
        )
    }

    fn ui_create_hud_template(&mut self, params: &Value) -> anyhow::Result<Value> {
        let template_type =
            optional_string(params, "type").or_else(|| optional_string(params, "template_type"));
        self.ui_mutation(
            "create_hud_template",
            json!({
                "type": template_type
            }),
        )
    }

    fn audio_mutation(&mut self, operation: &str, params: Value) -> anyhow::Result<Value> {
        let receipt = self
            .command_bus
            .submit(Box::new(AudioMutationCommand::new(operation, params)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn audio_import_clip(&mut self, params: &Value) -> anyhow::Result<Value> {
        let path = required_string(params, "path")?;
        let clip_id = optional_string(params, "clip_id");
        self.audio_mutation(
            "import_clip",
            json!({
                "path": path,
                "clip_id": clip_id
            }),
        )
    }

    fn audio_create_source(&mut self, params: &Value) -> anyhow::Result<Value> {
        let source_id = optional_string(params, "source_id");
        let entity_id = optional_string(params, "entity_id");
        let source_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        let spatial = params.get("spatial").cloned().unwrap_or_else(|| json!({}));
        self.audio_mutation(
            "create_source",
            json!({
                "source_id": source_id,
                "entity_id": entity_id,
                "params": source_params,
                "spatial": spatial
            }),
        )
    }

    fn audio_play(&mut self, params: &Value) -> anyhow::Result<Value> {
        let source_id = required_string(params, "source_id")?;
        let clip_id = required_string(params, "clip_id")?;
        self.audio_mutation(
            "play",
            json!({
                "source_id": source_id,
                "clip_id": clip_id
            }),
        )
    }

    fn audio_set_spatial(&mut self, params: &Value) -> anyhow::Result<Value> {
        let source_id = required_string(params, "source_id")?;
        let spatial = params
            .get("params")
            .cloned()
            .or_else(|| params.get("spatial").cloned())
            .unwrap_or_else(|| json!({}));
        self.audio_mutation(
            "set_spatial",
            json!({
                "source_id": source_id,
                "params": spatial
            }),
        )
    }

    fn audio_create_mixer(&mut self, params: &Value) -> anyhow::Result<Value> {
        let bus_id = optional_string(params, "bus_id");
        let mixer_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.audio_mutation(
            "create_mixer",
            json!({
                "bus_id": bus_id,
                "params": mixer_params
            }),
        )
    }

    fn audio_route(&mut self, params: &Value) -> anyhow::Result<Value> {
        let source_id = required_string(params, "source_id")?;
        let mixer_bus = required_string(params, "mixer_bus")?;
        self.audio_mutation(
            "route",
            json!({
                "source_id": source_id,
                "mixer_bus": mixer_bus
            }),
        )
    }

    fn net_mutation(&mut self, operation: &str, params: Value) -> anyhow::Result<Value> {
        let receipt = self
            .command_bus
            .submit(Box::new(NetMutationCommand::new(operation, params)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn net_create_server(&mut self, params: &Value) -> anyhow::Result<Value> {
        let server_id = optional_string(params, "server_id");
        let server_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.net_mutation(
            "create_server",
            json!({
                "server_id": server_id,
                "params": server_params
            }),
        )
    }

    fn net_connect_client(&mut self, params: &Value) -> anyhow::Result<Value> {
        let client_id = optional_string(params, "client_id");
        let endpoint = optional_string(params, "endpoint");
        let client_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        self.net_mutation(
            "connect_client",
            json!({
                "client_id": client_id,
                "endpoint": endpoint,
                "params": client_params
            }),
        )
    }

    fn net_enable_replication(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let components = required_string_array(params, "components")?;
        self.net_mutation(
            "enable_replication",
            json!({
                "entity_id": entity_id,
                "components": components
            }),
        )
    }

    fn net_set_prediction(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mode = optional_string(params, "mode");
        self.net_mutation(
            "set_prediction",
            json!({
                "mode": mode
            }),
        )
    }

    fn net_set_rollback(&mut self, params: &Value) -> anyhow::Result<Value> {
        let rollback_params = params
            .get("params")
            .cloned()
            .unwrap_or_else(|| params.clone());
        if !rollback_params.is_object() {
            bail!("net.set_rollback requires an object in 'params'");
        }
        self.net_mutation(
            "set_rollback",
            json!({
                "params": rollback_params
            }),
        )
    }

    fn build_mutation(&mut self, operation: &str, params: Value) -> anyhow::Result<Value> {
        let receipt = self
            .command_bus
            .submit(Box::new(BuildMutationCommand::new(operation, params)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn build_set_target(&mut self, params: &Value) -> anyhow::Result<Value> {
        let platform = optional_string(params, "platform")
            .or_else(|| optional_string(params, "target"))
            .unwrap_or_else(|| "windows".to_string());
        self.build_mutation(
            "set_target",
            json!({
                "platform": platform
            }),
        )
    }

    fn build_set_bundle_id(&mut self, params: &Value) -> anyhow::Result<Value> {
        let bundle_id = required_string(params, "id")?;
        self.build_mutation(
            "set_bundle_id",
            json!({
                "id": bundle_id
            }),
        )
    }

    fn build_set_version(&mut self, params: &Value) -> anyhow::Result<Value> {
        let version = required_string(params, "version")?;
        self.build_mutation(
            "set_version",
            json!({
                "version": version
            }),
        )
    }

    fn build_enable_feature(&mut self, params: &Value) -> anyhow::Result<Value> {
        let flag = required_string(params, "flag")?;
        self.build_mutation(
            "enable_feature",
            json!({
                "flag": flag
            }),
        )
    }

    fn build_export_project(&mut self, params: &Value) -> anyhow::Result<Value> {
        let export_path = optional_string(params, "path")
            .or_else(|| optional_string(params, "output_dir"))
            .or_else(|| optional_nested_string(params, "params", "path"))
            .unwrap_or_else(|| "dist/export".to_string());
        self.build_mutation(
            "export_project",
            json!({
                "path": export_path
            }),
        )
    }

    fn build_generate_installer(&mut self, params: &Value) -> anyhow::Result<Value> {
        let installer_path = optional_string(params, "path")
            .or_else(|| optional_string(params, "output_path"))
            .or_else(|| optional_nested_string(params, "params", "path"))
            .unwrap_or_else(|| "dist/installer".to_string());
        self.build_mutation(
            "generate_installer",
            json!({
                "path": installer_path
            }),
        )
    }

    fn debug_mutation(&mut self, operation: &str, params: Value) -> anyhow::Result<Value> {
        let receipt = self
            .command_bus
            .submit(Box::new(DebugMutationCommand::new(operation, params)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn debug_show_colliders(&mut self, params: &Value) -> anyhow::Result<Value> {
        let on = optional_bool(params, "on").unwrap_or(true);
        self.debug_mutation("show_colliders", json!({ "on": on }))
    }

    fn debug_show_navmesh(&mut self, params: &Value) -> anyhow::Result<Value> {
        let on = optional_bool(params, "on").unwrap_or(true);
        self.debug_mutation("show_navmesh", json!({ "on": on }))
    }

    fn debug_toggle_wireframe(&mut self, params: &Value) -> anyhow::Result<Value> {
        let on = optional_bool(params, "on").unwrap_or(true);
        self.debug_mutation("toggle_wireframe", json!({ "on": on }))
    }

    fn debug_capture_frame(&mut self, _params: &Value) -> anyhow::Result<Value> {
        self.debug_mutation("capture_frame", json!({}))
    }

    fn debug_get_profiler_snapshot(&self, params: &Value) -> anyhow::Result<Value> {
        let snapshots = &self.command_bus.context().debug.profiler_snapshots;
        let last_n = optional_usize(params, "last_n")
            .unwrap_or(1)
            .max(1)
            .min(snapshots.len().max(1));
        let start = snapshots.len().saturating_sub(last_n);
        let window = snapshots[start..].to_vec();
        Ok(json!({
            "count": window.len(),
            "latest": window.last(),
            "snapshots": window
        }))
    }

    fn debug_find_performance_hotspots(&self, params: &Value) -> anyhow::Result<Value> {
        let snapshots = &self.command_bus.context().debug.profiler_snapshots;
        if snapshots.is_empty() {
            return Ok(json!({
                "status": "empty",
                "message": "no profiler snapshots captured yet",
                "hotspots": []
            }));
        }

        let last_n = optional_usize(params, "last_n")
            .unwrap_or(8)
            .max(1)
            .min(snapshots.len());
        let start = snapshots.len().saturating_sub(last_n);
        let window = &snapshots[start..];

        let mut note_frequency = HashMap::<String, usize>::new();
        let mut min_fps = f32::MAX;
        let mut fps_sum = 0.0f32;
        for snapshot in window {
            min_fps = min_fps.min(snapshot.fps);
            fps_sum += snapshot.fps;
            for note in &snapshot.notes {
                *note_frequency.entry(note.clone()).or_insert(0) += 1;
            }
        }
        if min_fps == f32::MAX {
            min_fps = 0.0;
        }
        let avg_fps = fps_sum / window.len() as f32;
        if min_fps < 45.0 {
            *note_frequency
                .entry("fps_below_45".to_string())
                .or_insert(0) += 1;
        }

        let mut hotspots = note_frequency
            .into_iter()
            .map(|(note, hits)| {
                json!({
                    "id": note,
                    "hits": hits
                })
            })
            .collect::<Vec<Value>>();
        hotspots.sort_by(|a, b| {
            let a_hits = a.get("hits").and_then(Value::as_u64).unwrap_or(0);
            let b_hits = b.get("hits").and_then(Value::as_u64).unwrap_or(0);
            b_hits.cmp(&a_hits)
        });

        Ok(json!({
            "status": "ok",
            "sample_count": window.len(),
            "min_fps": min_fps,
            "avg_fps": avg_fps,
            "hotspots": hotspots
        }))
    }

    fn asset_mutation(&mut self, operation: &str, params: Value) -> anyhow::Result<Value> {
        let receipt = self
            .command_bus
            .submit(Box::new(AssetPipelineMutationCommand::new(
                operation, params,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn asset_import_file(&mut self, params: &Value) -> anyhow::Result<Value> {
        let path = required_string(params, "path")?;
        let target_subdir = optional_string(params, "target_subdir")
            .or_else(|| optional_nested_string(params, "options", "target_subdir"))
            .unwrap_or_else(|| "assets/imported".to_string());
        let receipt = self
            .command_bus
            .submit(Box::new(AssetImportFileCommand::new(path, target_subdir)))?;
        let import_json = command_receipt_to_json(receipt.clone());

        let instantiate_entity = optional_bool(params, "instantiate_entity")
            .or_else(|| optional_nested_bool(params, "options", "instantiate_entity"))
            .unwrap_or(false);
        if !instantiate_entity {
            return Ok(import_json);
        }

        let asset_id = receipt
            .result
            .payload
            .get("asset_id")
            .and_then(Value::as_str)
            .map(|value| value.to_string())
            .with_context(|| "asset.import_file missing asset_id in command payload")?;

        let entity_name = optional_string(params, "entity_name")
            .or_else(|| optional_nested_string(params, "options", "entity_name"))
            .unwrap_or_else(|| default_entity_name_from_asset_id(&asset_id));
        let translation = optional_vec3(params, "translation")?
            .or(optional_nested_vec3(params, "options", "translation")?)
            .or(optional_transform_position(params)?)
            .unwrap_or([0.0, 0.0, 0.0]);

        let instantiate_receipt =
            self.command_bus
                .submit(Box::new(AssetInstantiatePrefabCommand::new(
                    asset_id,
                    entity_name,
                    translation,
                )))?;
        Ok(json!({
            "import": import_json,
            "instantiate": command_receipt_to_json(instantiate_receipt)
        }))
    }

    fn asset_import_url(&mut self, params: &Value) -> anyhow::Result<Value> {
        let url = required_string(params, "url")?;
        let target_subdir = optional_string(params, "target_subdir")
            .or_else(|| optional_nested_string(params, "options", "target_subdir"))
            .unwrap_or_else(|| "assets/imported".to_string());
        let mut payload = json!({
            "url": url,
            "target_subdir": target_subdir
        });
        if let Some(file_name) = optional_string(params, "file_name")
            .or_else(|| optional_nested_string(params, "options", "file_name"))
        {
            payload["file_name"] = Value::String(file_name);
        }
        self.asset_mutation("import_url", payload)
    }

    fn asset_create_material(&mut self, params: &Value) -> anyhow::Result<Value> {
        let name = required_string(params, "name")?;
        let preset = optional_string(params, "preset").unwrap_or_else(|| "pbr_default".to_string());
        let material_params = params.get("params").cloned().unwrap_or_else(|| json!({}));
        let receipt = self
            .command_bus
            .submit(Box::new(AssetCreateMaterialCommand::new(
                name,
                preset,
                material_params,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn asset_create_texture(&mut self, params: &Value) -> anyhow::Result<Value> {
        let name = required_string(params, "name")?;
        let width = params
            .get("width")
            .and_then(Value::as_u64)
            .with_context(|| "asset.create_texture requires integer 'width'")?
            as u32;
        let height = params
            .get("height")
            .and_then(Value::as_u64)
            .with_context(|| "asset.create_texture requires integer 'height'")?
            as u32;
        let format = required_string(params, "format")?;
        let mut payload = json!({
            "name": name,
            "width": width,
            "height": height,
            "format": format,
            "params": params.get("params").cloned().unwrap_or_else(|| json!({}))
        });
        if let Some(texture_id) = optional_string(params, "texture_id") {
            payload["texture_id"] = Value::String(texture_id);
        }
        self.asset_mutation("create_texture", payload)
    }

    fn asset_create_shader(&mut self, params: &Value) -> anyhow::Result<Value> {
        let name = required_string(params, "name")?;
        let template = required_string(params, "template")?;
        let mut payload = json!({
            "name": name,
            "template": template,
            "params": params.get("params").cloned().unwrap_or_else(|| json!({}))
        });
        if let Some(shader_id) = optional_string(params, "shader_id") {
            payload["shader_id"] = Value::String(shader_id);
        }
        self.asset_mutation("create_shader", payload)
    }

    fn asset_create_prefab(&mut self, params: &Value) -> anyhow::Result<Value> {
        let name = required_string(params, "name")?;
        let entity_id = required_string(params, "entity_id")?;
        let mut payload = json!({
            "name": name,
            "entity_id": entity_id,
            "metadata": params.get("metadata").cloned().unwrap_or_else(|| json!({}))
        });
        if let Some(prefab_id) = optional_string(params, "prefab_id") {
            payload["prefab_id"] = Value::String(prefab_id);
        }
        self.asset_mutation("create_prefab", payload)
    }

    fn asset_save_prefab(&mut self, params: &Value) -> anyhow::Result<Value> {
        let prefab_id = required_string(params, "prefab_id")?;
        self.asset_mutation("save_prefab", json!({ "prefab_id": prefab_id }))
    }

    fn asset_rebuild_import(&mut self, params: &Value) -> anyhow::Result<Value> {
        let asset_id = required_string(params, "asset_id")?;
        self.asset_mutation(
            "rebuild_import",
            json!({
                "asset_id": asset_id,
                "params": params.get("params").cloned().unwrap_or_else(|| json!({}))
            }),
        )
    }

    fn asset_generate_lods(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mesh_id = required_string(params, "mesh_id")?;
        let mut payload = json!({
            "mesh_id": mesh_id,
            "params": params.get("params").cloned().unwrap_or_else(|| json!({}))
        });
        if let Some(levels) = optional_usize(params, "levels") {
            payload["levels"] = json!(levels);
        }
        if let Some(reduction) = optional_f32(params, "reduction") {
            payload["reduction"] = json!(reduction);
        }
        self.asset_mutation("generate_lods", payload)
    }

    fn asset_mesh_optimize(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mesh_id = required_string(params, "mesh_id")?;
        let profile = optional_string(params, "profile").unwrap_or_else(|| "balanced".to_string());
        self.asset_mutation(
            "mesh_optimize",
            json!({
                "mesh_id": mesh_id,
                "profile": profile,
                "params": params.get("params").cloned().unwrap_or_else(|| json!({}))
            }),
        )
    }

    fn asset_compress_textures(&mut self, params: &Value) -> anyhow::Result<Value> {
        let asset_id = required_string(params, "asset_id")?;
        let format = optional_string(params, "format").unwrap_or_else(|| "bc7".to_string());
        let quality = optional_string(params, "quality").unwrap_or_else(|| "balanced".to_string());
        self.asset_mutation(
            "compress_textures",
            json!({
                "asset_id": asset_id,
                "format": format,
                "quality": quality,
                "params": params.get("params").cloned().unwrap_or_else(|| json!({}))
            }),
        )
    }

    fn asset_bake_lightmaps(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.asset_mutation(
            "bake_lightmaps",
            json!({
                "params": params.get("params").cloned().unwrap_or_else(|| params.clone())
            }),
        )
    }

    fn asset_bake_reflection_probes(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.asset_mutation(
            "bake_reflection_probes",
            json!({
                "params": params.get("params").cloned().unwrap_or_else(|| params.clone())
            }),
        )
    }

    fn asset_assign_material(&mut self, params: &Value) -> anyhow::Result<Value> {
        let entity_id = required_string(params, "entity_id")?;
        let material_id = required_string(params, "material_id")?;
        let slot = optional_string(params, "slot").unwrap_or_else(|| "default".to_string());

        let ctx = self.command_bus.context();
        if !ctx.entity_exists(&entity_id) {
            bail!("entity '{}' not found", entity_id);
        }
        let material_exists = ctx.materials.contains_key(&material_id)
            || resolve_project_path(&ctx.project_root, Path::new(&material_id)).exists();
        if !material_exists {
            bail!(
                "material '{}' not found; create it with asset.create_material first",
                material_id
            );
        }

        let receipt = self
            .command_bus
            .submit(Box::new(EntityAddComponentCommand::new(
                entity_id.clone(),
                "MaterialOverride",
                json!({
                    "material_id": material_id,
                    "slot": slot
                }),
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn asset_instantiate_prefab(&mut self, params: &Value) -> anyhow::Result<Value> {
        let prefab_id = optional_string(params, "prefab_id")
            .or_else(|| optional_string(params, "asset_id"))
            .with_context(|| "asset.instantiate_prefab requires 'prefab_id' or 'asset_id'")?;
        let entity_name = optional_string(params, "entity_name")
            .unwrap_or_else(|| default_entity_name_from_asset_id(&prefab_id));
        let translation = optional_vec3(params, "translation")?
            .or(optional_transform_position(params)?)
            .unwrap_or([0.0, 0.0, 0.0]);

        let receipt = self
            .command_bus
            .submit(Box::new(AssetInstantiatePrefabCommand::new(
                prefab_id,
                entity_name,
                translation,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn render_assign_material(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.asset_assign_material(params)
    }

    fn render_create_light(&mut self, params: &Value) -> anyhow::Result<Value> {
        let light_type =
            optional_string(params, "type").unwrap_or_else(|| "directional".to_string());
        if !light_type.eq_ignore_ascii_case("directional") {
            bail!("render.create_light currently supports only 'directional'");
        }
        self.render_set_light_params(params)
    }

    fn render_set_light_params(&mut self, params: &Value) -> anyhow::Result<Value> {
        let current = self.command_bus.render_settings();
        let direction = optional_vec3(params, "direction")?.unwrap_or(current.light_direction);
        let color = optional_vec3(params, "color")?.unwrap_or(current.light_color);
        let intensity = optional_f32(params, "intensity").unwrap_or(current.light_intensity);
        let shadow_bias = optional_f32(params, "shadow_bias").unwrap_or(current.shadow_bias);
        let shadow_strength =
            optional_f32(params, "shadow_strength").unwrap_or(current.shadow_strength);
        let shadow_cascade_count = optional_usize(params, "shadow_cascade_count")
            .map(|value| value as u32)
            .unwrap_or(current.shadow_cascade_count);
        let receipt = self
            .command_bus
            .submit(Box::new(RenderSetLightCommand::new(
                direction,
                color,
                intensity,
                shadow_bias,
                shadow_strength,
                shadow_cascade_count,
            )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn render_set_ibl(&mut self, params: &Value) -> anyhow::Result<Value> {
        let current = self.command_bus.render_settings();
        let sky_color = optional_vec3(params, "sky_color")?.unwrap_or(current.ibl_sky_color);
        let ground_color =
            optional_vec3(params, "ground_color")?.unwrap_or(current.ibl_ground_color);
        let intensity = optional_f32(params, "intensity").unwrap_or(current.ibl_intensity);
        let receipt = self.command_bus.submit(Box::new(RenderSetIblCommand::new(
            sky_color,
            ground_color,
            intensity,
        )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn render_set_postprocess(&mut self, params: &Value) -> anyhow::Result<Value> {
        let current = self.command_bus.render_settings();
        let preset_name = optional_string(params, "preset");
        let mut resolved = postprocess_params_from_preset(preset_name.as_deref(), &current);
        resolved.exposure = optional_f32(params, "exposure").unwrap_or(resolved.exposure);
        resolved.gamma = optional_f32(params, "gamma").unwrap_or(resolved.gamma);
        resolved.bloom_intensity =
            optional_f32(params, "bloom_intensity").unwrap_or(resolved.bloom_intensity);
        resolved.bloom_threshold =
            optional_f32(params, "bloom_threshold").unwrap_or(resolved.bloom_threshold);
        resolved.bloom_radius =
            optional_f32(params, "bloom_radius").unwrap_or(resolved.bloom_radius);
        resolved.fog_density = optional_f32(params, "fog_density").unwrap_or(resolved.fog_density);
        if let Some(fog_color) = optional_vec3(params, "fog_color")? {
            resolved.fog_color = fog_color;
        }
        resolved.saturation = optional_f32(params, "saturation").unwrap_or(resolved.saturation);
        resolved.contrast = optional_f32(params, "contrast").unwrap_or(resolved.contrast);
        resolved.white_balance =
            optional_f32(params, "white_balance").unwrap_or(resolved.white_balance);
        if let Some(grade_tint) = optional_vec3(params, "grade_tint")? {
            resolved.grade_tint = grade_tint;
        }
        resolved.color_grading_preset = optional_string(params, "color_grading_preset")
            .or(preset_name)
            .unwrap_or_else(|| current.color_grading_preset.clone());
        let receipt = self
            .command_bus
            .submit(Box::new(RenderSetPostprocessCommand::new(resolved)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn render_set_lod_settings(&mut self, params: &Value) -> anyhow::Result<Value> {
        let current = self.command_bus.render_settings();
        let mut near_distance = current.lod_transition_distances[0];
        let mut far_distance = current.lod_transition_distances[1];

        if let Some(raw_distances) = params.get("transition_distances") {
            let distances = raw_distances.as_array().with_context(
                || "'transition_distances' must be an array with exactly 2 numeric values",
            )?;
            if distances.len() != 2 {
                bail!("'transition_distances' must contain exactly 2 values");
            }
            near_distance = distances[0]
                .as_f64()
                .map(|value| value as f32)
                .with_context(|| "'transition_distances[0]' must be numeric")?;
            far_distance = distances[1]
                .as_f64()
                .map(|value| value as f32)
                .with_context(|| "'transition_distances[1]' must be numeric")?;
        }

        near_distance = optional_f32(params, "near_distance").unwrap_or(near_distance);
        far_distance = optional_f32(params, "far_distance").unwrap_or(far_distance);
        let hysteresis = optional_f32(params, "hysteresis").unwrap_or(current.lod_hysteresis);

        let receipt = self.command_bus.submit(Box::new(RenderSetLodCommand::new(
            [near_distance, far_distance],
            hysteresis,
        )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn graph_create(&mut self, params: &Value) -> anyhow::Result<Value> {
        let graph_name = optional_string(params, "graph_name")
            .or_else(|| optional_string(params, "name"))
            .with_context(|| "graph.create requires 'graph_name'")?;
        let receipt = self
            .command_bus
            .submit(Box::new(LowcodeSetGraphCommand::new(NodeGraphFile::new(
                graph_name,
            ))))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn graph_add_node(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mut graph = self.active_graph_clone()?;
        let node_id = required_string(params, "id")?;
        if graph
            .nodes
            .iter()
            .any(|node| node.id.eq_ignore_ascii_case(&node_id))
        {
            bail!("graph node '{}' already exists", node_id);
        }
        let node_type = required_string(params, "type")?;
        if !supported_node_type(&node_type) {
            bail!("unsupported node type '{}'", node_type);
        }
        graph.nodes.push(NodeGraphNode {
            id: node_id,
            node_type,
            params: params.get("params").cloned().unwrap_or_else(|| json!({})),
        });
        let receipt = self
            .command_bus
            .submit(Box::new(LowcodeSetGraphCommand::new(graph)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn graph_connect(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mut graph = self.active_graph_clone()?;
        let from = required_string(params, "from")?;
        let to = required_string(params, "to")?;
        let pin = optional_string(params, "pin").unwrap_or_else(|| "flow".to_string());
        if !graph
            .nodes
            .iter()
            .any(|node| node.id.eq_ignore_ascii_case(&from))
        {
            bail!("source node '{}' not found", from);
        }
        if !graph
            .nodes
            .iter()
            .any(|node| node.id.eq_ignore_ascii_case(&to))
        {
            bail!("target node '{}' not found", to);
        }
        graph.edges.push(NodeGraphEdge { from, to, pin });
        let receipt = self
            .command_bus
            .submit(Box::new(LowcodeSetGraphCommand::new(graph)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn graph_delete_node(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mut graph = self.active_graph_clone()?;
        let node_id = required_string(params, "id")?;
        let previous_count = graph.nodes.len();
        graph
            .nodes
            .retain(|node| !node.id.eq_ignore_ascii_case(&node_id));
        if graph.nodes.len() == previous_count {
            bail!("node '{}' not found", node_id);
        }
        graph.edges.retain(|edge| {
            !edge.from.eq_ignore_ascii_case(&node_id) && !edge.to.eq_ignore_ascii_case(&node_id)
        });
        let receipt = self
            .command_bus
            .submit(Box::new(LowcodeSetGraphCommand::new(graph)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn graph_delete_edge(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mut graph = self.active_graph_clone()?;
        let from = required_string(params, "from")?;
        let to = required_string(params, "to")?;
        let pin = optional_string(params, "pin").unwrap_or_else(|| "flow".to_string());
        let previous_count = graph.edges.len();
        graph.edges.retain(|edge| {
            !(edge.from.eq_ignore_ascii_case(&from)
                && edge.to.eq_ignore_ascii_case(&to)
                && edge.pin.eq_ignore_ascii_case(&pin))
        });
        if graph.edges.len() == previous_count {
            bail!(
                "edge not found (from='{}', to='{}', pin='{}')",
                from,
                to,
                pin
            );
        }
        let receipt = self
            .command_bus
            .submit(Box::new(LowcodeSetGraphCommand::new(graph)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn graph_set_node_params(&mut self, params: &Value) -> anyhow::Result<Value> {
        let mut graph = self.active_graph_clone()?;
        let node_id = required_string(params, "id")?;
        let node_params = params
            .get("params")
            .cloned()
            .with_context(|| "graph.set_node_params requires 'params' object")?;
        let Some(node) = graph
            .nodes
            .iter_mut()
            .find(|node| node.id.eq_ignore_ascii_case(&node_id))
        else {
            bail!("node '{}' not found", node_id);
        };
        node.params = node_params;
        let receipt = self
            .command_bus
            .submit(Box::new(LowcodeSetGraphCommand::new(graph)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn graph_validate(&self) -> anyhow::Result<Value> {
        let graph = self
            .command_bus
            .context()
            .node_graph
            .graph
            .clone()
            .context("no active graph loaded")?;
        let report = validate_node_graph(&graph);
        Ok(json!({
            "graph_name": graph.graph_name,
            "validation": report
        }))
    }

    fn graph_run(&mut self, params: &Value) -> anyhow::Result<Value> {
        let events = params
            .get("events")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .filter_map(GraphEvent::parse)
                    .collect::<Vec<GraphEvent>>()
            })
            .unwrap_or_else(|| vec![GraphEvent::OnStart]);
        let receipt = self
            .command_bus
            .submit(Box::new(LowcodeRunGraphCommand::new(events)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn template_list(&self) -> anyhow::Result<Value> {
        let list = builtin_template_specs()
            .into_iter()
            .map(|spec| {
                json!({
                    "template_id": spec.template_id,
                    "display_name": spec.display_name,
                    "scene_name": spec.scene.name,
                    "entity_count": spec.scene.entities.len(),
                    "graph_name": spec.graph.graph_name,
                    "graph_nodes": spec.graph.nodes.len(),
                    "graph_edges": spec.graph.edges.len()
                })
            })
            .collect::<Vec<Value>>();
        Ok(json!({ "templates": list }))
    }

    fn template_apply(&mut self, params: &Value) -> anyhow::Result<Value> {
        let template_id = required_string(params, "template_id")?;
        let receipt = self
            .command_bus
            .submit(Box::new(LowcodeApplyTemplateCommand::new(template_id)))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn asset_get_template_bundle(&self, params: &Value) -> anyhow::Result<Value> {
        let template_id = required_string(params, "template_id")?;
        let bundle = builtin_template_bundle(&template_id)
            .with_context(|| format!("unknown template '{}'", template_id))?;
        Ok(json!({
            "template_id": template_id,
            "bundle": bundle
        }))
    }

    fn asset_validate_template_bundle(&mut self, params: &Value) -> anyhow::Result<Value> {
        let template_id = required_string(params, "template_id")?;
        let receipt =
            self.command_bus
                .submit(Box::new(LowcodeValidateTemplateBundleCommand::new(
                    template_id,
                )))?;
        Ok(command_receipt_to_json(receipt))
    }

    fn active_graph_clone(&self) -> anyhow::Result<NodeGraphFile> {
        self.command_bus
            .context()
            .node_graph
            .graph
            .clone()
            .context("no active graph loaded; call graph.create or template.apply first")
    }

    fn gen_run_task_graph(
        &mut self,
        macro_name: &str,
        steps: Vec<Value>,
        auto_transaction: bool,
    ) -> anyhow::Result<Value> {
        let step_count = steps.len();
        let execution = self.gen_execute_plan(&json!({
            "task_graph": {
                "steps": steps
            },
            "auto_transaction": auto_transaction
        }))?;
        let status = execution
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("ok")
            .to_string();
        Ok(json!({
            "macro": macro_name,
            "status": status,
            "step_count": step_count,
            "execution": execution
        }))
    }

    fn gen_create_game_from_template(&mut self, params: &Value) -> anyhow::Result<Value> {
        let template_id = required_string(params, "template_id")?;
        let auto_transaction = optional_bool(params, "auto_transaction").unwrap_or(true);
        let mut steps = vec![
            json!({"tool":"template.apply","params":{"template_id":template_id}}),
            json!({"tool":"graph.run","params":{"events":["OnStart"]}}),
        ];
        if let Some(postprocess) = params.get("postprocess").cloned() {
            steps.push(json!({
                "tool": "render.set_postprocess",
                "params": postprocess
            }));
        } else if let Some(preset) = optional_string(params, "postprocess_preset") {
            steps.push(json!({
                "tool":"render.set_postprocess",
                "params":{"preset":preset}
            }));
        }
        if let Some(name) = optional_string(params, "save_as") {
            steps.push(json!({
                "tool":"scene.save_as",
                "params":{"name":name}
            }));
        }
        self.gen_run_task_graph("gen.create_game_from_template", steps, auto_transaction)
    }

    fn gen_macro_from_prompt(
        &mut self,
        macro_name: &str,
        default_prompt: &str,
        params: &Value,
    ) -> anyhow::Result<Value> {
        let prompt =
            optional_string(params, "prompt").unwrap_or_else(|| default_prompt.to_string());
        let auto_transaction = optional_bool(params, "auto_transaction").unwrap_or(true);
        let plan = self.gen_plan_from_prompt(&json!({
            "prompt": prompt
        }))?;
        let mut steps = plan
            .get("steps")
            .and_then(Value::as_array)
            .cloned()
            .with_context(|| format!("{} planner output must contain 'steps' array", macro_name))?;
        if let Some(name) = optional_string(params, "save_as") {
            steps.push(json!({
                "tool":"scene.save_as",
                "params":{"name":name}
            }));
        }
        let plan_id = plan.get("plan_id").cloned().unwrap_or(Value::Null);
        let execution = self.gen_run_task_graph(macro_name, steps, auto_transaction)?;
        Ok(json!({
            "macro": macro_name,
            "plan_id": plan_id,
            "prompt": prompt,
            "status": execution.get("status").cloned().unwrap_or_else(|| json!("ok")),
            "result": execution
        }))
    }

    fn gen_create_platformer_level(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.gen_macro_from_prompt(
            "gen.create_platformer_level",
            "create a platform runner level",
            params,
        )
    }

    fn gen_create_shooter_arena(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.gen_macro_from_prompt(
            "gen.create_shooter_arena",
            "build a fast shooter prototype",
            params,
        )
    }

    fn gen_create_island_adventure(&mut self, params: &Value) -> anyhow::Result<Value> {
        self.gen_macro_from_prompt(
            "gen.create_island_adventure",
            "create a medieval island adventure",
            params,
        )
    }

    fn gen_package_demo_build(&mut self, params: &Value) -> anyhow::Result<Value> {
        let target = optional_string(params, "target").unwrap_or_else(|| "windows".to_string());
        let version = optional_string(params, "version").unwrap_or_else(|| "0.1.0".to_string());
        let bundle_id = optional_string(params, "bundle_id").unwrap_or_else(|| {
            format!(
                "com.demo.generated.{}",
                chrono::Utc::now().timestamp_millis()
            )
        });
        let export_path = optional_string(params, "export_path")
            .unwrap_or_else(|| "dist/demo_export".to_string());
        let installer_path = optional_string(params, "installer_path")
            .unwrap_or_else(|| "dist/demo_installer".to_string());
        let mut features = optional_string_array(params, "features");
        if features.is_empty() {
            features.push("demo".to_string());
        }

        let run_build = optional_bool(params, "run_build").unwrap_or(false);
        let run_target =
            optional_string(params, "run_target").unwrap_or_else(|| "editor".to_string());
        let run_profile =
            optional_string(params, "profile").unwrap_or_else(|| "release".to_string());
        let run_binary = optional_bool(params, "run_binary").unwrap_or(false);
        let dry_run = optional_bool(params, "dry_run").unwrap_or(true);
        let wait_for_run = optional_bool(params, "wait_for_run").unwrap_or(false);

        let mut steps = vec![
            json!({"tool":"build.set_target","params":{"platform":target}}),
            json!({"tool":"build.set_bundle_id","params":{"id":bundle_id}}),
            json!({"tool":"build.set_version","params":{"version":version}}),
        ];
        for flag in features {
            steps.push(json!({
                "tool":"build.enable_feature",
                "params":{"flag":flag}
            }));
        }
        steps.push(json!({
            "tool":"build.export_project",
            "params":{"path":export_path}
        }));
        steps.push(json!({
            "tool":"build.generate_installer",
            "params":{"path":installer_path}
        }));
        if run_build {
            steps.push(json!({
                "tool":"build.build_and_run",
                "params":{
                    "target":run_target,
                    "profile":run_profile,
                    "run":run_binary,
                    "dry_run":dry_run,
                    "wait_for_run":wait_for_run
                }
            }));
        }

        let execution = self.gen_run_task_graph("gen.package_demo_build", steps, false)?;
        Ok(json!({
            "macro": "gen.package_demo_build",
            "status": execution.get("status").cloned().unwrap_or_else(|| json!("ok")),
            "result": execution
        }))
    }

    fn gen_plan_from_prompt(&self, params: &Value) -> anyhow::Result<Value> {
        let prompt = required_string(params, "prompt")?;
        let lower = prompt.to_ascii_lowercase();
        let plan_timestamp = chrono::Utc::now().timestamp_millis();
        let mut steps = Vec::<Value>::new();

        if lower.contains("shooter") {
            let actor_name = format!("AgentPawn_{}", plan_timestamp);
            let weapon_id = format!("rifle_{}", plan_timestamp);
            let shoot_action = format!("Shoot_{}", plan_timestamp);
            steps.push(
                json!({"tool":"template.apply","params":{"template_id":"template_shooter_arena"}}),
            );
            steps.push(json!({"tool":"graph.run","params":{"events":["OnStart"]}}));
            steps.push(json!({"tool":"entity.create","params":{"name":actor_name,"mesh":"capsule","translation":[0.0,1.0,0.0]}}));
            steps.push(json!({"tool":"phys.add_collider","params":{"entity_id":actor_name,"shape":"capsule","size":[0.8,1.8,0.8]}}));
            steps.push(json!({"tool":"phys.add_rigidbody","params":{"entity_id":actor_name,"type":"dynamic","mass":75.0,"friction":0.7}}));
            steps.push(json!({"tool":"phys.add_character_controller","params":{"entity_id":actor_name,"radius":0.4,"height":1.8,"speed":5.4,"jump_strength":6.8}}));
            steps.push(json!({"tool":"phys.character_set_state","params":{"entity_id":actor_name,"state":"idle"}}));
            steps.push(json!({"tool":"game.create_input_action","params":{"name":shoot_action,"bindings":["MouseLeft","GamepadRT"]}}));
            steps.push(json!({"tool":"game.bind_action","params":{"name":shoot_action,"target_script_event":"weapon_fire"}}));
            steps.push(json!({"tool":"game.add_health_component","params":{"entity_id":actor_name,"max_health":100.0}}));
            steps.push(json!({"tool":"game.add_inventory","params":{"entity_id":actor_name,"capacity":6,"items":[]}}));
            steps.push(json!({"tool":"game.create_weapon","params":{"weapon_id":weapon_id,"rate":8.5,"recoil":0.9,"spread":0.06,"ammo_capacity":30}}));
            steps.push(json!({"tool":"game.attach_weapon","params":{"character_id":actor_name,"weapon_id":weapon_id}}));
            steps.push(json!({"tool":"game.fire_weapon","params":{"character_id":actor_name}}));
            steps.push(
                json!({"tool":"render.set_postprocess","params":{"exposure":1.05,"gamma":2.2}}),
            );
        } else if lower.contains("island") || lower.contains("medieval") || lower.contains("isla") {
            steps.push(
                json!({"tool":"template.apply","params":{"template_id":"template_medieval_island"}}),
            );
            steps.push(json!({"tool":"graph.run","params":{"events":["OnStart"]}}));
            steps.push(json!({"tool":"render.set_light_params","params":{"direction":[-0.4,-1.0,-0.2],"intensity":5.8}}));
        } else if lower.contains("platform")
            || lower.contains("plataforma")
            || lower.contains("runner")
        {
            steps.push(
                json!({"tool":"template.apply","params":{"template_id":"template_platform_runner"}}),
            );
            steps.push(json!({"tool":"graph.run","params":{"events":["OnStart"]}}));
            steps.push(
                json!({"tool":"render.set_postprocess","params":{"exposure":0.95,"gamma":2.2}}),
            );
        } else if lower.contains("horse")
            || lower.contains("caballo")
            || lower.contains("character animation")
        {
            let actor_name = format!("Rider_{}", plan_timestamp);
            let horse_template_id = format!("horse_tpl_{}", plan_timestamp);
            let horse_id = format!("horse_{}", plan_timestamp);
            steps.push(
                json!({"tool":"scene.create","params":{"name":"Generated Horse Animation Scene"}}),
            );
            steps.push(json!({"tool":"entity.create","params":{"name":actor_name,"mesh":"capsule","translation":[0.0,1.0,0.0]}}));
            steps.push(json!({"tool":"mount.create_horse_template","params":{"template_id":horse_template_id,"params":{"mesh":"horse","stats":{"speed":8.0}}}}));
            steps.push(json!({"tool":"mount.spawn_horse","params":{"template_id":horse_template_id,"horse_id":horse_id,"entity_id":"HorseMount","translation":[1.5,0.0,0.0]}}));
            steps.push(json!({"tool":"mount.mount_rider","params":{"horse_id":horse_id,"rider_id":actor_name}}));
            steps.push(
                json!({"tool":"mount.set_gait","params":{"horse_id":horse_id,"gait":"gallop"}}),
            );
            steps.push(json!({"tool":"anim.create_state_machine","params":{"name":"HorseController","controller_id":"horse_controller"}}));
            steps.push(json!({"tool":"anim.add_state","params":{"controller_id":"horse_controller","state_name":"idle","clip_id":"horse_idle"}}));
            steps.push(json!({"tool":"anim.add_state","params":{"controller_id":"horse_controller","state_name":"gallop","clip_id":"horse_gallop"}}));
            steps.push(json!({"tool":"anim.add_transition","params":{"controller_id":"horse_controller","from":"idle","to":"gallop","conditions":{"speed_gt":0.6}}}));
            steps.push(json!({"tool":"anim.set_parameter","params":{"controller_id":"horse_controller","key":"speed","value":1.0}}));
            steps.push(json!({"tool":"anim.add_animator","params":{"entity_id":actor_name,"controller_id":"horse_controller"}}));
            steps.push(json!({"tool":"anim.play","params":{"entity_id":actor_name,"clip_id":"horse_idle"}}));
        } else if lower.contains("npc")
            || lower.contains("enemy ai")
            || lower.contains("navmesh")
            || lower.contains("behavior tree")
        {
            let actor_name = format!("Npc_{}", plan_timestamp);
            let agent_id = format!("agent_{}", plan_timestamp);
            let tree_id = format!("bt_guard_{}", plan_timestamp);
            steps.push(json!({"tool":"scene.create","params":{"name":"Generated NPC AI Scene"}}));
            steps.push(json!({"tool":"entity.create","params":{"name":actor_name,"mesh":"capsule","translation":[0.0,1.0,0.0]}}));
            steps.push(json!({"tool":"ai.create_navmesh","params":{"navmesh_id":"main_navmesh","params":{"cell_size":0.2,"agent_radius":0.4}}}));
            steps.push(json!({"tool":"ai.bake_navmesh","params":{"navmesh_id":"main_navmesh"}}));
            steps.push(json!({"tool":"ai.add_agent","params":{"entity_id":actor_name,"agent_id":agent_id,"params":{"speed":3.6}}}));
            steps.push(json!({"tool":"ai.set_destination","params":{"agent_id":agent_id,"position":[6.0,0.0,6.0]}}));
            steps.push(json!({"tool":"ai.create_behavior_tree","params":{"name":"GuardTree","tree_id":tree_id}}));
            steps.push(json!({"tool":"ai.bt_add_node","params":{"tree_id":tree_id,"node_type":"Selector","node_id":"root","params":{}}}));
            steps.push(json!({"tool":"ai.bt_add_node","params":{"tree_id":tree_id,"node_type":"ChaseTarget","node_id":"chase","params":{}}}));
            steps.push(json!({"tool":"ai.bt_connect","params":{"tree_id":tree_id,"parent":"root","child":"chase"}}));
            steps.push(json!({"tool":"ai.assign_behavior","params":{"entity_id":actor_name,"tree_id":tree_id}}));
            steps.push(json!({"tool":"ai.set_blackboard","params":{"entity_id":actor_name,"key":"target","value":"Player"}}));
        } else if lower.contains("hud")
            || lower.contains(" ui ")
            || lower.starts_with("ui ")
            || lower.contains("interface")
        {
            let ui_actor = format!("UiActor_{}", plan_timestamp);
            steps.push(json!({"tool":"scene.create","params":{"name":"Generated UI HUD Scene"}}));
            steps.push(json!({"tool":"entity.create","params":{"name":ui_actor,"mesh":"capsule","translation":[0.0,1.0,0.0]}}));
            steps.push(json!({"tool":"game.add_health_component","params":{"entity_id":ui_actor,"max_health":100.0,"current_health":100.0}}));
            steps.push(json!({"tool":"ui.create_canvas","params":{"name":"MainHUD","canvas_id":"hud_main"}}));
            steps.push(json!({"tool":"ui.add_panel","params":{"canvas_id":"hud_main","ui_id":"hud_panel","params":{"anchor":"top_left","size":[360,120]}}}));
            steps.push(json!({"tool":"ui.add_text","params":{"canvas_id":"hud_main","ui_id":"hud_health","text":"HP: 100","params":{"anchor":"top_left"}}}));
            steps.push(json!({"tool":"ui.add_button","params":{"canvas_id":"hud_main","ui_id":"hud_pause","label":"Pause","params":{"anchor":"top_right"}}}));
            steps.push(json!({"tool":"ui.bind_to_data","params":{"ui_id":"hud_health","entity_id":ui_actor,"component_field":"Health.current_health"}}));
            steps.push(json!({"tool":"ui.create_hud_template","params":{"type":"shooter"}}));
        } else if lower.contains("audio") || lower.contains("sound") || lower.contains("music") {
            let speaker = format!("AudioEmitter_{}", plan_timestamp);
            let source_id = format!("src_{}", plan_timestamp);
            let clip_id = format!("clip_{}", plan_timestamp);
            steps.push(json!({"tool":"scene.create","params":{"name":"Generated Audio Scene"}}));
            steps.push(json!({"tool":"entity.create","params":{"name":speaker,"mesh":"cube","translation":[0.0,1.0,0.0]}}));
            steps.push(json!({"tool":"audio.import_clip","params":{"path":"Cargo.toml","clip_id":clip_id}}));
            steps.push(json!({"tool":"audio.create_mixer","params":{"bus_id":"master","params":{"volume":1.0}}}));
            steps.push(json!({"tool":"audio.create_source","params":{"source_id":source_id,"entity_id":speaker,"params":{"loop":false}}}));
            steps.push(json!({"tool":"audio.set_spatial","params":{"source_id":source_id,"params":{"min_distance":1.0,"max_distance":20.0}}}));
            steps.push(
                json!({"tool":"audio.route","params":{"source_id":source_id,"mixer_bus":"master"}}),
            );
            steps.push(
                json!({"tool":"audio.play","params":{"source_id":source_id,"clip_id":clip_id}}),
            );
        } else if lower.contains("network")
            || lower.contains("multiplayer")
            || lower.contains("replication")
            || lower.contains("netcode")
        {
            let entity_name = format!("NetActor_{}", plan_timestamp);
            let client_id = format!("client_{}", plan_timestamp);
            steps.push(
                json!({"tool":"scene.create","params":{"name":"Generated Networking Scene"}}),
            );
            steps.push(json!({"tool":"entity.create","params":{"name":entity_name,"mesh":"capsule","translation":[0.0,1.0,0.0]}}));
            steps.push(json!({"tool":"net.create_server","params":{"server_id":"server_main","params":{"port":7777}}}));
            steps.push(json!({"tool":"net.connect_client","params":{"client_id":client_id,"endpoint":"127.0.0.1:7777","params":{"transport":"udp"}}}));
            steps.push(json!({"tool":"net.enable_replication","params":{"entity_id":entity_name,"components":["Transform","Health"]}}));
            steps.push(json!({"tool":"net.set_prediction","params":{"mode":"hybrid"}}));
            steps.push(json!({"tool":"net.set_rollback","params":{"params":{"max_frames":6,"input_delay_ms":100}}}));
        } else if lower.contains("build")
            || lower.contains("export")
            || lower.contains("installer")
            || lower.contains("package")
        {
            let bundle_id = format!("com.ai.generated.{}", plan_timestamp);
            steps.push(json!({"tool":"build.set_target","params":{"platform":"windows"}}));
            steps.push(json!({"tool":"build.set_bundle_id","params":{"id":bundle_id}}));
            steps.push(json!({"tool":"build.set_version","params":{"version":"0.1.0"}}));
            steps.push(json!({"tool":"build.enable_feature","params":{"flag":"shipping"}}));
            steps.push(json!({"tool":"build.export_project","params":{"path":"dist/export"}}));
            steps.push(
                json!({"tool":"build.generate_installer","params":{"path":"dist/installer"}}),
            );
        } else if lower.contains("debug")
            || lower.contains("profile")
            || lower.contains("profiler")
            || lower.contains("performance")
        {
            steps.push(json!({"tool":"debug.show_colliders","params":{"on":true}}));
            steps.push(json!({"tool":"debug.show_navmesh","params":{"on":true}}));
            steps.push(json!({"tool":"debug.toggle_wireframe","params":{"on":true}}));
            steps.push(json!({"tool":"debug.capture_frame","params":{}}));
            steps.push(json!({"tool":"debug.get_profiler_snapshot","params":{"last_n":1}}));
            steps.push(json!({"tool":"debug.find_performance_hotspots","params":{"last_n":8}}));
        } else if lower.contains("model") || lower.contains("modelado") || lower.contains("sculpt")
        {
            let mesh_id = format!("mesh_gen_{}", plan_timestamp);
            steps.push(json!({"tool":"model.create_primitive","params":{"type":"cube","name":"GeneratedMesh","mesh_id":mesh_id,"translation":[0.0,0.0,0.0]}}));
            steps.push(json!({"tool":"model.enter_edit_mode","params":{"mesh_id":mesh_id}}));
            steps.push(json!({"tool":"model.select","params":{"mesh_id":mesh_id,"mode":"face","selector":{"faces":[0]}}}));
            steps.push(json!({"tool":"model.extrude","params":{"mesh_id":mesh_id,"params":{"distance":0.4}}}));
            steps.push(
                json!({"tool":"model.bevel","params":{"mesh_id":mesh_id,"params":{"width":0.05}}}),
            );
            steps.push(json!({"tool":"model.unwrap_uv","params":{"mesh_id":mesh_id,"method":"angle_based"}}));
            steps.push(json!({"tool":"model.sculpt_brush","params":{"mesh_id":mesh_id,"brush_type":"smooth","params":{"strength":0.3}}}));
            steps.push(json!({"tool":"model.exit_edit_mode","params":{"mesh_id":mesh_id}}));
        } else if lower.contains("water")
            || lower.contains("ocean")
            || lower.contains("river")
            || lower.contains("boat")
            || lower.contains("barco")
        {
            let boat_name = format!("Boat_{}", plan_timestamp);
            steps.push(json!({"tool":"scene.create","params":{"name":"Generated Water Scene"}}));
            steps.push(json!({"tool":"entity.create","params":{"name":boat_name,"mesh":"cube","translation":[0.0,0.5,0.0]}}));
            steps.push(json!({"tool":"water.create_ocean","params":{"ocean_id":"main_ocean","size":768.0,"waves":{"amplitude":1.2,"frequency":0.18,"phase":0.0},"params":{"base_height":0.0}}}));
            steps.push(json!({"tool":"water.set_waves","params":{"ocean_id":"main_ocean","params":{"amplitude":1.4,"frequency":0.2,"phase":0.4}}}));
            steps.push(json!({"tool":"water.enable_foam","params":{"ocean_id":"main_ocean","params":{"amount":0.6}}}));
            steps.push(json!({"tool":"water.enable_refraction","params":{"ocean_id":"main_ocean","params":{"strength":0.4}}}));
            steps.push(json!({"tool":"water.enable_caustics","params":{"ocean_id":"main_ocean","params":{"strength":0.5}}}));
            steps.push(json!({"tool":"water.add_buoyancy","params":{"entity_id":boat_name,"params":{"float_height":0.7,"stiffness":0.9}}}));
            steps.push(json!({"tool":"water.add_drag","params":{"entity_id":boat_name,"params":{"linear":0.2,"angular":0.15}}}));
        } else if lower.contains("vfx")
            || lower.contains("particle")
            || lower.contains("humo")
            || lower.contains("smoke")
            || lower.contains("magic")
        {
            steps.push(json!({"tool":"vfx.create_particle_system","params":{"name":"fx_smoke","particle_id":"fx_smoke","params":{"max_particles":4096}}}));
            steps.push(json!({"tool":"vfx.set_emitter","params":{"particle_id":"fx_smoke","params":{"rate":120.0,"lifetime":2.5}}}));
            steps.push(json!({"tool":"vfx.set_forces","params":{"particle_id":"fx_smoke","params":{"gravity":[0.0,0.5,0.0],"wind":[0.2,0.0,0.0]}}}));
            steps.push(json!({"tool":"vfx.set_renderer","params":{"particle_id":"fx_smoke","params":{"mode":"billboard","material":"smoke_mat"}}}));
            steps.push(json!({"tool":"vfx.create_graph","params":{"name":"fx_graph","graph_id":"fx_graph"}}));
            steps.push(json!({"tool":"vfx.add_node","params":{"graph_id":"fx_graph","node_type":"SpawnBurst","node_id":"spawn","params":{"count":48}}}));
            steps.push(json!({"tool":"vfx.add_node","params":{"graph_id":"fx_graph","node_type":"ApplyForce","node_id":"force","params":{"x":0.1,"y":0.8,"z":0.0}}}));
            steps.push(json!({"tool":"vfx.connect","params":{"graph_id":"fx_graph","out_node":"spawn","in_node":"force"}}));
            steps.push(json!({"tool":"vfx.compile_graph","params":{"graph_id":"fx_graph"}}));
        } else {
            steps.push(
                json!({"tool":"scene.create","params":{"name":format!("Generated Scene: {}", prompt)}}),
            );
            steps.push(json!({"tool":"entity.create","params":{"name":"RootProp","mesh":"cube","translation":[0.0,0.0,0.0]}}));
        }

        Ok(json!({
            "plan_id": format!("plan-{}", plan_timestamp),
            "prompt": prompt,
            "steps": steps
        }))
    }

    fn gen_execute_plan(&mut self, params: &Value) -> anyhow::Result<Value> {
        let task_graph = params.get("task_graph").unwrap_or(params);
        let steps = task_graph
            .get("steps")
            .and_then(Value::as_array)
            .with_context(|| "task_graph.steps must be an array")?;
        let auto_transaction = optional_bool(params, "auto_transaction")
            .or_else(|| task_graph.get("auto_transaction").and_then(Value::as_bool))
            .unwrap_or(true);

        if steps.len() > 128 {
            bail!("task_graph has too many steps (max 128)");
        }

        let existing_txn = self
            .command_bus
            .current_transaction_name()
            .map(|name| name.to_string());
        let mut started_txn_name: Option<String> = None;
        if auto_transaction && existing_txn.is_none() {
            let txn_name = format!("gen.execute_plan.{}", chrono::Utc::now().timestamp_millis());
            self.command_bus.begin_transaction(txn_name.clone())?;
            started_txn_name = Some(txn_name);
        }

        let mut step_results = Vec::with_capacity(steps.len());
        for (index, step) in steps.iter().enumerate() {
            let tool = step
                .get("tool")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|tool| !tool.is_empty())
                .with_context(|| format!("task_graph.steps[{}].tool is required", index))?;

            if tool.starts_with("gen.") {
                bail!(
                    "nested gen.* execution is not allowed in plans (step {})",
                    index
                );
            }
            if tool == "build.build_and_run" && started_txn_name.is_some() {
                let message = "build.build_and_run is not rollback-safe inside auto_transaction plans; set auto_transaction=false to allow this step";
                step_results.push(json!({
                    "index": index,
                    "tool": tool,
                    "status": "error",
                    "error": message
                }));
                let rollback_result = match self.command_bus.rollback_transaction() {
                    Ok(rolled_back_commands) => json!({
                        "attempted": true,
                        "status": "ok",
                        "rolled_back_commands": rolled_back_commands
                    }),
                    Err(rollback_err) => json!({
                        "attempted": true,
                        "status": "error",
                        "error": rollback_err.to_string()
                    }),
                };
                return Ok(json!({
                    "status": "error",
                    "failed_step": index,
                    "results": step_results,
                    "transaction": {
                        "auto_transaction": auto_transaction,
                        "started": started_txn_name.is_some(),
                        "active_before": existing_txn,
                        "started_name": started_txn_name,
                        "rollback": rollback_result
                    }
                }));
            }

            let step_params = step.get("params").cloned().unwrap_or_else(|| json!({}));
            match self.execute(tool, step_params.clone()) {
                Ok(result) => {
                    step_results.push(json!({
                        "index": index,
                        "tool": tool,
                        "status": "ok",
                        "result": result
                    }));
                }
                Err(err) => {
                    step_results.push(json!({
                        "index": index,
                        "tool": tool,
                        "status": "error",
                        "error": err.to_string(),
                        "params": step_params
                    }));
                    let rollback_result = if started_txn_name.is_some() {
                        match self.command_bus.rollback_transaction() {
                            Ok(rolled_back_commands) => json!({
                                "attempted": true,
                                "status": "ok",
                                "rolled_back_commands": rolled_back_commands
                            }),
                            Err(rollback_err) => json!({
                                "attempted": true,
                                "status": "error",
                                "error": rollback_err.to_string()
                            }),
                        }
                    } else {
                        json!({
                            "attempted": false
                        })
                    };
                    return Ok(json!({
                        "status": "error",
                        "failed_step": index,
                        "results": step_results,
                        "transaction": {
                            "auto_transaction": auto_transaction,
                            "started": started_txn_name.is_some(),
                            "active_before": existing_txn,
                            "started_name": started_txn_name,
                            "rollback": rollback_result
                        }
                    }));
                }
            }
        }

        let commit_result = if let Some(started_name) = started_txn_name.clone() {
            let committed_commands = self.command_bus.commit_transaction()?;
            json!({
                "attempted": true,
                "status": "ok",
                "transaction_name": started_name,
                "committed_commands": committed_commands
            })
        } else {
            json!({
                "attempted": false
            })
        };

        Ok(json!({
            "status": "ok",
            "executed_steps": step_results.len(),
            "results": step_results,
            "transaction": {
                "auto_transaction": auto_transaction,
                "started": started_txn_name.is_some(),
                "active_before": existing_txn,
                "started_name": started_txn_name,
                "commit": commit_result
            }
        }))
    }

    fn gen_validate_gameplay(&self, params: &Value) -> anyhow::Result<Value> {
        let requirements = params
            .get("min_requirements")
            .or_else(|| params.get("requirements"))
            .unwrap_or(params);

        let min_entities = optional_usize(requirements, "min_entities").unwrap_or(1);
        let required_entities = optional_string_array(requirements, "required_entities");
        let required_assets = optional_string_array(requirements, "required_assets");
        let target_fps = optional_f32(requirements, "target_fps");
        let required_components =
            optional_required_components(requirements.get("required_components"))?;

        let context = self.command_bus.context();
        let mut checks = Vec::<Value>::new();
        let mut passed = true;

        let entity_count = context.scene.entities.len();
        let entity_count_ok = entity_count >= min_entities;
        passed &= entity_count_ok;
        checks.push(json!({
            "name": "min_entities",
            "passed": entity_count_ok,
            "required": min_entities,
            "actual": entity_count,
            "message": if entity_count_ok {
                format!("entity count {} meets minimum {}", entity_count, min_entities)
            } else {
                format!("entity count {} is below minimum {}", entity_count, min_entities)
            }
        }));

        let mut missing_entities = Vec::<String>::new();
        for entity in &required_entities {
            if !context.entity_exists(entity) {
                missing_entities.push(entity.clone());
            }
        }
        let entities_ok = missing_entities.is_empty();
        passed &= entities_ok;
        checks.push(json!({
            "name": "required_entities",
            "passed": entities_ok,
            "required": required_entities,
            "missing": missing_entities
        }));

        let mut missing_assets = Vec::<String>::new();
        for asset in &required_assets {
            let imported_known = context.imported_assets.contains_key(asset);
            let resolved = resolve_project_path(&context.project_root, Path::new(asset));
            if !imported_known && !resolved.exists() {
                missing_assets.push(asset.clone());
            }
        }
        let assets_ok = missing_assets.is_empty();
        passed &= assets_ok;
        checks.push(json!({
            "name": "required_assets",
            "passed": assets_ok,
            "required": required_assets,
            "missing": missing_assets
        }));

        let mut missing_components = Vec::<Value>::new();
        for (entity_name, components) in &required_components {
            let bucket = context.components.get(entity_name);
            for component in components {
                let has_component = bucket
                    .map(|components_map| components_map.contains_key(component))
                    .unwrap_or(false);
                if !has_component {
                    missing_components.push(json!({
                        "entity_id": entity_name,
                        "component_type": component
                    }));
                }
            }
        }
        let components_ok = missing_components.is_empty();
        passed &= components_ok;
        checks.push(json!({
            "name": "required_components",
            "passed": components_ok,
            "required": required_components,
            "missing": missing_components
        }));

        if let Some(target_fps) = target_fps {
            let actual_fps = context.engine_state.fps;
            let fps_ok = actual_fps >= target_fps;
            passed &= fps_ok;
            checks.push(json!({
                "name": "target_fps",
                "passed": fps_ok,
                "required": target_fps,
                "actual": actual_fps
            }));
        }

        Ok(json!({
            "status": if passed { "ok" } else { "error" },
            "passed": passed,
            "scene": {
                "name": context.scene.name,
                "entity_count": context.scene.entities.len(),
                "revision": context.revision
            },
            "checks": checks
        }))
    }

    fn build_build_and_run(&self, params: &Value) -> anyhow::Result<Value> {
        let target = optional_string(params, "target").unwrap_or_else(|| "editor".to_string());
        let profile = optional_string(params, "profile").unwrap_or_else(|| "debug".to_string());
        let run = optional_bool(params, "run").unwrap_or(true);
        let dry_run = optional_bool(params, "dry_run").unwrap_or(true);
        let wait_for_run = optional_bool(params, "wait_for_run").unwrap_or(false);

        if !matches!(profile.to_ascii_lowercase().as_str(), "debug" | "release") {
            bail!("build profile must be 'debug' or 'release'");
        }

        let mut build_args = vec!["build".to_string(), "-p".to_string(), target.clone()];
        let mut run_args = vec!["run".to_string(), "-p".to_string(), target.clone()];
        if profile.eq_ignore_ascii_case("release") {
            build_args.push("--release".to_string());
            run_args.push("--release".to_string());
        }

        let project_root = &self.command_bus.context().project_root;
        if dry_run {
            return Ok(json!({
                "status": "dry_run",
                "project_root": project_root.display().to_string(),
                "steps": {
                    "build": {
                        "program": "cargo",
                        "args": build_args
                    },
                    "run": if run {
                        json!({
                            "program": "cargo",
                            "args": run_args,
                            "wait_for_run": wait_for_run
                        })
                    } else {
                        json!(null)
                    }
                }
            }));
        }

        let build_output = ProcessCommand::new("cargo")
            .current_dir(project_root)
            .args(&build_args)
            .output()
            .with_context(|| "failed to execute cargo build command")?;
        let build_status_code = build_output.status.code().unwrap_or(-1);
        let build_stdout = truncated_output(&build_output.stdout);
        let build_stderr = truncated_output(&build_output.stderr);
        if !build_output.status.success() {
            return Ok(json!({
                "status": "error",
                "step": "build",
                "exit_code": build_status_code,
                "stdout": build_stdout,
                "stderr": build_stderr
            }));
        }

        if !run {
            return Ok(json!({
                "status": "ok",
                "step": "build",
                "exit_code": build_status_code,
                "stdout": build_stdout,
                "stderr": build_stderr
            }));
        }

        if wait_for_run {
            let run_output = ProcessCommand::new("cargo")
                .current_dir(project_root)
                .args(&run_args)
                .output()
                .with_context(|| "failed to execute cargo run command")?;
            return Ok(json!({
                "status": if run_output.status.success() { "ok" } else { "error" },
                "step": "run",
                "exit_code": run_output.status.code().unwrap_or(-1),
                "build": {
                    "exit_code": build_status_code
                },
                "stdout": truncated_output(&run_output.stdout),
                "stderr": truncated_output(&run_output.stderr)
            }));
        }

        let child = ProcessCommand::new("cargo")
            .current_dir(project_root)
            .args(&run_args)
            .spawn()
            .with_context(|| "failed to spawn cargo run command")?;
        Ok(json!({
            "status": "ok",
            "step": "run_spawned",
            "build": {
                "exit_code": build_status_code
            },
            "pid": child.id()
        }))
    }

    fn history_undo(&mut self, params: &Value) -> anyhow::Result<Value> {
        let steps = optional_usize(params, "steps").unwrap_or(1).max(1);
        let undone = self.command_bus.history_undo(steps)?;
        Ok(json!({
            "undone_steps": undone
        }))
    }

    fn history_redo(&mut self, params: &Value) -> anyhow::Result<Value> {
        let steps = optional_usize(params, "steps").unwrap_or(1).max(1);
        let redone = self.command_bus.history_redo(steps)?;
        Ok(json!({
            "redone_steps": redone
        }))
    }

    fn history_mark(&mut self, params: &Value) -> anyhow::Result<Value> {
        let label = required_string(params, "label")?;
        self.command_bus.history_mark(label.clone());
        Ok(json!({
            "mark": label
        }))
    }

    fn history_jump_to(&mut self, params: &Value) -> anyhow::Result<Value> {
        let label = required_string(params, "label")?;
        self.command_bus.history_jump_to(&label)?;
        Ok(json!({
            "jumped_to": label
        }))
    }
}

fn command_receipt_to_json(receipt: crate::command_bus::CommandReceipt) -> Value {
    json!({
        "command_id": receipt.command_id,
        "status": receipt.status,
        "result": receipt.result
    })
}

fn normalize_scene_path(input: &str) -> PathBuf {
    let mut path = PathBuf::from(input);
    if path.extension().is_none() {
        path.set_extension("json");
    }
    if path.components().count() == 1 {
        path = PathBuf::from("samples").join(path);
    }
    path
}

fn default_scene_duplicate_path(source_path: &Path) -> PathBuf {
    let parent = source_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("samples"));
    let stem = source_path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("scene");
    let extension = source_path
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("json");
    parent.join(format!("{}_copy.{}", stem, extension))
}

fn postprocess_params_from_preset(
    preset: Option<&str>,
    current: &RenderSettings,
) -> RenderPostprocessParams {
    let preset_id = preset
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());

    match preset_id.as_deref() {
        Some("natural_day") | Some("natural-day") => RenderPostprocessParams {
            exposure: 1.0,
            gamma: 2.2,
            bloom_intensity: 0.18,
            bloom_threshold: 1.1,
            bloom_radius: 1.2,
            fog_density: 0.02,
            fog_color: [0.76, 0.82, 0.90],
            saturation: 1.05,
            contrast: 1.03,
            white_balance: 0.05,
            grade_tint: [1.0, 1.0, 0.98],
            color_grading_preset: "natural_day".to_string(),
        },
        Some("filmic_sunset") | Some("filmic-sunset") | Some("golden_hour") => {
            RenderPostprocessParams {
                exposure: 1.08,
                gamma: 2.15,
                bloom_intensity: 0.32,
                bloom_threshold: 0.95,
                bloom_radius: 1.8,
                fog_density: 0.08,
                fog_color: [0.98, 0.64, 0.45],
                saturation: 1.12,
                contrast: 1.08,
                white_balance: 0.35,
                grade_tint: [1.08, 0.95, 0.86],
                color_grading_preset: "filmic_sunset".to_string(),
            }
        }
        Some("noir_indoor") | Some("noir-indoor") => RenderPostprocessParams {
            exposure: 0.75,
            gamma: 2.4,
            bloom_intensity: 0.12,
            bloom_threshold: 1.35,
            bloom_radius: 1.0,
            fog_density: 0.04,
            fog_color: [0.22, 0.24, 0.28],
            saturation: 0.25,
            contrast: 1.28,
            white_balance: -0.18,
            grade_tint: [0.88, 0.92, 1.02],
            color_grading_preset: "noir_indoor".to_string(),
        },
        _ => RenderPostprocessParams {
            exposure: current.exposure,
            gamma: current.gamma,
            bloom_intensity: current.bloom_intensity,
            bloom_threshold: current.bloom_threshold,
            bloom_radius: current.bloom_radius,
            fog_density: current.fog_density,
            fog_color: current.fog_color,
            saturation: current.saturation,
            contrast: current.contrast,
            white_balance: current.white_balance,
            grade_tint: current.grade_tint,
            color_grading_preset: current.color_grading_preset.clone(),
        },
    }
}

fn required_string(params: &Value, key: &str) -> anyhow::Result<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .with_context(|| format!("missing required string '{}'", key))
}

fn optional_string(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn optional_nested_string(params: &Value, object_key: &str, key: &str) -> Option<String> {
    params
        .get(object_key)
        .and_then(Value::as_object)
        .and_then(|obj| obj.get(key))
        .and_then(Value::as_str)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn optional_bool(params: &Value, key: &str) -> Option<bool> {
    params.get(key).and_then(Value::as_bool)
}

fn optional_nested_bool(params: &Value, object_key: &str, key: &str) -> Option<bool> {
    params
        .get(object_key)
        .and_then(Value::as_object)
        .and_then(|obj| obj.get(key))
        .and_then(Value::as_bool)
}

fn optional_f32(params: &Value, key: &str) -> Option<f32> {
    params
        .get(key)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
}

fn optional_usize(params: &Value, key: &str) -> Option<usize> {
    params
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
}

fn required_value<'a>(params: &'a Value, key: &str) -> anyhow::Result<&'a Value> {
    params
        .get(key)
        .with_context(|| format!("missing required value '{}'", key))
}

fn optional_vec3(params: &Value, key: &str) -> anyhow::Result<Option<[f32; 3]>> {
    let Some(value) = params.get(key) else {
        return Ok(None);
    };
    Ok(Some(parse_vec3(value, key)?))
}

fn optional_nested_vec3(
    params: &Value,
    object_key: &str,
    key: &str,
) -> anyhow::Result<Option<[f32; 3]>> {
    let Some(value) = params
        .get(object_key)
        .and_then(Value::as_object)
        .and_then(|obj| obj.get(key))
    else {
        return Ok(None);
    };
    Ok(Some(parse_vec3(value, key)?))
}

fn optional_transform_position(params: &Value) -> anyhow::Result<Option<[f32; 3]>> {
    let Some(transform) = params.get("transform") else {
        return Ok(None);
    };
    let Some(position) = transform.get("position") else {
        return Ok(None);
    };
    Ok(Some(parse_vec3(position, "transform.position")?))
}

fn parse_vec3(value: &Value, key: &str) -> anyhow::Result<[f32; 3]> {
    let arr = value
        .as_array()
        .with_context(|| format!("'{}' must be an array with 3 numbers", key))?;
    if arr.len() != 3 {
        bail!("'{}' must contain exactly 3 values", key);
    }
    let mut out = [0.0f32; 3];
    for (idx, raw) in arr.iter().enumerate() {
        out[idx] = raw
            .as_f64()
            .map(|v| v as f32)
            .with_context(|| format!("'{}[{}]' must be numeric", key, idx))?;
    }
    Ok(out)
}

fn normalize_vec3(vec: [f32; 3]) -> Option<[f32; 3]> {
    let len_sq = vec[0] * vec[0] + vec[1] * vec[1] + vec[2] * vec[2];
    if len_sq <= f32::EPSILON {
        return None;
    }
    let inv_len = len_sq.sqrt().recip();
    Some([vec[0] * inv_len, vec[1] * inv_len, vec[2] * inv_len])
}

fn ray_aabb_hit_distance(
    origin: [f32; 3],
    dir: [f32; 3],
    center: [f32; 3],
    half_extents: [f32; 3],
) -> Option<f32> {
    let min = [
        center[0] - half_extents[0],
        center[1] - half_extents[1],
        center[2] - half_extents[2],
    ];
    let max = [
        center[0] + half_extents[0],
        center[1] + half_extents[1],
        center[2] + half_extents[2],
    ];
    let mut t_min = 0.0f32;
    let mut t_max = f32::INFINITY;

    for axis in 0..3 {
        let ray_origin = origin[axis];
        let ray_dir = dir[axis];
        if ray_dir.abs() < 1e-6 {
            if ray_origin < min[axis] || ray_origin > max[axis] {
                return None;
            }
            continue;
        }

        let inv = ray_dir.recip();
        let t0 = (min[axis] - ray_origin) * inv;
        let t1 = (max[axis] - ray_origin) * inv;
        let near = t0.min(t1);
        let far = t0.max(t1);
        t_min = t_min.max(near);
        t_max = t_max.min(far);
        if t_max < t_min {
            return None;
        }
    }

    if t_max < 0.0 {
        None
    } else {
        Some(t_min.max(0.0))
    }
}

fn aabb_overlap(
    center_a: [f32; 3],
    half_a: [f32; 3],
    center_b: [f32; 3],
    half_b: [f32; 3],
) -> bool {
    (center_a[0] - center_b[0]).abs() <= (half_a[0] + half_b[0])
        && (center_a[1] - center_b[1]).abs() <= (half_a[1] + half_b[1])
        && (center_a[2] - center_b[2]).abs() <= (half_a[2] + half_b[2])
}

fn sphere_aabb_intersects(
    sphere_center: [f32; 3],
    sphere_radius: f32,
    aabb_center: [f32; 3],
    aabb_half_extents: [f32; 3],
) -> bool {
    let aabb_min = [
        aabb_center[0] - aabb_half_extents[0],
        aabb_center[1] - aabb_half_extents[1],
        aabb_center[2] - aabb_half_extents[2],
    ];
    let aabb_max = [
        aabb_center[0] + aabb_half_extents[0],
        aabb_center[1] + aabb_half_extents[1],
        aabb_center[2] + aabb_half_extents[2],
    ];
    let mut closest = [0.0f32; 3];
    for axis in 0..3 {
        closest[axis] = sphere_center[axis].clamp(aabb_min[axis], aabb_max[axis]);
    }
    let delta = [
        sphere_center[0] - closest[0],
        sphere_center[1] - closest[1],
        sphere_center[2] - closest[2],
    ];
    let distance_sq = delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2];
    distance_sq <= sphere_radius * sphere_radius
}

fn required_string_array(params: &Value, key: &str) -> anyhow::Result<Vec<String>> {
    let arr = params
        .get(key)
        .and_then(Value::as_array)
        .with_context(|| format!("missing required array '{}'", key))?;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        let value = item
            .as_str()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .with_context(|| format!("'{}[{}]' must be a non-empty string", key, idx))?;
        out.push(value.to_string());
    }
    Ok(out)
}

fn optional_string_array(params: &Value, key: &str) -> Vec<String> {
    params
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<String>>()
        })
        .unwrap_or_default()
}

fn optional_required_components(
    value: Option<&Value>,
) -> anyhow::Result<Vec<(String, Vec<String>)>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let Some(map) = value.as_object() else {
        bail!("required_components must be an object mapping entity_id to component list");
    };
    let mut out = Vec::with_capacity(map.len());
    for (entity_id, components_value) in map {
        let Some(components) = components_value.as_array() else {
            bail!(
                "required_components.{} must be an array of component names",
                entity_id
            );
        };
        let names = components
            .iter()
            .filter_map(|component| component.as_str())
            .map(str::trim)
            .filter(|component| !component.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<String>>();
        out.push((entity_id.clone(), names));
    }
    Ok(out)
}

fn truncated_output(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes).trim().to_string();
    const MAX_CHARS: usize = 2000;
    if text.chars().count() <= MAX_CHARS {
        text
    } else {
        text.chars().take(MAX_CHARS).collect::<String>()
    }
}

fn collect_project_tree(project_root: &Path, max_entries: usize) -> anyhow::Result<Value> {
    let mut directories = Vec::new();
    let mut files = Vec::new();
    let mut stack = vec![project_root.to_path_buf()];
    let mut truncated = false;

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path
                .strip_prefix(project_root)
                .unwrap_or(path.as_path())
                .to_path_buf();
            let rel_str = rel.to_string_lossy().replace('\\', "/");
            if rel_str.is_empty() {
                continue;
            }
            if path.is_dir() {
                if rel_str == ".git"
                    || rel_str.starts_with(".git/")
                    || rel_str.starts_with("target/")
                {
                    continue;
                }
                directories.push(rel_str.clone());
                if directories.len() + files.len() >= max_entries {
                    truncated = true;
                    break;
                }
                stack.push(path);
            } else {
                files.push(rel_str);
                if directories.len() + files.len() >= max_entries {
                    truncated = true;
                    break;
                }
            }
        }
        if truncated {
            break;
        }
    }

    directories.sort();
    files.sort();
    Ok(json!({
        "project_root": project_root.display().to_string(),
        "directories": directories,
        "files": files,
        "truncated": truncated
    }))
}

fn collect_matching_files(
    project_root: &Path,
    root: &Path,
    query: &str,
    max_results: usize,
    out: &mut Vec<String>,
) {
    if !root.exists() {
        return;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if !file_name.contains(query) {
                continue;
            }
            let rel = path
                .strip_prefix(project_root)
                .unwrap_or(path.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            out.push(rel);
            if out.len() >= max_results {
                return;
            }
        }
    }
}

fn default_entity_name_from_asset_id(asset_id: &str) -> String {
    let file_name = Path::new(asset_id)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.trim().is_empty())
        .unwrap_or("ImportedAsset");
    let mut out = String::with_capacity(file_name.len());
    for ch in file_name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch);
        } else if ch.is_ascii_whitespace() {
            out.push('_');
        }
    }
    if out.trim_matches('_').is_empty() {
        "ImportedAsset".to_string()
    } else {
        out.trim_matches('_').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn gen_execute_plan_rolls_back_when_a_step_fails() {
        let mut runtime = ToolRuntime::new(".");
        let result = runtime
            .execute(
                "gen.execute_plan",
                json!({
                    "task_graph": {
                        "steps": [
                            { "tool": "scene.create", "params": { "name": "Txn Scene" } },
                            { "tool": "entity.create", "params": { "name": "Hero", "mesh": "cube", "translation": [0.0, 0.0, 0.0] } },
                            { "tool": "entity.create", "params": { "name": "Hero", "mesh": "cube", "translation": [1.0, 0.0, 0.0] } }
                        ]
                    }
                }),
            )
            .expect("gen.execute_plan should return status payload");

        assert_eq!(result.get("status").and_then(Value::as_str), Some("error"));
        assert_eq!(
            result
                .get("transaction")
                .and_then(|txn| txn.get("rollback"))
                .and_then(|rollback| rollback.get("status"))
                .and_then(Value::as_str),
            Some("ok")
        );

        let snapshot = runtime.scene_snapshot();
        assert_eq!(snapshot.name, "Empty Scene");
        assert!(snapshot.entities.is_empty());
    }

    #[test]
    fn gen_validate_gameplay_reports_missing_requirements() {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "Validation Scene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Player",
                    "mesh": "cube",
                    "translation": [0.0, 0.0, 0.0]
                }),
            )
            .expect("entity.create should succeed");

        let result = runtime
            .execute(
                "gen.validate_gameplay",
                json!({
                    "min_requirements": {
                        "min_entities": 2,
                        "required_entities": ["Player", "Enemy"],
                        "required_components": {
                            "Player": ["Health"]
                        }
                    }
                }),
            )
            .expect("gen.validate_gameplay should succeed");

        assert_eq!(result.get("status").and_then(Value::as_str), Some("error"));
        assert_eq!(result.get("passed").and_then(Value::as_bool), Some(false));
    }

    #[test]
    fn build_build_and_run_dry_run_returns_commands() {
        let mut runtime = ToolRuntime::new(".");
        let result = runtime
            .execute(
                "build.build_and_run",
                json!({
                    "target": "editor",
                    "profile": "debug",
                    "run": true,
                    "dry_run": true
                }),
            )
            .expect("build.build_and_run dry_run should succeed");

        assert_eq!(
            result.get("status").and_then(Value::as_str),
            Some("dry_run")
        );
        assert_eq!(
            result
                .get("steps")
                .and_then(|steps| steps.get("build"))
                .and_then(|build| build.get("program"))
                .and_then(Value::as_str),
            Some("cargo")
        );
    }

    #[test]
    fn registry_contains_phase3_to_phase27_tools() {
        let runtime = ToolRuntime::new(".");
        let names = runtime
            .list_tools()
            .into_iter()
            .map(|schema| schema.name)
            .collect::<HashSet<String>>();
        for required in [
            "tool.get_engine_state",
            "tool.get_cycle_context",
            "tool.get_project_tree",
            "tool.search_assets",
            "tool.read_asset_metadata",
            "tool.get_selection",
            "tool.set_selection",
            "tool.get_viewport_camera",
            "tool.set_viewport_camera",
            "tool.get_rules",
            "tool.get_project_memory",
            "tool.set_project_memory",
            "tool.get_constraints",
            "tool.set_constraints",
            "tool.set_objective",
            "tool.get_diagnostics",
            "tool.clear_diagnostics",
            "tool.begin_transaction",
            "tool.commit_transaction",
            "tool.rollback_transaction",
            "tool.create_checkpoint",
            "tool.rollback_to_checkpoint",
            "tool.log",
            "tool.open_task",
            "tool.update_task",
            "tool.close_task",
            "scene.create",
            "scene.open",
            "scene.save",
            "scene.save_as",
            "scene.duplicate",
            "scene.close",
            "scene.set_sky",
            "scene.set_time_of_day",
            "scene.add_fog",
            "scene.add_postprocess",
            "scene.enable_world_streaming",
            "scene.create_stream_chunk",
            "scene.assign_entity_to_chunk",
            "entity.create",
            "entity.clone",
            "entity.delete",
            "entity.rename",
            "entity.parent",
            "entity.unparent",
            "entity.find_by_name",
            "entity.find_by_tag",
            "entity.set_transform",
            "entity.translate",
            "entity.rotate",
            "entity.scale",
            "entity.get_transform",
            "entity.add_component",
            "entity.remove_component",
            "entity.get_component",
            "entity.set_component",
            "render.set_lod_settings",
            "graph.create",
            "graph.add_node",
            "graph.connect",
            "graph.delete_node",
            "graph.delete_edge",
            "graph.set_node_params",
            "graph.validate",
            "graph.run",
            "template.list",
            "template.apply",
            "asset.import_file",
            "asset.import_url",
            "asset.create_material",
            "asset.create_texture",
            "asset.create_shader",
            "asset.create_prefab",
            "asset.save_prefab",
            "asset.instantiate_prefab",
            "asset.rebuild_import",
            "asset.generate_lods",
            "asset.mesh_optimize",
            "asset.compress_textures",
            "asset.bake_lightmaps",
            "asset.bake_reflection_probes",
            "asset.get_template_bundle",
            "asset.validate_template_bundle",
            "asset.assign_material",
            "phys.add_collider",
            "phys.set_collider",
            "phys.remove_collider",
            "phys.add_rigidbody",
            "phys.set_mass",
            "phys.set_friction",
            "phys.set_restitution",
            "phys.apply_force",
            "phys.apply_impulse",
            "phys.set_gravity",
            "phys.raycast",
            "phys.overlap",
            "phys.add_character_controller",
            "phys.character_move",
            "phys.character_jump",
            "phys.character_set_state",
            "game.create_input_action",
            "game.bind_action",
            "game.set_rebind",
            "game.create_weapon",
            "game.attach_weapon",
            "game.fire_weapon",
            "game.apply_damage",
            "game.add_health_component",
            "game.add_trigger",
            "game.add_pickup",
            "game.add_inventory",
            "game.add_interactable",
            "anim.add_animator",
            "anim.create_state_machine",
            "anim.add_state",
            "anim.add_transition",
            "anim.set_parameter",
            "anim.play",
            "anim.blend",
            "anim.add_ik",
            "anim.retarget",
            "anim.bake_animation",
            "model.create_primitive",
            "model.enter_edit_mode",
            "model.exit_edit_mode",
            "model.select",
            "model.extrude",
            "model.inset",
            "model.bevel",
            "model.loop_cut",
            "model.knife",
            "model.merge",
            "model.subdivide",
            "model.triangulate",
            "model.add_modifier",
            "model.set_modifier",
            "model.apply_modifier",
            "model.remove_modifier",
            "model.unwrap_uv",
            "model.pack_uv",
            "model.generate_lightmap_uv",
            "model.voxel_remesh",
            "model.decimate",
            "model.smooth",
            "model.sculpt_brush",
            "model.sculpt_mask",
            "vfx.create_particle_system",
            "vfx.set_emitter",
            "vfx.set_forces",
            "vfx.set_collision",
            "vfx.set_renderer",
            "vfx.attach_to_entity",
            "vfx.create_graph",
            "vfx.add_node",
            "vfx.connect",
            "vfx.compile_graph",
            "water.create_ocean",
            "water.create_river",
            "water.create_waterfall",
            "water.set_waves",
            "water.enable_foam",
            "water.enable_refraction",
            "water.enable_caustics",
            "water.add_buoyancy",
            "water.add_drag",
            "water.sample_height",
            "mount.create_horse_template",
            "mount.spawn_horse",
            "mount.mount_rider",
            "mount.dismount",
            "mount.set_gait",
            "mount.set_path_follow",
            "ai.create_navmesh",
            "ai.bake_navmesh",
            "ai.add_agent",
            "ai.set_destination",
            "ai.create_behavior_tree",
            "ai.bt_add_node",
            "ai.bt_connect",
            "ai.assign_behavior",
            "ai.set_blackboard",
            "ui.create_canvas",
            "ui.add_panel",
            "ui.add_text",
            "ui.add_button",
            "ui.bind_to_data",
            "ui.create_hud_template",
            "audio.import_clip",
            "audio.create_source",
            "audio.play",
            "audio.set_spatial",
            "audio.create_mixer",
            "audio.route",
            "net.create_server",
            "net.connect_client",
            "net.enable_replication",
            "net.set_prediction",
            "net.set_rollback",
            "build.set_target",
            "build.set_bundle_id",
            "build.set_version",
            "build.enable_feature",
            "build.export_project",
            "build.generate_installer",
            "render.assign_material",
            "gen.create_game_from_template",
            "gen.create_platformer_level",
            "gen.create_shooter_arena",
            "gen.create_island_adventure",
            "gen.plan_from_prompt",
            "gen.execute_plan",
            "gen.validate_gameplay",
            "gen.package_demo_build",
            "build.build_and_run",
            "debug.show_colliders",
            "debug.show_navmesh",
            "debug.toggle_wireframe",
            "debug.capture_frame",
            "debug.get_profiler_snapshot",
            "debug.find_performance_hotspots",
        ] {
            assert!(
                names.contains(required),
                "missing required tool in phase3/phase27 contract: {}",
                required
            );
        }
    }

    #[test]
    fn scene_phase4_tools_update_scene_runtime_state() {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "Phase4 Scene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Hero",
                    "mesh": "cube",
                    "translation": [0.0, 0.0, 0.0]
                }),
            )
            .expect("entity.create should succeed");
        runtime
            .execute(
                "scene.set_sky",
                json!({
                    "preset": "sunset_cinematic"
                }),
            )
            .expect("scene.set_sky should succeed");
        runtime
            .execute(
                "scene.set_time_of_day",
                json!({
                    "value": 18.5
                }),
            )
            .expect("scene.set_time_of_day should succeed");
        runtime
            .execute(
                "scene.add_fog",
                json!({
                    "density": 0.03,
                    "color": [0.8, 0.7, 0.6],
                    "start": 4.0,
                    "end": 60.0
                }),
            )
            .expect("scene.add_fog should succeed");
        runtime
            .execute(
                "scene.enable_world_streaming",
                json!({
                    "chunksize": 32.0,
                    "range": 3
                }),
            )
            .expect("scene.enable_world_streaming should succeed");
        runtime
            .execute(
                "scene.create_stream_chunk",
                json!({
                    "chunk_id": "chunk_a",
                    "center": [0.0, 0.0, 0.0],
                    "radius": 16.0
                }),
            )
            .expect("scene.create_stream_chunk should succeed");
        runtime
            .execute(
                "scene.assign_entity_to_chunk",
                json!({
                    "entity_id": "Hero",
                    "chunk_id": "chunk_a"
                }),
            )
            .expect("scene.assign_entity_to_chunk should succeed");

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert_eq!(
            state
                .get("scene")
                .and_then(|scene| scene.get("sky_preset"))
                .and_then(Value::as_str),
            Some("sunset_cinematic")
        );
        assert_eq!(
            state
                .get("scene")
                .and_then(|scene| scene.get("time_of_day"))
                .and_then(Value::as_f64),
            Some(18.5)
        );
        assert_eq!(
            state
                .get("scene")
                .and_then(|scene| scene.get("world_streaming"))
                .and_then(|streaming| streaming.get("enabled"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            state
                .get("scene")
                .and_then(|scene| scene.get("world_streaming"))
                .and_then(|streaming| streaming.get("entity_to_chunk"))
                .and_then(|assignments| assignments.get("Hero"))
                .and_then(Value::as_str),
            Some("chunk_a")
        );
    }

    #[test]
    fn phase6_and_phase7_tools_update_physics_and_gameplay_state() {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "Phase67 Scene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Hero67",
                    "mesh": "capsule",
                    "translation": [0.0, 1.0, 0.0]
                }),
            )
            .expect("entity.create should succeed");
        runtime
            .execute(
                "phys.set_gravity",
                json!({
                    "gravity": [0.0, -12.0, 0.0]
                }),
            )
            .expect("phys.set_gravity should succeed");
        runtime
            .execute(
                "phys.add_collider",
                json!({
                    "entity_id": "Hero67",
                    "shape": "capsule",
                    "size": [0.8, 1.8, 0.8]
                }),
            )
            .expect("phys.add_collider should succeed");
        runtime
            .execute(
                "phys.add_rigidbody",
                json!({
                    "entity_id": "Hero67",
                    "type": "dynamic",
                    "mass": 70.0,
                    "friction": 0.8,
                    "restitution": 0.1
                }),
            )
            .expect("phys.add_rigidbody should succeed");

        let raycast = runtime
            .execute(
                "phys.raycast",
                json!({
                    "origin": [0.0, 1.0, -4.0],
                    "dir": [0.0, 0.0, 1.0],
                    "maxdist": 20.0
                }),
            )
            .expect("phys.raycast should succeed");
        assert_eq!(raycast.get("hit").and_then(Value::as_bool), Some(true));
        assert_eq!(
            raycast
                .get("closest_hit")
                .and_then(|hit| hit.get("entity_id"))
                .and_then(Value::as_str),
            Some("Hero67")
        );

        runtime
            .execute(
                "phys.apply_impulse",
                json!({
                    "entity_id": "Hero67",
                    "impulse": [0.0, 0.0, 10.0]
                }),
            )
            .expect("phys.apply_impulse should succeed");
        runtime
            .execute(
                "game.create_weapon",
                json!({
                    "weapon_id": "rifle67",
                    "rate": 9.0,
                    "recoil": 1.0,
                    "spread": 0.04,
                    "ammo_capacity": 12
                }),
            )
            .expect("game.create_weapon should succeed");
        runtime
            .execute(
                "game.attach_weapon",
                json!({
                    "character_id": "Hero67",
                    "weapon_id": "rifle67"
                }),
            )
            .expect("game.attach_weapon should succeed");
        runtime
            .execute(
                "game.add_health_component",
                json!({
                    "entity_id": "Hero67",
                    "max_health": 120.0,
                    "current_health": 120.0
                }),
            )
            .expect("game.add_health_component should succeed");
        runtime
            .execute(
                "game.fire_weapon",
                json!({
                    "character_id": "Hero67"
                }),
            )
            .expect("game.fire_weapon should succeed");
        runtime
            .execute(
                "game.apply_damage",
                json!({
                    "target_id": "Hero67",
                    "amount": 25.0,
                    "damage_type": "bullet"
                }),
            )
            .expect("game.apply_damage should succeed");

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert_eq!(
            state
                .get("physics")
                .and_then(|physics| physics.get("gravity"))
                .and_then(Value::as_array)
                .and_then(|gravity| gravity.get(1))
                .and_then(Value::as_f64),
            Some(-12.0)
        );
        assert_eq!(
            state
                .get("physics")
                .and_then(|physics| physics.get("collider_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("physics")
                .and_then(|physics| physics.get("rigidbody_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("gameplay")
                .and_then(|gameplay| gameplay.get("weapon_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("gameplay")
                .and_then(|gameplay| gameplay.get("attachment_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("gameplay")
                .and_then(|gameplay| gameplay.get("fire_events"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("gameplay")
                .and_then(|gameplay| gameplay.get("weapons"))
                .and_then(|weapons| weapons.get("rifle67"))
                .and_then(|weapon| weapon.get("ammo_current"))
                .and_then(Value::as_u64),
            Some(11)
        );
        assert_eq!(
            state
                .get("gameplay")
                .and_then(|gameplay| gameplay.get("total_damage_applied"))
                .and_then(Value::as_f64),
            Some(25.0)
        );
        assert_eq!(
            state
                .get("gameplay")
                .and_then(|gameplay| gameplay.get("attachments"))
                .and_then(|attachments| attachments.get("Hero67"))
                .and_then(Value::as_str),
            Some("rifle67")
        );
        assert_eq!(
            state
                .get("scene")
                .and_then(|scene| scene.get("last_message"))
                .and_then(Value::as_str),
            Some("'Hero67' received 25.0 bullet damage")
        );
    }

    #[test]
    fn phase8_and_phase9_tools_extend_physics_and_gameplay_baseline() {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "Phase89 Scene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Hero89",
                    "mesh": "capsule",
                    "translation": [0.0, 1.0, 0.0]
                }),
            )
            .expect("entity.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Pickup89",
                    "mesh": "cube",
                    "translation": [1.0, 0.0, 0.0]
                }),
            )
            .expect("entity.create should succeed");
        runtime
            .execute(
                "phys.add_collider",
                json!({
                    "entity_id": "Hero89",
                    "shape": "capsule",
                    "size": [0.8, 1.8, 0.8]
                }),
            )
            .expect("phys.add_collider should succeed");
        runtime
            .execute(
                "phys.add_rigidbody",
                json!({
                    "entity_id": "Hero89",
                    "type": "dynamic",
                    "mass": 80.0,
                    "friction": 0.7,
                    "restitution": 0.1
                }),
            )
            .expect("phys.add_rigidbody should succeed");
        runtime
            .execute(
                "phys.set_mass",
                json!({
                    "entity_id": "Hero89",
                    "mass": 85.0
                }),
            )
            .expect("phys.set_mass should succeed");
        runtime
            .execute(
                "phys.set_friction",
                json!({
                    "entity_id": "Hero89",
                    "value": 0.9
                }),
            )
            .expect("phys.set_friction should succeed");
        runtime
            .execute(
                "phys.set_restitution",
                json!({
                    "entity_id": "Hero89",
                    "value": 0.2
                }),
            )
            .expect("phys.set_restitution should succeed");
        runtime
            .execute(
                "phys.apply_force",
                json!({
                    "entity_id": "Hero89",
                    "force": [0.0, 0.0, 12.0],
                    "dt": 0.016
                }),
            )
            .expect("phys.apply_force should succeed");
        runtime
            .execute(
                "phys.add_character_controller",
                json!({
                    "entity_id": "Hero89",
                    "radius": 0.42,
                    "height": 1.85,
                    "speed": 5.5,
                    "jump_strength": 6.9
                }),
            )
            .expect("phys.add_character_controller should succeed");
        runtime
            .execute(
                "phys.character_move",
                json!({
                    "entity_id": "Hero89",
                    "input": [1.0, 0.0, 0.0],
                    "dt": 0.05
                }),
            )
            .expect("phys.character_move should succeed");
        runtime
            .execute(
                "phys.character_jump",
                json!({
                    "entity_id": "Hero89",
                    "strength": 7.1
                }),
            )
            .expect("phys.character_jump should succeed");
        runtime
            .execute(
                "phys.character_set_state",
                json!({
                    "entity_id": "Hero89",
                    "state": "mounted"
                }),
            )
            .expect("phys.character_set_state should succeed");

        runtime
            .execute(
                "phys.add_collider",
                json!({
                    "entity_id": "Pickup89",
                    "shape": "box",
                    "size": [0.6, 0.6, 0.6]
                }),
            )
            .expect("phys.add_collider should succeed");
        runtime
            .execute(
                "phys.set_collider",
                json!({
                    "entity_id": "Pickup89",
                    "shape": "sphere",
                    "size": [0.8, 0.8, 0.8]
                }),
            )
            .expect("phys.set_collider should succeed");

        let overlap = runtime
            .execute(
                "phys.overlap",
                json!({
                    "shape": "box",
                    "center": [0.5, 0.5, 0.0],
                    "size": [3.0, 3.0, 3.0]
                }),
            )
            .expect("phys.overlap should succeed");
        assert!(
            overlap
                .get("count")
                .and_then(Value::as_u64)
                .unwrap_or_default()
                >= 2
        );

        runtime
            .execute(
                "phys.remove_collider",
                json!({
                    "entity_id": "Pickup89"
                }),
            )
            .expect("phys.remove_collider should succeed");

        runtime
            .execute(
                "game.create_input_action",
                json!({
                    "name": "Shoot89",
                    "bindings": ["MouseLeft", "GamepadRT"]
                }),
            )
            .expect("game.create_input_action should succeed");
        runtime
            .execute(
                "game.bind_action",
                json!({
                    "name": "Shoot89",
                    "target_script_event": "weapon_fire"
                }),
            )
            .expect("game.bind_action should succeed");
        runtime
            .execute(
                "game.set_rebind",
                json!({
                    "action": "Shoot89",
                    "binding": "KeyF"
                }),
            )
            .expect("game.set_rebind should succeed");
        runtime
            .execute(
                "game.add_inventory",
                json!({
                    "entity_id": "Hero89",
                    "capacity": 5,
                    "items": ["medkit"]
                }),
            )
            .expect("game.add_inventory should succeed");
        runtime
            .execute(
                "game.add_trigger",
                json!({
                    "entity_id": "Hero89",
                    "shape": "sphere",
                    "radius": 2.0,
                    "params": {
                        "event": "enter_zone"
                    }
                }),
            )
            .expect("game.add_trigger should succeed");
        runtime
            .execute(
                "game.add_pickup",
                json!({
                    "entity_id": "Pickup89",
                    "item_data": {
                        "kind": "ammo",
                        "amount": 15
                    }
                }),
            )
            .expect("game.add_pickup should succeed");
        runtime
            .execute(
                "game.add_interactable",
                json!({
                    "entity_id": "Hero89",
                    "prompt": "Press F to interact",
                    "actions": ["open_menu", "inspect"]
                }),
            )
            .expect("game.add_interactable should succeed");

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert_eq!(
            state
                .get("physics")
                .and_then(|physics| physics.get("collider_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("physics")
                .and_then(|physics| physics.get("character_controller_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("gameplay")
                .and_then(|gameplay| gameplay.get("input_action_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("gameplay")
                .and_then(|gameplay| gameplay.get("trigger_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("gameplay")
                .and_then(|gameplay| gameplay.get("pickup_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("gameplay")
                .and_then(|gameplay| gameplay.get("inventory_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("gameplay")
                .and_then(|gameplay| gameplay.get("interactable_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("gameplay")
                .and_then(|gameplay| gameplay.get("input_actions"))
                .and_then(|actions| actions.get("Shoot89"))
                .and_then(|action| action.get("target_event"))
                .and_then(Value::as_str),
            Some("weapon_fire")
        );
        assert_eq!(
            state
                .get("gameplay")
                .and_then(|gameplay| gameplay.get("input_actions"))
                .and_then(|actions| actions.get("Shoot89"))
                .and_then(|action| action.get("bindings"))
                .and_then(Value::as_array)
                .and_then(|bindings| bindings.first())
                .and_then(Value::as_str),
            Some("KeyF")
        );
    }

    #[test]
    fn phase10_and_phase11_tools_update_animation_and_modeling_state() {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "Phase1011 Scene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "HeroAnim",
                    "mesh": "capsule",
                    "translation": [0.0, 1.0, 0.0]
                }),
            )
            .expect("entity.create should succeed");

        runtime
            .execute(
                "anim.create_state_machine",
                json!({
                    "name": "Hero Controller",
                    "controller_id": "hero_ctrl"
                }),
            )
            .expect("anim.create_state_machine should succeed");
        runtime
            .execute(
                "anim.add_state",
                json!({
                    "controller_id": "hero_ctrl",
                    "state_name": "idle",
                    "clip_id": "idle_clip"
                }),
            )
            .expect("anim.add_state should succeed");
        runtime
            .execute(
                "anim.add_state",
                json!({
                    "controller_id": "hero_ctrl",
                    "state_name": "run",
                    "clip_id": "run_clip"
                }),
            )
            .expect("anim.add_state should succeed");
        runtime
            .execute(
                "anim.add_transition",
                json!({
                    "controller_id": "hero_ctrl",
                    "from": "idle",
                    "to": "run",
                    "conditions": {"speed_gt": 0.2}
                }),
            )
            .expect("anim.add_transition should succeed");
        runtime
            .execute(
                "anim.set_parameter",
                json!({
                    "controller_id": "hero_ctrl",
                    "key": "speed",
                    "value": 1.0
                }),
            )
            .expect("anim.set_parameter should succeed");
        runtime
            .execute(
                "anim.add_animator",
                json!({
                    "entity_id": "HeroAnim",
                    "controller_id": "hero_ctrl"
                }),
            )
            .expect("anim.add_animator should succeed");
        runtime
            .execute(
                "anim.play",
                json!({
                    "entity_id": "HeroAnim",
                    "clip_id": "idle_clip"
                }),
            )
            .expect("anim.play should succeed");
        runtime
            .execute(
                "anim.blend",
                json!({
                    "entity_id": "HeroAnim",
                    "clip_a": "idle_clip",
                    "clip_b": "run_clip",
                    "weight": 0.4
                }),
            )
            .expect("anim.blend should succeed");
        runtime
            .execute(
                "anim.add_ik",
                json!({
                    "entity_id": "HeroAnim",
                    "chain": "leg_l",
                    "params": {"target": [0.0, 0.0, 0.0]}
                }),
            )
            .expect("anim.add_ik should succeed");
        runtime
            .execute(
                "anim.retarget",
                json!({
                    "source_rig": "humanoid_a",
                    "target_rig": "humanoid_b",
                    "mapping": {"hips": "pelvis"}
                }),
            )
            .expect("anim.retarget should succeed");
        runtime
            .execute(
                "anim.bake_animation",
                json!({
                    "entity_id": "HeroAnim",
                    "params": {"fps": 30}
                }),
            )
            .expect("anim.bake_animation should succeed");

        runtime
            .execute(
                "model.create_primitive",
                json!({
                    "type": "cube",
                    "name": "BlockA",
                    "mesh_id": "mesh_block_a",
                    "translation": [2.0, 0.0, 0.0]
                }),
            )
            .expect("model.create_primitive should succeed");
        runtime
            .execute(
                "model.enter_edit_mode",
                json!({
                    "mesh_id": "mesh_block_a"
                }),
            )
            .expect("model.enter_edit_mode should succeed");
        runtime
            .execute(
                "model.select",
                json!({
                    "mesh_id": "mesh_block_a",
                    "mode": "face",
                    "selector": {"faces": [0,1]}
                }),
            )
            .expect("model.select should succeed");
        runtime
            .execute(
                "model.extrude",
                json!({
                    "mesh_id": "mesh_block_a",
                    "params": {"distance": 0.4}
                }),
            )
            .expect("model.extrude should succeed");
        runtime
            .execute(
                "model.add_modifier",
                json!({
                    "mesh_id": "mesh_block_a",
                    "type": "bevel",
                    "modifier_id": "bev_1",
                    "params": {"width": 0.05}
                }),
            )
            .expect("model.add_modifier should succeed");
        runtime
            .execute(
                "model.apply_modifier",
                json!({
                    "mesh_id": "mesh_block_a",
                    "modifier_id": "bev_1"
                }),
            )
            .expect("model.apply_modifier should succeed");
        runtime
            .execute(
                "model.unwrap_uv",
                json!({
                    "mesh_id": "mesh_block_a",
                    "method": "angle_based",
                    "params": {"margin": 0.02}
                }),
            )
            .expect("model.unwrap_uv should succeed");
        runtime
            .execute(
                "model.pack_uv",
                json!({
                    "mesh_id": "mesh_block_a",
                    "params": {"rotate": true}
                }),
            )
            .expect("model.pack_uv should succeed");
        runtime
            .execute(
                "model.generate_lightmap_uv",
                json!({
                    "mesh_id": "mesh_block_a",
                    "params": {"channel": 1}
                }),
            )
            .expect("model.generate_lightmap_uv should succeed");
        runtime
            .execute(
                "model.decimate",
                json!({
                    "mesh_id": "mesh_block_a",
                    "ratio": 0.5
                }),
            )
            .expect("model.decimate should succeed");
        runtime
            .execute(
                "model.sculpt_brush",
                json!({
                    "mesh_id": "mesh_block_a",
                    "brush_type": "smooth",
                    "params": {"strength": 0.3}
                }),
            )
            .expect("model.sculpt_brush should succeed");
        runtime
            .execute(
                "model.sculpt_mask",
                json!({
                    "mesh_id": "mesh_block_a",
                    "params": {"strength": 0.8}
                }),
            )
            .expect("model.sculpt_mask should succeed");
        runtime
            .execute(
                "model.exit_edit_mode",
                json!({
                    "mesh_id": "mesh_block_a"
                }),
            )
            .expect("model.exit_edit_mode should succeed");

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert_eq!(
            state
                .get("animation")
                .and_then(|animation| animation.get("state_machine_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("animation")
                .and_then(|animation| animation.get("animator_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("animation")
                .and_then(|animation| animation.get("active_clip_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("animation")
                .and_then(|animation| animation.get("blend_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("animation")
                .and_then(|animation| animation.get("ik_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("animation")
                .and_then(|animation| animation.get("retarget_jobs"))
                .and_then(Value::as_array)
                .map(|arr| arr.len()),
            Some(1)
        );
        assert_eq!(
            state
                .get("animation")
                .and_then(|animation| animation.get("bake_jobs"))
                .and_then(Value::as_array)
                .map(|arr| arr.len()),
            Some(1)
        );
        assert_eq!(
            state
                .get("animation")
                .and_then(|animation| animation.get("state_machines"))
                .and_then(|state_machines| state_machines.get("hero_ctrl"))
                .and_then(|controller| controller.get("parameters"))
                .and_then(|params| params.get("speed"))
                .and_then(Value::as_f64),
            Some(1.0)
        );

        assert_eq!(
            state
                .get("modeling")
                .and_then(|modeling| modeling.get("mesh_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("modeling")
                .and_then(|modeling| modeling.get("edit_mode_count"))
                .and_then(Value::as_u64),
            Some(0)
        );
        assert_eq!(
            state
                .get("modeling")
                .and_then(|modeling| modeling.get("selection_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("modeling")
                .and_then(|modeling| modeling.get("modifier_stack_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("modeling")
                .and_then(|modeling| modeling.get("uv_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("modeling")
                .and_then(|modeling| modeling.get("sculpt_mask_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert!(
            state
                .get("modeling")
                .and_then(|modeling| modeling.get("operation_log_len"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
                >= 10
        );
    }

    #[test]
    fn phase12_and_phase13_tools_update_vfx_and_water_state() {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "Phase1213 Scene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Boat13",
                    "mesh": "cube",
                    "translation": [0.0, 0.5, 0.0]
                }),
            )
            .expect("entity.create should succeed");

        runtime
            .execute(
                "water.create_ocean",
                json!({
                    "ocean_id": "ocean_main",
                    "size": 640.0,
                    "waves": {"amplitude": 1.3, "frequency": 0.17, "phase": 0.2},
                    "params": {"base_height": 0.0}
                }),
            )
            .expect("water.create_ocean should succeed");
        runtime
            .execute(
                "water.set_waves",
                json!({
                    "ocean_id": "ocean_main",
                    "params": {"amplitude": 1.5, "frequency": 0.2, "phase": 0.3}
                }),
            )
            .expect("water.set_waves should succeed");
        runtime
            .execute(
                "water.enable_foam",
                json!({
                    "ocean_id": "ocean_main",
                    "params": {"amount": 0.6}
                }),
            )
            .expect("water.enable_foam should succeed");
        runtime
            .execute(
                "water.enable_refraction",
                json!({
                    "ocean_id": "ocean_main",
                    "params": {"strength": 0.45}
                }),
            )
            .expect("water.enable_refraction should succeed");
        runtime
            .execute(
                "water.enable_caustics",
                json!({
                    "ocean_id": "ocean_main",
                    "params": {"strength": 0.5}
                }),
            )
            .expect("water.enable_caustics should succeed");
        runtime
            .execute(
                "water.add_buoyancy",
                json!({
                    "entity_id": "Boat13",
                    "params": {"float_height": 0.7}
                }),
            )
            .expect("water.add_buoyancy should succeed");
        runtime
            .execute(
                "water.add_drag",
                json!({
                    "entity_id": "Boat13",
                    "params": {"linear": 0.2, "angular": 0.1}
                }),
            )
            .expect("water.add_drag should succeed");
        runtime
            .execute(
                "water.create_river",
                json!({
                    "river_id": "river_a",
                    "path": [[-10.0,0.0,0.0],[0.0,0.0,4.0],[10.0,0.0,0.0]]
                }),
            )
            .expect("water.create_river should succeed");
        runtime
            .execute(
                "water.create_waterfall",
                json!({
                    "waterfall_id": "fall_a",
                    "params": {"height": 15.0}
                }),
            )
            .expect("water.create_waterfall should succeed");

        let sample = runtime
            .execute(
                "water.sample_height",
                json!({
                    "ocean_id": "ocean_main",
                    "position": [4.0, 0.0, 6.0]
                }),
            )
            .expect("water.sample_height should succeed");
        assert!(sample.get("height").and_then(Value::as_f64).is_some());

        runtime
            .execute(
                "vfx.create_particle_system",
                json!({
                    "name": "smoke_fx",
                    "particle_id": "fx_smoke",
                    "params": {"max_particles": 4096}
                }),
            )
            .expect("vfx.create_particle_system should succeed");
        runtime
            .execute(
                "vfx.set_emitter",
                json!({
                    "particle_id": "fx_smoke",
                    "params": {"rate": 120.0, "lifetime": 2.2}
                }),
            )
            .expect("vfx.set_emitter should succeed");
        runtime
            .execute(
                "vfx.set_forces",
                json!({
                    "particle_id": "fx_smoke",
                    "params": {"gravity": [0.0, 0.6, 0.0]}
                }),
            )
            .expect("vfx.set_forces should succeed");
        runtime
            .execute(
                "vfx.set_collision",
                json!({
                    "particle_id": "fx_smoke",
                    "params": {"enabled": true}
                }),
            )
            .expect("vfx.set_collision should succeed");
        runtime
            .execute(
                "vfx.set_renderer",
                json!({
                    "particle_id": "fx_smoke",
                    "params": {"mode": "billboard"}
                }),
            )
            .expect("vfx.set_renderer should succeed");
        runtime
            .execute(
                "vfx.attach_to_entity",
                json!({
                    "particle_id": "fx_smoke",
                    "entity_id": "Boat13",
                    "socket": "smokestack"
                }),
            )
            .expect("vfx.attach_to_entity should succeed");
        runtime
            .execute(
                "vfx.create_graph",
                json!({
                    "name": "fx_graph",
                    "graph_id": "fx_graph"
                }),
            )
            .expect("vfx.create_graph should succeed");
        runtime
            .execute(
                "vfx.add_node",
                json!({
                    "graph_id": "fx_graph",
                    "node_type": "SpawnBurst",
                    "node_id": "spawn"
                }),
            )
            .expect("vfx.add_node should succeed");
        runtime
            .execute(
                "vfx.add_node",
                json!({
                    "graph_id": "fx_graph",
                    "node_type": "ApplyForce",
                    "node_id": "force"
                }),
            )
            .expect("vfx.add_node should succeed");
        runtime
            .execute(
                "vfx.connect",
                json!({
                    "graph_id": "fx_graph",
                    "out_node": "spawn",
                    "in_node": "force"
                }),
            )
            .expect("vfx.connect should succeed");
        runtime
            .execute(
                "vfx.compile_graph",
                json!({
                    "graph_id": "fx_graph"
                }),
            )
            .expect("vfx.compile_graph should succeed");

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert_eq!(
            state
                .get("water")
                .and_then(|water| water.get("ocean_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("water")
                .and_then(|water| water.get("river_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("water")
                .and_then(|water| water.get("waterfall_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("water")
                .and_then(|water| water.get("buoyancy_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("water")
                .and_then(|water| water.get("drag_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("vfx")
                .and_then(|vfx| vfx.get("particle_system_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("vfx")
                .and_then(|vfx| vfx.get("graph_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("vfx")
                .and_then(|vfx| vfx.get("graphs"))
                .and_then(|graphs| graphs.get("fx_graph"))
                .and_then(|graph| graph.get("compiled"))
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn phase14_and_phase15_tools_update_mount_and_ai_state() {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "Phase1415 Scene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Rider14",
                    "mesh": "capsule",
                    "translation": [0.0, 1.0, 0.0]
                }),
            )
            .expect("entity.create should succeed");

        runtime
            .execute(
                "mount.create_horse_template",
                json!({
                    "template_id": "horse_tpl_a",
                    "params": {"mesh":"horse","stats":{"speed":8.2}}
                }),
            )
            .expect("mount.create_horse_template should succeed");
        runtime
            .execute(
                "mount.spawn_horse",
                json!({
                    "template_id": "horse_tpl_a",
                    "horse_id": "horse_a",
                    "entity_id": "Horse14",
                    "translation": [2.0, 0.0, 0.0]
                }),
            )
            .expect("mount.spawn_horse should succeed");
        runtime
            .execute(
                "mount.mount_rider",
                json!({
                    "horse_id": "horse_a",
                    "rider_id": "Rider14"
                }),
            )
            .expect("mount.mount_rider should succeed");
        runtime
            .execute(
                "mount.set_gait",
                json!({
                    "horse_id": "horse_a",
                    "gait": "gallop"
                }),
            )
            .expect("mount.set_gait should succeed");
        runtime
            .execute(
                "mount.set_path_follow",
                json!({
                    "horse_id": "horse_a",
                    "path_id": "track_main"
                }),
            )
            .expect("mount.set_path_follow should succeed");

        runtime
            .execute(
                "ai.create_navmesh",
                json!({
                    "navmesh_id": "main_navmesh",
                    "params": {"cell_size": 0.2}
                }),
            )
            .expect("ai.create_navmesh should succeed");
        runtime
            .execute(
                "ai.bake_navmesh",
                json!({
                    "navmesh_id": "main_navmesh"
                }),
            )
            .expect("ai.bake_navmesh should succeed");
        runtime
            .execute(
                "ai.add_agent",
                json!({
                    "entity_id": "Rider14",
                    "agent_id": "agent_rider_14",
                    "params": {"speed": 3.4}
                }),
            )
            .expect("ai.add_agent should succeed");
        runtime
            .execute(
                "ai.set_destination",
                json!({
                    "agent_id": "agent_rider_14",
                    "position": [8.0, 0.0, 3.0]
                }),
            )
            .expect("ai.set_destination should succeed");
        runtime
            .execute(
                "ai.create_behavior_tree",
                json!({
                    "name": "GuardTree",
                    "tree_id": "bt_guard_14"
                }),
            )
            .expect("ai.create_behavior_tree should succeed");
        runtime
            .execute(
                "ai.bt_add_node",
                json!({
                    "tree_id": "bt_guard_14",
                    "node_type": "Selector",
                    "node_id": "root"
                }),
            )
            .expect("ai.bt_add_node should succeed");
        runtime
            .execute(
                "ai.bt_add_node",
                json!({
                    "tree_id": "bt_guard_14",
                    "node_type": "ChaseTarget",
                    "node_id": "chase"
                }),
            )
            .expect("ai.bt_add_node should succeed");
        runtime
            .execute(
                "ai.bt_connect",
                json!({
                    "tree_id": "bt_guard_14",
                    "parent": "root",
                    "child": "chase"
                }),
            )
            .expect("ai.bt_connect should succeed");
        runtime
            .execute(
                "ai.assign_behavior",
                json!({
                    "entity_id": "Rider14",
                    "tree_id": "bt_guard_14"
                }),
            )
            .expect("ai.assign_behavior should succeed");
        runtime
            .execute(
                "ai.set_blackboard",
                json!({
                    "entity_id": "Rider14",
                    "key": "target",
                    "value": "Enemy_1"
                }),
            )
            .expect("ai.set_blackboard should succeed");

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert_eq!(
            state
                .get("mount")
                .and_then(|mount| mount.get("template_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("mount")
                .and_then(|mount| mount.get("horse_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("mount")
                .and_then(|mount| mount.get("horses"))
                .and_then(|horses| horses.get("horse_a"))
                .and_then(|horse| horse.get("gait"))
                .and_then(Value::as_str),
            Some("gallop")
        );
        assert_eq!(
            state
                .get("mount")
                .and_then(|mount| mount.get("rider_to_horse"))
                .and_then(|map| map.get("Rider14"))
                .and_then(Value::as_str),
            Some("horse_a")
        );
        assert_eq!(
            state
                .get("ai")
                .and_then(|ai| ai.get("navmesh_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("ai")
                .and_then(|ai| ai.get("agent_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("ai")
                .and_then(|ai| ai.get("behavior_tree_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("ai")
                .and_then(|ai| ai.get("agents"))
                .and_then(|agents| agents.get("agent_rider_14"))
                .and_then(|agent| agent.get("behavior_tree_id"))
                .and_then(Value::as_str),
            Some("bt_guard_14")
        );
    }

    #[test]
    fn phase16_and_phase17_tools_update_ui_and_audio_state() {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "Phase1617 Scene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "HeroUI17",
                    "mesh": "capsule",
                    "translation": [0.0, 1.0, 0.0]
                }),
            )
            .expect("entity.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Speaker17",
                    "mesh": "cube",
                    "translation": [1.0, 1.0, 0.0]
                }),
            )
            .expect("entity.create should succeed");
        runtime
            .execute(
                "game.add_health_component",
                json!({
                    "entity_id": "HeroUI17",
                    "max_health": 120.0,
                    "current_health": 120.0
                }),
            )
            .expect("game.add_health_component should succeed");
        runtime
            .execute(
                "ui.create_canvas",
                json!({
                    "name": "MainHUD",
                    "canvas_id": "hud_main"
                }),
            )
            .expect("ui.create_canvas should succeed");
        runtime
            .execute(
                "ui.add_text",
                json!({
                    "canvas_id": "hud_main",
                    "ui_id": "txt_health",
                    "text": "HP: 120"
                }),
            )
            .expect("ui.add_text should succeed");
        runtime
            .execute(
                "ui.bind_to_data",
                json!({
                    "ui_id": "txt_health",
                    "entity_id": "HeroUI17",
                    "component_field": "Health.current_health"
                }),
            )
            .expect("ui.bind_to_data should succeed");
        runtime
            .execute(
                "ui.create_hud_template",
                json!({
                    "type": "shooter"
                }),
            )
            .expect("ui.create_hud_template should succeed");

        runtime
            .execute(
                "audio.import_clip",
                json!({
                    "path": "Cargo.toml",
                    "clip_id": "clip_test"
                }),
            )
            .expect("audio.import_clip should succeed");
        runtime
            .execute(
                "audio.create_mixer",
                json!({
                    "bus_id": "master",
                    "params": {"volume": 1.0}
                }),
            )
            .expect("audio.create_mixer should succeed");
        runtime
            .execute(
                "audio.create_source",
                json!({
                    "source_id": "source_a",
                    "entity_id": "Speaker17",
                    "params": {"loop": false}
                }),
            )
            .expect("audio.create_source should succeed");
        runtime
            .execute(
                "audio.set_spatial",
                json!({
                    "source_id": "source_a",
                    "params": {"min_distance": 1.0, "max_distance": 25.0}
                }),
            )
            .expect("audio.set_spatial should succeed");
        runtime
            .execute(
                "audio.route",
                json!({
                    "source_id": "source_a",
                    "mixer_bus": "master"
                }),
            )
            .expect("audio.route should succeed");
        runtime
            .execute(
                "audio.play",
                json!({
                    "source_id": "source_a",
                    "clip_id": "clip_test"
                }),
            )
            .expect("audio.play should succeed");

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert_eq!(
            state
                .get("ui")
                .and_then(|ui| ui.get("canvas_count"))
                .and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(
            state
                .get("ui")
                .and_then(|ui| ui.get("binding_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("ui")
                .and_then(|ui| ui.get("active_hud_template"))
                .and_then(Value::as_str),
            Some("shooter")
        );
        assert_eq!(
            state
                .get("audio")
                .and_then(|audio| audio.get("clip_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("audio")
                .and_then(|audio| audio.get("source_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("audio")
                .and_then(|audio| audio.get("mixer_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("audio")
                .and_then(|audio| audio.get("play_events"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("audio")
                .and_then(|audio| audio.get("sources"))
                .and_then(|sources| sources.get("source_a"))
                .and_then(|source| source.get("mixer_bus"))
                .and_then(Value::as_str),
            Some("master")
        );
    }

    #[test]
    fn phase18_to_phase20_tools_update_network_build_and_debug_state() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let project_root = std::env::temp_dir().join(format!("ai_phase1820_{}", unique_suffix));
        fs::create_dir_all(project_root.join("samples")).expect("should create temp project root");

        let mut runtime = ToolRuntime::new(&project_root);
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "Phase1820 Scene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "NetHero1820",
                    "mesh": "capsule",
                    "translation": [0.0, 1.0, 0.0]
                }),
            )
            .expect("entity.create should succeed");

        runtime
            .execute(
                "net.create_server",
                json!({
                    "server_id": "server_main",
                    "params": {"port": 7777}
                }),
            )
            .expect("net.create_server should succeed");
        runtime
            .execute(
                "net.connect_client",
                json!({
                    "client_id": "client_a",
                    "endpoint": "127.0.0.1:7777",
                    "params": {"transport": "udp"}
                }),
            )
            .expect("net.connect_client should succeed");
        runtime
            .execute(
                "net.enable_replication",
                json!({
                    "entity_id": "NetHero1820",
                    "components": ["Transform", "Health"]
                }),
            )
            .expect("net.enable_replication should succeed");
        runtime
            .execute(
                "net.set_prediction",
                json!({
                    "mode": "hybrid"
                }),
            )
            .expect("net.set_prediction should succeed");
        runtime
            .execute(
                "net.set_rollback",
                json!({
                    "params": {"max_frames": 8, "input_delay_ms": 80}
                }),
            )
            .expect("net.set_rollback should succeed");

        runtime
            .execute(
                "build.set_target",
                json!({
                    "platform": "linux"
                }),
            )
            .expect("build.set_target should succeed");
        runtime
            .execute(
                "build.set_bundle_id",
                json!({
                    "id": "com.rey30.phase1820"
                }),
            )
            .expect("build.set_bundle_id should succeed");
        runtime
            .execute(
                "build.set_version",
                json!({
                    "version": "1.2.3"
                }),
            )
            .expect("build.set_version should succeed");
        runtime
            .execute(
                "build.enable_feature",
                json!({
                    "flag": "shipping_mode"
                }),
            )
            .expect("build.enable_feature should succeed");
        let export_result = runtime
            .execute(
                "build.export_project",
                json!({
                    "path": "dist/export"
                }),
            )
            .expect("build.export_project should succeed");
        let installer_result = runtime
            .execute(
                "build.generate_installer",
                json!({
                    "path": "dist/installer"
                }),
            )
            .expect("build.generate_installer should succeed");

        runtime
            .execute(
                "debug.show_colliders",
                json!({
                    "on": true
                }),
            )
            .expect("debug.show_colliders should succeed");
        runtime
            .execute(
                "debug.show_navmesh",
                json!({
                    "on": true
                }),
            )
            .expect("debug.show_navmesh should succeed");
        runtime
            .execute(
                "debug.toggle_wireframe",
                json!({
                    "on": true
                }),
            )
            .expect("debug.toggle_wireframe should succeed");
        runtime
            .execute("debug.capture_frame", json!({}))
            .expect("debug.capture_frame should succeed");

        let profiler = runtime
            .execute(
                "debug.get_profiler_snapshot",
                json!({
                    "last_n": 1
                }),
            )
            .expect("debug.get_profiler_snapshot should succeed");
        assert_eq!(profiler.get("count").and_then(Value::as_u64), Some(1));

        let hotspots = runtime
            .execute(
                "debug.find_performance_hotspots",
                json!({
                    "last_n": 8
                }),
            )
            .expect("debug.find_performance_hotspots should succeed");
        assert_eq!(hotspots.get("status").and_then(Value::as_str), Some("ok"));
        assert!(hotspots.get("hotspots").and_then(Value::as_array).is_some());

        let export_manifest_path = export_result
            .get("result")
            .and_then(|result| result.get("payload"))
            .and_then(|payload| payload.get("manifest_path"))
            .and_then(Value::as_str)
            .expect("build.export_project should return manifest path");
        assert!(Path::new(export_manifest_path).exists());

        let installer_manifest_path = installer_result
            .get("result")
            .and_then(|result| result.get("payload"))
            .and_then(|payload| payload.get("installer_path"))
            .and_then(Value::as_str)
            .expect("build.generate_installer should return installer path");
        assert!(Path::new(installer_manifest_path).exists());

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert_eq!(
            state
                .get("networking")
                .and_then(|net| net.get("has_server"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            state
                .get("networking")
                .and_then(|net| net.get("client_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("networking")
                .and_then(|net| net.get("prediction_mode"))
                .and_then(Value::as_str),
            Some("hybrid")
        );
        assert_eq!(
            state
                .get("build")
                .and_then(|build| build.get("target"))
                .and_then(Value::as_str),
            Some("linux")
        );
        assert_eq!(
            state
                .get("build")
                .and_then(|build| build.get("bundle_id"))
                .and_then(Value::as_str),
            Some("com.rey30.phase1820")
        );
        assert_eq!(
            state
                .get("build")
                .and_then(|build| build.get("version"))
                .and_then(Value::as_str),
            Some("1.2.3")
        );
        assert_eq!(
            state
                .get("build")
                .and_then(|build| build.get("enabled_feature_count"))
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            state
                .get("debug")
                .and_then(|debug| debug.get("show_colliders"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            state
                .get("debug")
                .and_then(|debug| debug.get("show_navmesh"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            state
                .get("debug")
                .and_then(|debug| debug.get("wireframe"))
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            state
                .get("debug")
                .and_then(|debug| debug.get("captured_frames"))
                .and_then(Value::as_u64),
            Some(1)
        );

        fs::remove_dir_all(project_root).expect("temp project root cleanup should succeed");
    }

    #[test]
    fn phase22_tools_manage_cycle_context_memory_constraints_and_diagnostics() {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "Phase22 Scene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Hero22",
                    "mesh": "capsule",
                    "translation": [0.0, 1.0, 0.0]
                }),
            )
            .expect("entity.create should succeed");
        runtime
            .execute(
                "tool.set_selection",
                json!({
                    "entity_ids": ["Hero22"]
                }),
            )
            .expect("tool.set_selection should succeed");
        runtime
            .execute(
                "tool.set_objective",
                json!({
                    "objective": "build a stable 60 fps arena"
                }),
            )
            .expect("tool.set_objective should succeed");
        runtime
            .execute(
                "tool.set_project_memory",
                json!({
                    "style": "stylized",
                    "target_platform": "windows",
                    "target_fps": 60.0,
                    "notes": ["focus on readability"],
                    "tags": ["s22"]
                }),
            )
            .expect("tool.set_project_memory should succeed");
        runtime
            .execute(
                "tool.set_constraints",
                json!({
                    "target_fps": 60.0,
                    "resolution": "1920x1080",
                    "allow_external_assets": false,
                    "max_gpu_memory_mb": 4096,
                    "notes": ["sin assets externos"]
                }),
            )
            .expect("tool.set_constraints should succeed");
        runtime
            .execute(
                "tool.log",
                json!({
                    "level": "warn",
                    "message": "fps fluctuating in heavy scene"
                }),
            )
            .expect("tool.log warn should succeed");
        runtime
            .execute(
                "tool.log",
                json!({
                    "level": "error",
                    "message": "missing impostor LOD"
                }),
            )
            .expect("tool.log error should succeed");

        let cycle = runtime
            .execute(
                "tool.get_cycle_context",
                json!({
                    "max_entities": 8,
                    "recent_commands": 8,
                    "diagnostics_last_n": 8
                }),
            )
            .expect("tool.get_cycle_context should succeed");
        assert_eq!(
            cycle.get("objective").and_then(Value::as_str),
            Some("build a stable 60 fps arena")
        );
        assert_eq!(
            cycle
                .get("project_memory")
                .and_then(|memory| memory.get("style"))
                .and_then(Value::as_str),
            Some("stylized")
        );
        assert_eq!(
            cycle
                .get("constraints")
                .and_then(|constraints| constraints.get("allow_external_assets"))
                .and_then(Value::as_bool),
            Some(false)
        );
        assert!(
            cycle
                .get("diagnostics")
                .and_then(|diag| diag.get("warning_count"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
                >= 1
        );
        assert!(
            cycle
                .get("diagnostics")
                .and_then(|diag| diag.get("error_count"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
                >= 1
        );

        let diagnostics_before = runtime
            .execute("tool.get_diagnostics", json!({"last_n": 16}))
            .expect("tool.get_diagnostics should succeed")
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        assert!(diagnostics_before >= 2);
        let cleared = runtime
            .execute("tool.clear_diagnostics", json!({"level":"warn"}))
            .expect("tool.clear_diagnostics should succeed")
            .get("cleared")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        assert!(cleared >= 1);
    }

    #[test]
    fn phase23_asset_assign_material_tools_bind_material_component() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let project_root =
            std::env::temp_dir().join(format!("ai_phase23_material_{}", unique_suffix));
        fs::create_dir_all(project_root.join("samples")).expect("should create temp project root");

        let mut runtime = ToolRuntime::new(&project_root);
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "Phase23 Scene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Crate23",
                    "mesh": "cube",
                    "translation": [0.0, 0.0, 0.0]
                }),
            )
            .expect("entity.create should succeed");
        runtime
            .execute(
                "asset.create_material",
                json!({
                    "name": "mat_crate23",
                    "preset": "pbr_default",
                    "params": {
                        "base_color": [0.8, 0.4, 0.2]
                    }
                }),
            )
            .expect("asset.create_material should succeed");
        runtime
            .execute(
                "asset.assign_material",
                json!({
                    "entity_id": "Crate23",
                    "material_id": "mat_crate23",
                    "slot": "base"
                }),
            )
            .expect("asset.assign_material should succeed");
        runtime
            .execute(
                "render.assign_material",
                json!({
                    "entity_id": "Crate23",
                    "material_id": "mat_crate23",
                    "slot": "overlay"
                }),
            )
            .expect("render.assign_material should succeed");

        let component = runtime
            .command_bus()
            .context()
            .components
            .get("Crate23")
            .and_then(|bucket| bucket.get("MaterialOverride"))
            .cloned()
            .expect("MaterialOverride component should exist");
        assert_eq!(
            component.get("material_id").and_then(Value::as_str),
            Some("mat_crate23")
        );
        assert_eq!(
            component.get("slot").and_then(Value::as_str),
            Some("overlay")
        );

        fs::remove_dir_all(project_root).expect("temp project root cleanup should succeed");
    }

    #[test]
    fn phase24_entity_lifecycle_tools_cover_clone_rename_parent_delete_and_search() {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "Phase24 Scene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Root24",
                    "mesh": "cube",
                    "translation": [0.0, 0.0, 0.0]
                }),
            )
            .expect("entity.create root should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Child24",
                    "mesh": "capsule",
                    "translation": [1.0, 0.0, 0.0]
                }),
            )
            .expect("entity.create child should succeed");
        runtime
            .execute(
                "entity.set_component",
                json!({
                    "entity_id": "Child24",
                    "component_type": "Tags",
                    "data": ["enemy", "phase24"]
                }),
            )
            .expect("entity.set_component should succeed");
        runtime
            .execute(
                "entity.parent",
                json!({
                    "child_id": "Child24",
                    "parent_id": "Root24"
                }),
            )
            .expect("entity.parent should succeed");
        runtime
            .execute(
                "entity.clone",
                json!({
                    "entity_id": "Child24",
                    "name": "Child24Clone",
                    "translation_offset": [0.5, 0.0, 0.0],
                    "copy_components": true,
                    "copy_parent": true
                }),
            )
            .expect("entity.clone should succeed");
        runtime
            .execute(
                "entity.rename",
                json!({
                    "entity_id": "Child24Clone",
                    "name": "Child24Renamed"
                }),
            )
            .expect("entity.rename should succeed");
        runtime
            .execute(
                "entity.unparent",
                json!({
                    "child_id": "Child24Renamed"
                }),
            )
            .expect("entity.unparent should succeed");

        let by_name = runtime
            .execute(
                "entity.find_by_name",
                json!({
                    "query": "Child24"
                }),
            )
            .expect("entity.find_by_name should succeed");
        assert!(by_name.get("count").and_then(Value::as_u64).unwrap_or(0) >= 2);

        let by_tag = runtime
            .execute(
                "entity.find_by_tag",
                json!({
                    "tag": "enemy"
                }),
            )
            .expect("entity.find_by_tag should succeed");
        assert!(
            by_tag
                .get("entity_ids")
                .and_then(Value::as_array)
                .map(|items| items.iter().any(|value| value.as_str() == Some("Child24")))
                .unwrap_or(false)
        );

        runtime
            .execute(
                "entity.delete",
                json!({
                    "entity_id": "Child24Renamed"
                }),
            )
            .expect("entity.delete should succeed");
        assert!(
            !runtime
                .command_bus()
                .context()
                .scene
                .entities
                .iter()
                .any(|entity| entity.name == "Child24Renamed")
        );
    }

    #[test]
    fn phase25_entity_transform_and_component_tools_cover_translate_rotate_scale_and_component_io()
    {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "Phase25 Scene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Actor25",
                    "mesh": "cube",
                    "translation": [0.0, 0.0, 0.0]
                }),
            )
            .expect("entity.create should succeed");
        runtime
            .execute(
                "entity.translate",
                json!({
                    "entity_id": "Actor25",
                    "delta": [2.0, 1.0, -3.0]
                }),
            )
            .expect("entity.translate should succeed");
        runtime
            .execute(
                "entity.rotate",
                json!({
                    "entity_id": "Actor25",
                    "delta": [0.0, 90.0, 0.0]
                }),
            )
            .expect("entity.rotate should succeed");
        runtime
            .execute(
                "entity.scale",
                json!({
                    "entity_id": "Actor25",
                    "factor": [2.0, 1.5, 0.5]
                }),
            )
            .expect("entity.scale should succeed");
        runtime
            .execute(
                "entity.set_component",
                json!({
                    "entity_id": "Actor25",
                    "component_type": "Health",
                    "data": {"value": 120}
                }),
            )
            .expect("entity.set_component should succeed");

        let health = runtime
            .execute(
                "entity.get_component",
                json!({
                    "entity_id": "Actor25",
                    "component_type": "Health"
                }),
            )
            .expect("entity.get_component should succeed");
        assert_eq!(health.get("exists").and_then(Value::as_bool), Some(true));
        assert_eq!(
            health
                .get("data")
                .and_then(|data| data.get("value"))
                .and_then(Value::as_i64),
            Some(120)
        );

        let transform = runtime
            .execute(
                "entity.get_transform",
                json!({
                    "entity_id": "Actor25"
                }),
            )
            .expect("entity.get_transform should succeed");
        assert_eq!(
            transform
                .get("translation")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(Value::as_f64),
            Some(2.0)
        );
        assert_eq!(
            transform
                .get("rotation")
                .and_then(Value::as_array)
                .and_then(|items| items.get(1))
                .and_then(Value::as_f64),
            Some(90.0)
        );
        assert_eq!(
            transform
                .get("scale")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(Value::as_f64),
            Some(2.0)
        );

        runtime
            .execute(
                "entity.remove_component",
                json!({
                    "entity_id": "Actor25",
                    "component_type": "Health"
                }),
            )
            .expect("entity.remove_component should succeed");
        let health_after = runtime
            .execute(
                "entity.get_component",
                json!({
                    "entity_id": "Actor25",
                    "component_type": "Health"
                }),
            )
            .expect("entity.get_component should succeed");
        assert_eq!(
            health_after.get("exists").and_then(Value::as_bool),
            Some(false)
        );
    }

    #[test]
    fn phase26_asset_pipeline_import_create_and_prefab_tools_work() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let project_root =
            std::env::temp_dir().join(format!("ai_phase26_assets_{}", unique_suffix));
        fs::create_dir_all(project_root.join("samples")).expect("should create temp project root");

        let mut runtime = ToolRuntime::new(&project_root);
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "Phase26 Scene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Crate26",
                    "mesh": "cube",
                    "translation": [0.0, 0.0, 0.0]
                }),
            )
            .expect("entity.create should succeed");

        let source_file = project_root.join("samples").join("crate_phase26.glb");
        fs::write(&source_file, b"phase26_glb").expect("should write source sample file");
        runtime
            .execute(
                "asset.import_file",
                json!({
                    "path": source_file.display().to_string(),
                    "target_subdir": "assets/imported"
                }),
            )
            .expect("asset.import_file should succeed");
        runtime
            .execute(
                "asset.create_texture",
                json!({
                    "name": "crate_albedo_26",
                    "texture_id": "crate_albedo_26",
                    "width": 1024,
                    "height": 1024,
                    "format": "rgba8",
                    "params": {
                        "mipmaps": true
                    }
                }),
            )
            .expect("asset.create_texture should succeed");
        runtime
            .execute(
                "asset.create_shader",
                json!({
                    "name": "crate_shader_26",
                    "shader_id": "crate_shader_26",
                    "template": "pbr_lit",
                    "params": {
                        "use_normal_map": true
                    }
                }),
            )
            .expect("asset.create_shader should succeed");
        runtime
            .execute(
                "asset.create_prefab",
                json!({
                    "name": "crate_prefab_26",
                    "prefab_id": "crate_prefab_26",
                    "entity_id": "Crate26",
                    "metadata": {
                        "category": "props"
                    }
                }),
            )
            .expect("asset.create_prefab should succeed");
        runtime
            .execute(
                "asset.save_prefab",
                json!({
                    "prefab_id": "crate_prefab_26"
                }),
            )
            .expect("asset.save_prefab should succeed");

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert_eq!(
            state
                .get("assets")
                .and_then(|assets| assets.get("textures"))
                .and_then(|textures| textures.get("crate_albedo_26"))
                .and_then(|texture| texture.get("width"))
                .and_then(Value::as_u64),
            Some(1024)
        );
        assert_eq!(
            state
                .get("assets")
                .and_then(|assets| assets.get("shaders"))
                .and_then(|shaders| shaders.get("crate_shader_26"))
                .and_then(|shader| shader.get("template"))
                .and_then(Value::as_str),
            Some("pbr_lit")
        );
        assert_eq!(
            state
                .get("assets")
                .and_then(|assets| assets.get("prefabs"))
                .and_then(|prefabs| prefabs.get("crate_prefab_26"))
                .and_then(|prefab| prefab.get("source_entity_id"))
                .and_then(Value::as_str),
            Some("Crate26")
        );

        fs::remove_dir_all(project_root).expect("temp project root cleanup should succeed");
    }

    #[test]
    fn phase27_asset_pipeline_process_and_bake_tools_work() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let project_root =
            std::env::temp_dir().join(format!("ai_phase27_assets_{}", unique_suffix));
        fs::create_dir_all(project_root.join("samples")).expect("should create temp project root");

        let mut runtime = ToolRuntime::new(&project_root);
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "Phase27 Scene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Mesh27",
                    "mesh": "cube",
                    "translation": [0.0, 0.0, 0.0]
                }),
            )
            .expect("entity.create should succeed");

        let source_file = project_root.join("samples").join("mesh27.glb");
        fs::write(&source_file, b"phase27_glb").expect("should write source sample file");
        let import_result = runtime
            .execute(
                "asset.import_file",
                json!({
                    "path": source_file.display().to_string(),
                    "target_subdir": "assets/imported"
                }),
            )
            .expect("asset.import_file should succeed");
        let imported_asset_id = import_result
            .get("result")
            .and_then(|result| result.get("payload"))
            .and_then(|payload| payload.get("asset_id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .expect("asset.import_file should return asset_id");

        runtime
            .execute(
                "asset.create_texture",
                json!({
                    "name": "tex27",
                    "texture_id": "tex27",
                    "width": 256,
                    "height": 256,
                    "format": "rgba8"
                }),
            )
            .expect("asset.create_texture should succeed");
        runtime
            .execute(
                "asset.rebuild_import",
                json!({
                    "asset_id": imported_asset_id
                }),
            )
            .expect("asset.rebuild_import should succeed");
        runtime
            .execute(
                "asset.generate_lods",
                json!({
                    "mesh_id": "cube",
                    "levels": 3,
                    "reduction": 0.5
                }),
            )
            .expect("asset.generate_lods should succeed");
        runtime
            .execute(
                "asset.mesh_optimize",
                json!({
                    "mesh_id": "cube",
                    "profile": "aggressive"
                }),
            )
            .expect("asset.mesh_optimize should succeed");
        runtime
            .execute(
                "asset.compress_textures",
                json!({
                    "asset_id": "tex27",
                    "format": "bc7",
                    "quality": "high"
                }),
            )
            .expect("asset.compress_textures should succeed");
        runtime
            .execute(
                "asset.bake_lightmaps",
                json!({
                    "params": {
                        "resolution": 1024
                    }
                }),
            )
            .expect("asset.bake_lightmaps should succeed");
        runtime
            .execute(
                "asset.bake_reflection_probes",
                json!({
                    "params": {
                        "probe_count": 4
                    }
                }),
            )
            .expect("asset.bake_reflection_probes should succeed");

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert!(
            state
                .get("assets")
                .and_then(|assets| assets.get("pipeline"))
                .and_then(|pipeline| pipeline.get("rebuilds"))
                .and_then(Value::as_array)
                .map(|rebuilds| !rebuilds.is_empty())
                .unwrap_or(false)
        );
        assert!(
            state
                .get("assets")
                .and_then(|assets| assets.get("pipeline"))
                .and_then(|pipeline| pipeline.get("lods"))
                .and_then(|lods| lods.get("cube"))
                .is_some()
        );
        assert!(
            state
                .get("assets")
                .and_then(|assets| assets.get("pipeline"))
                .and_then(|pipeline| pipeline.get("texture_compressions"))
                .and_then(|compressions| compressions.get("tex27"))
                .is_some()
        );
        assert!(
            state
                .get("assets")
                .and_then(|assets| assets.get("pipeline"))
                .and_then(|pipeline| pipeline.get("lightmap_bakes"))
                .and_then(Value::as_array)
                .map(|bakes| !bakes.is_empty())
                .unwrap_or(false)
        );
        assert!(
            state
                .get("assets")
                .and_then(|assets| assets.get("pipeline"))
                .and_then(|pipeline| pipeline.get("reflection_probe_bakes"))
                .and_then(Value::as_array)
                .map(|bakes| !bakes.is_empty())
                .unwrap_or(false)
        );

        fs::remove_dir_all(project_root).expect("temp project root cleanup should succeed");
    }

    #[test]
    fn gen_plan_from_prompt_shooter_includes_phase6_to_phase9_steps() {
        let mut runtime = ToolRuntime::new(".");
        let plan = runtime
            .execute(
                "gen.plan_from_prompt",
                json!({
                    "prompt": "build a fast shooter prototype"
                }),
            )
            .expect("gen.plan_from_prompt should succeed");

        let tools = plan
            .get("steps")
            .and_then(Value::as_array)
            .expect("plan must include steps")
            .iter()
            .filter_map(|step| step.get("tool"))
            .filter_map(Value::as_str)
            .collect::<Vec<&str>>();

        assert!(tools.contains(&"phys.add_collider"));
        assert!(tools.contains(&"phys.add_rigidbody"));
        assert!(tools.contains(&"phys.add_character_controller"));
        assert!(tools.contains(&"game.create_input_action"));
        assert!(tools.contains(&"game.bind_action"));
        assert!(tools.contains(&"game.create_weapon"));
        assert!(tools.contains(&"game.attach_weapon"));
        assert!(tools.contains(&"game.fire_weapon"));
    }

    #[test]
    fn gen_plan_from_prompt_horse_includes_animation_steps() {
        let mut runtime = ToolRuntime::new(".");
        let plan = runtime
            .execute(
                "gen.plan_from_prompt",
                json!({
                    "prompt": "create a caballo cinematic"
                }),
            )
            .expect("gen.plan_from_prompt should succeed");
        let tools = plan
            .get("steps")
            .and_then(Value::as_array)
            .expect("plan must include steps")
            .iter()
            .filter_map(|step| step.get("tool"))
            .filter_map(Value::as_str)
            .collect::<Vec<&str>>();
        assert!(tools.contains(&"anim.create_state_machine"));
        assert!(tools.contains(&"anim.add_state"));
        assert!(tools.contains(&"anim.add_animator"));
        assert!(tools.contains(&"mount.create_horse_template"));
        assert!(tools.contains(&"mount.spawn_horse"));
        assert!(tools.contains(&"mount.mount_rider"));
    }

    #[test]
    fn gen_plan_from_prompt_modelado_includes_modeling_steps() {
        let mut runtime = ToolRuntime::new(".");
        let plan = runtime
            .execute(
                "gen.plan_from_prompt",
                json!({
                    "prompt": "haz modelado base con sculpt"
                }),
            )
            .expect("gen.plan_from_prompt should succeed");
        let tools = plan
            .get("steps")
            .and_then(Value::as_array)
            .expect("plan must include steps")
            .iter()
            .filter_map(|step| step.get("tool"))
            .filter_map(Value::as_str)
            .collect::<Vec<&str>>();
        assert!(tools.contains(&"model.create_primitive"));
        assert!(tools.contains(&"model.extrude"));
        assert!(tools.contains(&"model.sculpt_brush"));
    }

    #[test]
    fn gen_plan_from_prompt_water_includes_water_steps() {
        let mut runtime = ToolRuntime::new(".");
        let plan = runtime
            .execute(
                "gen.plan_from_prompt",
                json!({
                    "prompt": "create an ocean scene with a barco"
                }),
            )
            .expect("gen.plan_from_prompt should succeed");
        let tools = plan
            .get("steps")
            .and_then(Value::as_array)
            .expect("plan must include steps")
            .iter()
            .filter_map(|step| step.get("tool"))
            .filter_map(Value::as_str)
            .collect::<Vec<&str>>();
        assert!(tools.contains(&"water.create_ocean"));
        assert!(tools.contains(&"water.add_buoyancy"));
        assert!(tools.contains(&"water.add_drag"));
    }

    #[test]
    fn gen_plan_from_prompt_vfx_includes_vfx_steps() {
        let mut runtime = ToolRuntime::new(".");
        let plan = runtime
            .execute(
                "gen.plan_from_prompt",
                json!({
                    "prompt": "make vfx particle smoke burst"
                }),
            )
            .expect("gen.plan_from_prompt should succeed");
        let tools = plan
            .get("steps")
            .and_then(Value::as_array)
            .expect("plan must include steps")
            .iter()
            .filter_map(|step| step.get("tool"))
            .filter_map(Value::as_str)
            .collect::<Vec<&str>>();
        assert!(tools.contains(&"vfx.create_particle_system"));
        assert!(tools.contains(&"vfx.set_emitter"));
        assert!(tools.contains(&"vfx.compile_graph"));
    }

    #[test]
    fn gen_plan_from_prompt_ai_includes_ai_steps() {
        let mut runtime = ToolRuntime::new(".");
        let plan = runtime
            .execute(
                "gen.plan_from_prompt",
                json!({
                    "prompt": "create npc enemy ai with navmesh and behavior tree"
                }),
            )
            .expect("gen.plan_from_prompt should succeed");
        let tools = plan
            .get("steps")
            .and_then(Value::as_array)
            .expect("plan must include steps")
            .iter()
            .filter_map(|step| step.get("tool"))
            .filter_map(Value::as_str)
            .collect::<Vec<&str>>();
        assert!(tools.contains(&"ai.create_navmesh"));
        assert!(tools.contains(&"ai.add_agent"));
        assert!(tools.contains(&"ai.create_behavior_tree"));
        assert!(tools.contains(&"ai.assign_behavior"));
        assert!(tools.contains(&"ai.set_blackboard"));
    }

    #[test]
    fn gen_plan_from_prompt_ui_includes_ui_steps() {
        let mut runtime = ToolRuntime::new(".");
        let plan = runtime
            .execute(
                "gen.plan_from_prompt",
                json!({
                    "prompt": "create hud ui interface for player stats"
                }),
            )
            .expect("gen.plan_from_prompt should succeed");
        let tools = plan
            .get("steps")
            .and_then(Value::as_array)
            .expect("plan must include steps")
            .iter()
            .filter_map(|step| step.get("tool"))
            .filter_map(Value::as_str)
            .collect::<Vec<&str>>();
        assert!(tools.contains(&"ui.create_canvas"));
        assert!(tools.contains(&"ui.add_text"));
        assert!(tools.contains(&"ui.bind_to_data"));
        assert!(tools.contains(&"ui.create_hud_template"));
    }

    #[test]
    fn gen_plan_from_prompt_audio_includes_audio_steps() {
        let mut runtime = ToolRuntime::new(".");
        let plan = runtime
            .execute(
                "gen.plan_from_prompt",
                json!({
                    "prompt": "add audio sound and music setup"
                }),
            )
            .expect("gen.plan_from_prompt should succeed");
        let tools = plan
            .get("steps")
            .and_then(Value::as_array)
            .expect("plan must include steps")
            .iter()
            .filter_map(|step| step.get("tool"))
            .filter_map(Value::as_str)
            .collect::<Vec<&str>>();
        assert!(tools.contains(&"audio.import_clip"));
        assert!(tools.contains(&"audio.create_source"));
        assert!(tools.contains(&"audio.create_mixer"));
        assert!(tools.contains(&"audio.route"));
        assert!(tools.contains(&"audio.play"));
    }

    #[test]
    fn gen_plan_from_prompt_network_includes_net_steps() {
        let mut runtime = ToolRuntime::new(".");
        let plan = runtime
            .execute(
                "gen.plan_from_prompt",
                json!({
                    "prompt": "setup multiplayer network replication baseline"
                }),
            )
            .expect("gen.plan_from_prompt should succeed");
        let tools = plan
            .get("steps")
            .and_then(Value::as_array)
            .expect("plan must include steps")
            .iter()
            .filter_map(|step| step.get("tool"))
            .filter_map(Value::as_str)
            .collect::<Vec<&str>>();
        assert!(tools.contains(&"net.create_server"));
        assert!(tools.contains(&"net.connect_client"));
        assert!(tools.contains(&"net.enable_replication"));
        assert!(tools.contains(&"net.set_prediction"));
        assert!(tools.contains(&"net.set_rollback"));
    }

    #[test]
    fn gen_plan_from_prompt_build_includes_build_steps() {
        let mut runtime = ToolRuntime::new(".");
        let plan = runtime
            .execute(
                "gen.plan_from_prompt",
                json!({
                    "prompt": "prepare build export package and installer"
                }),
            )
            .expect("gen.plan_from_prompt should succeed");
        let tools = plan
            .get("steps")
            .and_then(Value::as_array)
            .expect("plan must include steps")
            .iter()
            .filter_map(|step| step.get("tool"))
            .filter_map(Value::as_str)
            .collect::<Vec<&str>>();
        assert!(tools.contains(&"build.set_target"));
        assert!(tools.contains(&"build.set_bundle_id"));
        assert!(tools.contains(&"build.set_version"));
        assert!(tools.contains(&"build.enable_feature"));
        assert!(tools.contains(&"build.export_project"));
        assert!(tools.contains(&"build.generate_installer"));
    }

    #[test]
    fn gen_plan_from_prompt_debug_includes_debug_steps() {
        let mut runtime = ToolRuntime::new(".");
        let plan = runtime
            .execute(
                "gen.plan_from_prompt",
                json!({
                    "prompt": "run a debug profiler performance pass"
                }),
            )
            .expect("gen.plan_from_prompt should succeed");
        let tools = plan
            .get("steps")
            .and_then(Value::as_array)
            .expect("plan must include steps")
            .iter()
            .filter_map(|step| step.get("tool"))
            .filter_map(Value::as_str)
            .collect::<Vec<&str>>();
        assert!(tools.contains(&"debug.show_colliders"));
        assert!(tools.contains(&"debug.capture_frame"));
        assert!(tools.contains(&"debug.get_profiler_snapshot"));
        assert!(tools.contains(&"debug.find_performance_hotspots"));
    }

    #[test]
    fn gen_create_game_from_template_applies_template_and_runs_graph() {
        let mut runtime = ToolRuntime::new(".");
        let result = runtime
            .execute(
                "gen.create_game_from_template",
                json!({
                    "template_id": "template_platform_runner",
                    "auto_transaction": true
                }),
            )
            .expect("gen.create_game_from_template should succeed");
        assert_eq!(result.get("status").and_then(Value::as_str), Some("ok"));

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert_eq!(
            state
                .get("lowcode")
                .and_then(|lowcode| lowcode.get("active_template_id"))
                .and_then(Value::as_str),
            Some("template_platform_runner")
        );
    }

    #[test]
    fn gen_macro_tools_create_expected_worlds() {
        let mut runtime = ToolRuntime::new(".");

        let shooter = runtime
            .execute(
                "gen.create_shooter_arena",
                json!({
                    "auto_transaction": true
                }),
            )
            .expect("gen.create_shooter_arena should succeed");
        assert_eq!(shooter.get("status").and_then(Value::as_str), Some("ok"));
        let shooter_state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert!(
            shooter_state
                .get("gameplay")
                .and_then(|gameplay| gameplay.get("weapon_count"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
                >= 1
        );

        let platformer = runtime
            .execute(
                "gen.create_platformer_level",
                json!({
                    "auto_transaction": true
                }),
            )
            .expect("gen.create_platformer_level should succeed");
        assert_eq!(platformer.get("status").and_then(Value::as_str), Some("ok"));
        let platformer_state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert_eq!(
            platformer_state
                .get("lowcode")
                .and_then(|lowcode| lowcode.get("active_template_id"))
                .and_then(Value::as_str),
            Some("template_platform_runner")
        );

        let island = runtime
            .execute(
                "gen.create_island_adventure",
                json!({
                    "auto_transaction": true
                }),
            )
            .expect("gen.create_island_adventure should succeed");
        assert_eq!(island.get("status").and_then(Value::as_str), Some("ok"));
        let island_state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert_eq!(
            island_state
                .get("lowcode")
                .and_then(|lowcode| lowcode.get("active_template_id"))
                .and_then(Value::as_str),
            Some("template_medieval_island")
        );
    }

    #[test]
    fn gen_package_demo_build_produces_manifest_artifacts() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let project_root = std::env::temp_dir().join(format!("ai_package_demo_{}", unique_suffix));
        fs::create_dir_all(project_root.join("samples")).expect("should create temp project root");

        let mut runtime = ToolRuntime::new(&project_root);
        let result = runtime
            .execute(
                "gen.package_demo_build",
                json!({
                    "target": "windows",
                    "bundle_id": "com.rey30.demo.pkg",
                    "version": "2.0.0",
                    "features": ["demo_mode"],
                    "export_path": "dist/demo_pkg_export",
                    "installer_path": "dist/demo_pkg_installer",
                    "run_build": false
                }),
            )
            .expect("gen.package_demo_build should succeed");
        assert_eq!(result.get("status").and_then(Value::as_str), Some("ok"));

        let export_manifest = project_root
            .join("dist")
            .join("demo_pkg_export")
            .join("export_manifest.json");
        let installer_manifest = project_root
            .join("dist")
            .join("demo_pkg_installer")
            .join("installer_manifest.json");
        assert!(export_manifest.exists());
        assert!(installer_manifest.exists());

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert_eq!(
            state
                .get("build")
                .and_then(|build| build.get("target"))
                .and_then(Value::as_str),
            Some("windows")
        );
        assert_eq!(
            state
                .get("build")
                .and_then(|build| build.get("bundle_id"))
                .and_then(Value::as_str),
            Some("com.rey30.demo.pkg")
        );
        assert_eq!(
            state
                .get("build")
                .and_then(|build| build.get("version"))
                .and_then(Value::as_str),
            Some("2.0.0")
        );
        assert_eq!(
            state
                .get("build")
                .and_then(|build| build.get("enabled_feature_count"))
                .and_then(Value::as_u64),
            Some(1)
        );

        fs::remove_dir_all(project_root).expect("temp project root cleanup should succeed");
    }

    #[test]
    fn scene_duplicate_creates_target_file() {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        let project_root = std::env::temp_dir().join(format!("ai_scene_dup_{}", unique_suffix));
        fs::create_dir_all(project_root.join("samples")).expect("should create temp project root");

        let mut runtime = ToolRuntime::new(&project_root);
        runtime
            .execute(
                "scene.create",
                json!({
                    "name": "SourceScene"
                }),
            )
            .expect("scene.create should succeed");
        runtime
            .execute(
                "entity.create",
                json!({
                    "name": "Crate",
                    "mesh": "cube",
                    "translation": [1.0, 2.0, 3.0]
                }),
            )
            .expect("entity.create should succeed");
        runtime
            .execute(
                "scene.save_as",
                json!({
                    "name": "source_scene"
                }),
            )
            .expect("scene.save_as should succeed");
        runtime
            .execute(
                "scene.duplicate",
                json!({
                    "scene_id": "source_scene",
                    "name": "source_scene_copy"
                }),
            )
            .expect("scene.duplicate should succeed");

        let duplicated_path = project_root.join("samples").join("source_scene_copy.json");
        assert!(duplicated_path.exists());
        let duplicated_raw = fs::read_to_string(&duplicated_path).expect("duplicate file readable");
        let duplicated_scene: SceneFile =
            serde_json::from_str(&duplicated_raw).expect("duplicate scene json should parse");
        assert_eq!(duplicated_scene.name, "SourceScene");
        assert_eq!(duplicated_scene.entities.len(), 1);

        fs::remove_dir_all(project_root).expect("temp project root cleanup should succeed");
    }

    #[test]
    fn render_set_postprocess_preset_updates_extended_fields() {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "render.set_postprocess",
                json!({
                    "preset": "filmic_sunset"
                }),
            )
            .expect("render.set_postprocess should succeed");

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert_eq!(
            state
                .get("render")
                .and_then(|render| render.get("color_grading_preset"))
                .and_then(Value::as_str),
            Some("filmic_sunset")
        );
        assert!(
            state
                .get("render")
                .and_then(|render| render.get("bloom_intensity"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                > 0.2
        );
        assert!(
            state
                .get("render")
                .and_then(|render| render.get("fog_density"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                > 0.01
        );
    }

    #[test]
    fn render_set_ibl_updates_engine_state() {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "render.set_ibl",
                json!({
                    "intensity": 0.9,
                    "sky_color": [0.9, 0.95, 1.0],
                    "ground_color": [0.22, 0.20, 0.18]
                }),
            )
            .expect("render.set_ibl should succeed");

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        let ibl_intensity = state
            .get("render")
            .and_then(|render| render.get("ibl_intensity"))
            .and_then(Value::as_f64)
            .unwrap_or_default();
        assert!((ibl_intensity - 0.9).abs() < 1e-4);
    }

    #[test]
    fn render_set_lod_settings_updates_engine_state() {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "render.set_lod_settings",
                json!({
                    "near_distance": 14.0,
                    "far_distance": 52.0,
                    "hysteresis": 2.5
                }),
            )
            .expect("render.set_lod_settings should succeed");

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        let near = state
            .get("render")
            .and_then(|render| render.get("lod_transition_distances"))
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(Value::as_f64)
            .unwrap_or_default();
        let hysteresis = state
            .get("render")
            .and_then(|render| render.get("lod_hysteresis"))
            .and_then(Value::as_f64)
            .unwrap_or_default();
        assert!((near - 14.0).abs() < 1e-4);
        assert!((hysteresis - 2.5).abs() < 1e-4);
    }

    #[test]
    fn template_apply_and_graph_run_update_lowcode_state() {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "template.apply",
                json!({
                    "template_id": "template_shooter_arena"
                }),
            )
            .expect("template.apply should succeed");
        runtime
            .execute(
                "graph.run",
                json!({
                    "events": ["OnStart"]
                }),
            )
            .expect("graph.run should succeed");

        let state = runtime
            .execute("tool.get_engine_state", json!({}))
            .expect("tool.get_engine_state should succeed");
        assert_eq!(
            state
                .get("lowcode")
                .and_then(|lowcode| lowcode.get("active_template_id"))
                .and_then(Value::as_str),
            Some("template_shooter_arena")
        );
        assert!(
            state
                .get("scene")
                .and_then(|scene| scene.get("entity_count"))
                .and_then(Value::as_u64)
                .unwrap_or(0)
                >= 4
        );
    }

    #[test]
    fn graph_editing_tools_create_and_validate_graph() {
        let mut runtime = ToolRuntime::new(".");
        runtime
            .execute(
                "graph.create",
                json!({
                    "graph_name": "test_graph"
                }),
            )
            .expect("graph.create should succeed");
        runtime
            .execute(
                "graph.add_node",
                json!({
                    "id": "start",
                    "type": "OnStart"
                }),
            )
            .expect("graph.add_node should succeed");
        runtime
            .execute(
                "graph.add_node",
                json!({
                    "id": "spawn",
                    "type": "SpawnEntity",
                    "params": {
                        "base_name": "TestEnemy",
                        "count": 1
                    }
                }),
            )
            .expect("graph.add_node should succeed");
        runtime
            .execute(
                "graph.connect",
                json!({
                    "from": "start",
                    "to": "spawn",
                    "pin": "flow"
                }),
            )
            .expect("graph.connect should succeed");

        let report = runtime
            .execute("graph.validate", json!({}))
            .expect("graph.validate should succeed");
        assert_eq!(
            report
                .get("validation")
                .and_then(|validation| validation.get("valid"))
                .and_then(Value::as_bool),
            Some(true)
        );
    }
}
