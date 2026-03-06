# Ideas Nuevas - Mapeo de Implementacion

Fecha: 2026-03-05

## 1) Objetivo

`nuevas_Ideas.txt` define un contrato grande de `tool-calling + command bus`.
Este documento aterriza ese contrato al estado real del repo y fija donde va cada parte.

## 2) Estado Actual del Repo

- `ai/`: runtime AI, modos `OFF/API/LOCAL`, world builder y audit log.
- `editor/`: loop principal + hotkeys AI + render en tiempo real.
- `render/`: PBR baseline + HDR + tone mapping + shadow map direccional.
- `assets/`: carga de escenas JSON.
- `ecs/`: world ECS minimo.
- `tools/`: CLIs (`world_builder`) y ahora CLI de tool-call.

## 3) Mapeo de prefijos de tools a modulos/crates

- `tool.*`:
  - Implementacion: `ai/src/tool_registry.rs`
  - Estado/transacciones/comandos: `ai/src/command_bus.rs`
- `scene.*`:
  - Commands en `ai/src/command_bus.rs`
  - Persistencia JSON: `assets` + filesystem
- `entity.*`:
  - Commands en `ai/src/command_bus.rs`
  - ECS real (siguiente fase): `ecs/` + sincronizacion editor/runtime
- `asset.*`:
  - Base existente en `assets/`
  - Pipeline/importadores avanzados: nuevo modulo en `assets/` (fase siguiente)
- `render.*`:
  - Va en `render/` (pipeline controls, lights, captures)
- `phys.*`:
  - Recomendado nuevo crate `physics/` y adaptador en `editor`/`ecs`
- `vfx.*`, `water.*`, `anim.*`, `ui.*`, `audio.*`:
  - Baseline actual en `ai/src/tool_registry.rs` + `ai/src/command_bus.rs`
  - Recomendado mover a crates dedicados por dominio para no acoplar `render/ai`
- `build.*`:
  - Va en `tools/` (orquestacion) + scripts de build/export
- `gen.*` (macro-tools prompt->plan->execute):
  - Planner/Executor en `ai/`
  - Ejecucion efectiva via `tool_registry` (no acceso directo a memoria)

## 4) Base ya implementada en esta iteracion

- `Command Bus` con contrato:
  - `validate/execute/undo/serialize/cost_estimate`
  - `submit`, `submit_batch`, `get_status`, `replay`
- Transacciones:
  - `begin`, `commit`, `rollback`, `checkpoint`, `rollback_to`
- Historial:
  - `undo`, `redo`, `mark`, `jump_to`
- Tool registry MVP:
  - `tool.get_engine_state`
  - `tool.get_project_tree`
  - `tool.search_assets`
  - `tool.read_asset_metadata`
  - `tool.get_selection` / `tool.set_selection`
  - `tool.get_viewport_camera` / `tool.set_viewport_camera`
  - `tool.begin/commit/rollback_transaction`
  - `tool.create_checkpoint` / `tool.rollback_to_checkpoint`
  - `tool.log`, `tool.open_task`, `tool.update_task`, `tool.close_task`
  - `scene.create/open/save/save_as/duplicate/close`
  - `scene.set_sky/scene.set_time_of_day/scene.add_fog/scene.add_postprocess`
  - `scene.enable_world_streaming/scene.create_stream_chunk/scene.assign_entity_to_chunk`
  - `entity.create/get_transform/set_transform/add_component`
  - `history.undo/redo/mark/jump_to`
- Integracion `entity.* -> ECS`:
  - `CommandContext` mantiene `runtime_world` (`ecs::SceneWorld`) sincronizado por comando.
  - Cada `execute/undo/redo/rollback` reconstruye ECS desde escena + componentes dinamicos.
  - `tool.get_engine_state` ahora reporta `ecs_entities`.
- Integracion editor en vivo:
  - `AiOrchestrator` expone `tool_scene_snapshot`, `tool_scene_revision`, `tool_render_settings`.
  - `editor` aplica sync de escena+ECS por revision y actualiza render settings por frame.
  - `F7` ejecuta `gen.plan_from_prompt` + `gen.execute_plan`.
- Tools nuevos de expansion:
  - `asset.import_file`, `asset.create_material`, `asset.instantiate_prefab`
  - `asset.get_template_bundle`, `asset.validate_template_bundle`
  - `render.create_light`, `render.set_light_params`, `render.set_postprocess`, `render.set_lod_settings`
  - `graph.create`, `graph.add_node`, `graph.connect`, `graph.delete_node`, `graph.delete_edge`, `graph.set_node_params`, `graph.validate`, `graph.run`
  - `template.list`, `template.apply`
  - `phys.add_collider`, `phys.set_collider`, `phys.remove_collider`, `phys.add_rigidbody`, `phys.set_mass`, `phys.set_friction`, `phys.set_restitution`
  - `phys.apply_force`, `phys.apply_impulse`, `phys.set_gravity`, `phys.raycast`, `phys.overlap`
  - `phys.add_character_controller`, `phys.character_move`, `phys.character_jump`, `phys.character_set_state`
  - `game.create_input_action`, `game.bind_action`, `game.set_rebind`
  - `game.create_weapon`, `game.attach_weapon`, `game.fire_weapon`, `game.apply_damage`, `game.add_health_component`
  - `game.add_trigger`, `game.add_pickup`, `game.add_inventory`, `game.add_interactable`
  - `anim.add_animator`, `anim.create_state_machine`, `anim.add_state`, `anim.add_transition`, `anim.set_parameter`
  - `anim.play`, `anim.blend`, `anim.add_ik`, `anim.retarget`, `anim.bake_animation`
  - `model.create_primitive`, `model.enter_edit_mode`, `model.exit_edit_mode`, `model.select`
  - `model.extrude`, `model.inset`, `model.bevel`, `model.loop_cut`, `model.knife`, `model.merge`, `model.subdivide`, `model.triangulate`
  - `model.add_modifier`, `model.set_modifier`, `model.apply_modifier`, `model.remove_modifier`
  - `model.unwrap_uv`, `model.pack_uv`, `model.generate_lightmap_uv`
  - `model.voxel_remesh`, `model.decimate`, `model.smooth`, `model.sculpt_brush`, `model.sculpt_mask`
  - `vfx.create_particle_system`, `vfx.set_emitter`, `vfx.set_forces`, `vfx.set_collision`, `vfx.set_renderer`, `vfx.attach_to_entity`
  - `vfx.create_graph`, `vfx.add_node`, `vfx.connect`, `vfx.compile_graph`
  - `water.create_ocean`, `water.create_river`, `water.create_waterfall`, `water.set_waves`
  - `water.enable_foam`, `water.enable_refraction`, `water.enable_caustics`
  - `water.add_buoyancy`, `water.add_drag`, `water.sample_height`
  - `mount.create_horse_template`, `mount.spawn_horse`, `mount.mount_rider`, `mount.dismount`, `mount.set_gait`, `mount.set_path_follow`
  - `ai.create_navmesh`, `ai.bake_navmesh`, `ai.add_agent`, `ai.set_destination`
  - `ai.create_behavior_tree`, `ai.bt_add_node`, `ai.bt_connect`, `ai.assign_behavior`, `ai.set_blackboard`
  - `ui.create_canvas`, `ui.add_panel`, `ui.add_text`, `ui.add_button`, `ui.bind_to_data`, `ui.create_hud_template`
  - `audio.import_clip`, `audio.create_source`, `audio.play`, `audio.set_spatial`, `audio.create_mixer`, `audio.route`
  - `gen.plan_from_prompt`, `gen.execute_plan`, `gen.validate_gameplay`
  - `build.build_and_run`
  - `gen.execute_plan` ahora usa transaccion automatica (`begin/commit/rollback`) por defecto.
- S5 bridge:
  - `execute_tool` intenta remote RPC en `API`/`LOCAL` (si enabled) y hace fallback a `ToolRuntime` local cuando no es strict.
- Integracion con `AiOrchestrator`:
  - `execute_tool(tool_name, params_json)`
  - `tool_catalog()`
  - Auditoria unificada de tool-calls en `logs/ai_tool_calls/*.log`
- CLI nuevo:
  - `cargo run -p tools --bin tool_call -- <tool.name> [json_params]`
- S6/S7 base:
  - `CommandContext` mantiene `physics` + `gameplay` runtime state persistente y serializable en `tool.get_engine_state`.
  - `gen.plan_from_prompt` (caso shooter) ahora incluye pasos de configuracion fisica y combate.
  - Pruebas de integracion para `phys.*` + `game.*` agregadas en `ai/src/tool_registry.rs`.
- S8/S9 expansion:
  - Estado fisico incluye `character_controllers`; gameplay incluye `input_actions`, `triggers`, `pickups`, `inventories`, `interactables`.
  - Se agrego query `phys.overlap` (box/sphere) y controles de rigidbody (`set_mass/set_friction/set_restitution/apply_force`).
  - `gen.plan_from_prompt` para shooter ahora agrega setup de input action + character controller.
- S10/S11 expansion:
  - Estado runtime incluye `animation` (controllers/states/transitions/ik/retarget/bake) y `modeling` (meshes/edit/modifiers/uv/sculpt/log).
  - Tool-calling de animacion/modelado se implemento con comandos genericos undoables (`AnimMutationCommand`, `ModelMutationCommand`).
  - Planner agrega ramas para prompts de animacion de personaje/caballo y modelado/sculpt.
- S12/S13 expansion:
  - Estado runtime incluye `vfx` (particle systems + graphs) y `water` (oceans/rivers/waterfalls/buoyancy/drag).
  - Tool-calling de VFX/Water se implemento con comandos genericos undoables (`VfxMutationCommand`, `WaterMutationCommand`).
  - Planner agrega ramas para prompts de humo/particulas y agua/oceano/barco.
- S14/S15 expansion:
  - Estado runtime incluye `mount` (horse templates/horses/rider bindings) y `ai` (navmesh/agents/behavior trees/blackboard).
  - Tool-calling de mounts/NPC AI se implemento con comandos genericos undoables (`MountMutationCommand`, `NpcAiMutationCommand`).
  - Planner agrega ramas para prompts de caballo montable y npc/navmesh/behavior tree.
- S16/S17 expansion:
  - Estado runtime incluye `ui` (canvases/elements/bindings/hud template) y `audio` (clips/sources/mixers/play events).
  - Tool-calling de UI/Audio se implemento con comandos genericos undoables (`UiMutationCommand`, `AudioMutationCommand`).
  - Planner agrega ramas para prompts de HUD/UI y audio/sound/music.
- S18/S19/S20 expansion:
  - Estado runtime incluye `networking` (server/client/replication/prediction/rollback), `build` (target/bundle/version/features/export paths) y `debug` (toggles/capturas/profiler snapshots).
  - Tool-calling de networking/build/debug se implemento con comandos undoables (`NetMutationCommand`, `BuildMutationCommand`, `DebugMutationCommand`) y consultas read-only de profiler/hotspots.
  - Planner agrega ramas para prompts de multiplayer/networking, build/export/installer y debug/performance.
- S21 expansion:
  - Se agregan macro-tools `gen.create_game_from_template`, `gen.create_platformer_level`, `gen.create_shooter_arena`, `gen.create_island_adventure` y `gen.package_demo_build`.
  - Los macro-tools encapsulan planes multi-step y ejecutan el flujo via `gen.execute_plan` para mantener trazabilidad/rollback.
  - Se extiende contrato de tools y pruebas de integracion para cubrir fase 21 completa.
- S22 expansion:
  - Se agrega capa de contexto de ciclo para MLL con `tool.get_cycle_context`, `tool.get_rules`, `tool.get_project_memory/set_project_memory`, `tool.get_constraints/set_constraints`, `tool.set_objective`.
  - Se agrega feedback operacional con `tool.get_diagnostics` / `tool.clear_diagnostics` y captura de warning/error desde `tool.log` y fallos de tool-calls.
  - `tool.get_engine_state` incorpora `project_memory`, `constraints` y resumen de diagnosticos.
- S23 expansion:
  - Se completa el pack inicial recomendado con `asset.assign_material` (y alias `render.assign_material`) para binding de materiales por entidad/slot.
  - Se extiende contrato y pruebas de integracion para cubrir fase 23 completa.
- S24 expansion:
  - Se completa `entity.*` lifecycle/hierarchy/search con `entity.clone`, `entity.delete`, `entity.rename`, `entity.parent`, `entity.unparent`, `entity.find_by_name`, `entity.find_by_tag`.
  - `entity.rename` ahora remapea referencias runtime por entidad (selection/componentes/fisica/gameplay/UI/audio/networking) y `entity.delete` usa snapshot para `undo` estable.
- S25 expansion:
  - Se completa `entity.*` transform/component contract con `entity.translate`, `entity.rotate`, `entity.scale`, `entity.remove_component`, `entity.get_component`, `entity.set_component`.
  - `entity.get_transform` ahora reporta `translation + rotation + scale` (rotation/scale persistidos en componentes dinamicos).
  - Se extiende contrato y pruebas de integracion para cubrir fase 25 completa.
- S26 expansion:
  - Se completa bloque `asset.*` de creacion/import con `asset.import_url`, `asset.create_texture`, `asset.create_shader`, `asset.create_prefab`, `asset.save_prefab`.
  - Se agrega estado runtime de assets (`textures`, `shaders`, `prefabs`) y serializacion en `tool.get_engine_state`.
- S27 expansion:
  - Se completa bloque `asset.*` de proceso/optimizacion con `asset.rebuild_import`, `asset.generate_lods`, `asset.mesh_optimize`, `asset.compress_textures`, `asset.bake_lightmaps`, `asset.bake_reflection_probes`.
  - Se agrega estado runtime `assets.pipeline` para historial de rebuild/LOD/optimizacion/compresion/bake.
  - Se extiende contrato y pruebas de integracion para cubrir fase 27 completa.

## 5) Secuencia recomendada para implementar "todo al tiempo" sin romper

1. Consolidar Pack MVP (esta base) con pruebas de regresion.
2. Agregar `asset.*` y `render.*` de control cinematografico.
3. Conectar `entity.*` a ECS runtime real (no solo escena JSON).
4. Introducir `gen.plan_from_prompt` + `gen.execute_plan` como macro-tools.
5. Expandir sobre la base actual (`phys.*`, `game.*`, `anim.*`, `model.*`, `vfx.*`, `water.*`, `mount.*`, `ai.*`, `ui.*`, `audio.*`, `net.*`, `build.*`, `debug.*`, `gen.*`) con loop de contexto/memoria/constraints para decisiones del MLL.

## 6) Regla de seguridad del contrato

- La IA opera solo por tools.
- Cada mutacion pasa por command bus.
- Todo cambio tiene trace/audit y rollback posible.
