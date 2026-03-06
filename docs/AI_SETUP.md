# AI Setup (OFF / API / LOCAL)

Date: 2026-03-04

## Security First

- Never commit secrets to git.
- Keep keys only in local `.env`.
- If any key was shared in plain text, rotate/revoke it immediately.

## Quick Start

1. Copy `.env.example` to `.env`.
2. Set `AI_MODE` to one of:
   - `OFF`
   - `API`
   - `LOCAL`
3. Launch editor:
   - `cargo run -p editor`

## API Mode

Required:

- `AI_MODE=API`
- `AI_API_PROVIDER` (example: `openai`, `anthropic`)
- `AI_API_KEY`

Optional:

- `AI_API_BASE_URL`

## LOCAL Mode (llama.cpp recommended)

Required:

- `AI_MODE=LOCAL`
- `LOCAL_MLL_BIN` (example: `C:\tools\llama.cpp\llama-server.exe`)
- `LOCAL_MLL_MODEL` (path to `.gguf`)

Optional:

- `LOCAL_MLL_HOST` (default: `127.0.0.1`)
- `LOCAL_MLL_PORT` (default: `8080`)
- `LOCAL_MLL_EXTRA_ARGS` (default example: `--ctx-size 4096`)
- `LOCAL_MLL_MAX_RESTARTS` (default: `2`)

The local model is executed in a separate process and supervised with restart limits.

## Editor Controls

- `F1`: switch to `OFF`
- `F2`: switch to `API`
- `F3`: switch to `LOCAL`
- `F4`: apply low-code template `template_shooter_arena`
- `F5`: apply low-code template `template_medieval_island`
- `F6`: run World Builder and save `samples/generated_scene.json`
- `F7`: run generator plan (`gen.plan_from_prompt` + `gen.execute_plan`)
- `F8`: apply cinematic preset `Natural Day`
- `F9`: apply cinematic preset `Filmic Sunset`
- `F10`: apply cinematic preset `Noir Indoor`
- `F11`: toggle profiler panel (CPU/cull/draw/LOD/cache metrics)
- `F12`: apply low-code template `template_platform_runner`
- `G`: run `graph.run` with `OnUpdate`
- `V`: run `graph.validate`

## Performance Tuning Env

- `ASSET_CACHE_MB`: async asset-cache capacity in MB (default: `128`)

## S5 Remote Tool-Calling Flags

- API bridge:
  - `AI_API_REMOTE_TOOL_CALLS` (`true/false`, default `false`)
  - `AI_API_REMOTE_TOOL_CALLS_STRICT` (`true/false`, default `false`)
  - `AI_API_TOOL_ENDPOINT` (optional direct endpoint; if missing uses `AI_API_BASE_URL + /tool-call`)
  - `AI_API_TIMEOUT_MS` (default `8000`)

- LOCAL RPC bridge:
  - `LOCAL_MLL_RPC_TOOL_CALLS` (`true/false`, default `false`)
  - `LOCAL_MLL_RPC_TOOL_CALLS_STRICT` (`true/false`, default `false`)
  - `LOCAL_MLL_RPC_PATH` (default `/tool-call`)
  - `LOCAL_MLL_RPC_TIMEOUT_MS` (default `5000`)

## Audit Logs

- Tool calls are append-only JSONL entries under:
  - `logs/ai_tool_calls/YYYY-MM-DD.log`

## S6/S7 Quick Smoke (Tool CLI)

Example sequence:

```bash
cargo run -p tools --bin tool_call -- scene.create "{\"name\":\"S67 Smoke\"}"
cargo run -p tools --bin tool_call -- entity.create "{\"name\":\"Hero\",\"mesh\":\"capsule\",\"translation\":[0,1,0]}"
cargo run -p tools --bin tool_call -- phys.add_collider "{\"entity_id\":\"Hero\",\"shape\":\"capsule\",\"size\":[0.8,1.8,0.8]}"
cargo run -p tools --bin tool_call -- phys.add_rigidbody "{\"entity_id\":\"Hero\",\"type\":\"dynamic\",\"mass\":75}"
cargo run -p tools --bin tool_call -- game.create_weapon "{\"weapon_id\":\"rifle\",\"ammo_capacity\":30}"
cargo run -p tools --bin tool_call -- game.attach_weapon "{\"character_id\":\"Hero\",\"weapon_id\":\"rifle\"}"
cargo run -p tools --bin tool_call -- game.fire_weapon "{\"character_id\":\"Hero\"}"
cargo run -p tools --bin tool_call -- tool.get_engine_state
```

S8/S9 extra checks:

```bash
cargo run -p tools --bin tool_call -- phys.add_character_controller "{\"entity_id\":\"Hero\",\"speed\":5.5,\"jump_strength\":6.8}"
cargo run -p tools --bin tool_call -- phys.character_move "{\"entity_id\":\"Hero\",\"input\":[1,0,0],\"dt\":0.016}"
cargo run -p tools --bin tool_call -- game.create_input_action "{\"name\":\"Shoot\",\"bindings\":[\"MouseLeft\",\"GamepadRT\"]}"
cargo run -p tools --bin tool_call -- game.bind_action "{\"name\":\"Shoot\",\"target_script_event\":\"weapon_fire\"}"
cargo run -p tools --bin tool_call -- game.add_inventory "{\"entity_id\":\"Hero\",\"capacity\":6,\"items\":[]}"
```

S10/S11 extra checks:

```bash
cargo run -p tools --bin tool_call -- anim.create_state_machine "{\"name\":\"HeroController\",\"controller_id\":\"hero_ctrl\"}"
cargo run -p tools --bin tool_call -- anim.add_state "{\"controller_id\":\"hero_ctrl\",\"state_name\":\"idle\",\"clip_id\":\"idle_clip\"}"
cargo run -p tools --bin tool_call -- anim.add_animator "{\"entity_id\":\"Hero\",\"controller_id\":\"hero_ctrl\"}"
cargo run -p tools --bin tool_call -- model.create_primitive "{\"type\":\"cube\",\"name\":\"BlockA\",\"mesh_id\":\"mesh_block_a\"}"
cargo run -p tools --bin tool_call -- model.extrude "{\"mesh_id\":\"mesh_block_a\",\"params\":{\"distance\":0.4}}"
```

S12/S13 extra checks:

```bash
cargo run -p tools --bin tool_call -- vfx.create_particle_system "{\"name\":\"SmokeTrail\",\"id\":\"vfx_smoke_01\"}"
cargo run -p tools --bin tool_call -- vfx.set_emitter "{\"id\":\"vfx_smoke_01\",\"params\":{\"rate\":42,\"lifetime\":2.5}}"
cargo run -p tools --bin tool_call -- vfx.create_graph "{\"name\":\"SmokeGraph\",\"id\":\"vfx_graph_smoke\"}"
cargo run -p tools --bin tool_call -- water.create_ocean "{\"id\":\"ocean_main\",\"params\":{\"size\":[2048,2048],\"amplitude\":1.2,\"wavelength\":24.0,\"speed\":1.6}}"
cargo run -p tools --bin tool_call -- water.add_buoyancy "{\"entity_id\":\"Hero\",\"params\":{\"sample_points\":4,\"force\":1.0}}"
cargo run -p tools --bin tool_call -- water.sample_height "{\"ocean_id\":\"ocean_main\",\"position\":[0,0,0]}"
```

S14/S15 extra checks:

```bash
cargo run -p tools --bin tool_call -- mount.create_horse_template "{\"template_id\":\"horse_tpl_demo\",\"params\":{\"mesh\":\"horse\",\"stats\":{\"speed\":8.0}}}"
cargo run -p tools --bin tool_call -- mount.spawn_horse "{\"template_id\":\"horse_tpl_demo\",\"horse_id\":\"horse_demo\",\"entity_id\":\"HorseDemo\",\"translation\":[2,0,0]}"
cargo run -p tools --bin tool_call -- mount.mount_rider "{\"horse_id\":\"horse_demo\",\"rider_id\":\"Hero\"}"
cargo run -p tools --bin tool_call -- ai.create_navmesh "{\"navmesh_id\":\"navmesh_main\",\"params\":{\"cell_size\":0.2}}"
cargo run -p tools --bin tool_call -- ai.add_agent "{\"entity_id\":\"Hero\",\"agent_id\":\"hero_ai\",\"params\":{\"speed\":3.5}}"
cargo run -p tools --bin tool_call -- ai.create_behavior_tree "{\"name\":\"GuardTree\",\"tree_id\":\"bt_guard\"}"
cargo run -p tools --bin tool_call -- ai.bt_add_node "{\"tree_id\":\"bt_guard\",\"node_type\":\"Selector\",\"node_id\":\"root\"}"
cargo run -p tools --bin tool_call -- ai.assign_behavior "{\"entity_id\":\"Hero\",\"tree_id\":\"bt_guard\"}"
```

S16/S17 extra checks:

```bash
cargo run -p tools --bin tool_call -- ui.create_canvas "{\"name\":\"MainHUD\",\"canvas_id\":\"hud_main\"}"
cargo run -p tools --bin tool_call -- ui.add_text "{\"canvas_id\":\"hud_main\",\"ui_id\":\"hud_health\",\"text\":\"HP: 100\"}"
cargo run -p tools --bin tool_call -- ui.create_hud_template "{\"type\":\"shooter\"}"
cargo run -p tools --bin tool_call -- audio.import_clip "{\"path\":\"Cargo.toml\",\"clip_id\":\"clip_demo\"}"
cargo run -p tools --bin tool_call -- audio.create_mixer "{\"bus_id\":\"master\",\"params\":{\"volume\":1.0}}"
cargo run -p tools --bin tool_call -- audio.create_source "{\"source_id\":\"src_demo\",\"entity_id\":\"Hero\",\"params\":{\"loop\":false}}"
cargo run -p tools --bin tool_call -- audio.route "{\"source_id\":\"src_demo\",\"mixer_bus\":\"master\"}"
cargo run -p tools --bin tool_call -- audio.play "{\"source_id\":\"src_demo\",\"clip_id\":\"clip_demo\"}"
```

S18/S19/S20 extra checks:

```bash
cargo run -p tools --bin tool_call -- net.create_server "{\"server_id\":\"server_main\",\"params\":{\"port\":7777}}"
cargo run -p tools --bin tool_call -- net.connect_client "{\"client_id\":\"client_a\",\"endpoint\":\"127.0.0.1:7777\",\"params\":{\"transport\":\"udp\"}}"
cargo run -p tools --bin tool_call -- net.enable_replication "{\"entity_id\":\"Hero\",\"components\":[\"Transform\",\"Health\"]}"
cargo run -p tools --bin tool_call -- build.set_target "{\"platform\":\"windows\"}"
cargo run -p tools --bin tool_call -- build.set_bundle_id "{\"id\":\"com.rey30.demo\"}"
cargo run -p tools --bin tool_call -- build.export_project "{\"path\":\"dist/export\"}"
cargo run -p tools --bin tool_call -- build.generate_installer "{\"path\":\"dist/installer\"}"
cargo run -p tools --bin tool_call -- debug.show_colliders "{\"on\":true}"
cargo run -p tools --bin tool_call -- debug.capture_frame "{}"
cargo run -p tools --bin tool_call -- debug.find_performance_hotspots "{\"last_n\":8}"
```

S21 extra checks:

```bash
cargo run -p tools --bin tool_call -- gen.create_game_from_template "{\"template_id\":\"template_platform_runner\"}"
cargo run -p tools --bin tool_call -- gen.create_platformer_level "{}"
cargo run -p tools --bin tool_call -- gen.create_shooter_arena "{}"
cargo run -p tools --bin tool_call -- gen.create_island_adventure "{}"
cargo run -p tools --bin tool_call -- gen.package_demo_build "{\"target\":\"windows\",\"bundle_id\":\"com.rey30.demo\",\"version\":\"1.0.0\",\"features\":[\"demo\"],\"run_build\":false}"
```

S22 extra checks:

```bash
cargo run -p tools --bin tool_call -- tool.set_project_memory "{\"style\":\"stylized\",\"target_platform\":\"windows\",\"target_fps\":60,\"notes\":[\"focus gameplay readability\"],\"tags\":[\"phase22\"]}"
cargo run -p tools --bin tool_call -- tool.set_constraints "{\"target_fps\":60,\"resolution\":\"1920x1080\",\"allow_external_assets\":false,\"max_gpu_memory_mb\":4096}"
cargo run -p tools --bin tool_call -- tool.set_objective "{\"objective\":\"haz isla con castillo a 60fps\"}"
cargo run -p tools --bin tool_call -- tool.log "{\"level\":\"warn\",\"message\":\"shadow cascade budget high\"}"
cargo run -p tools --bin tool_call -- tool.get_cycle_context "{\"max_entities\":16,\"recent_commands\":8,\"diagnostics_last_n\":16}"
cargo run -p tools --bin tool_call -- tool.get_rules
```

S23 extra checks:

```bash
cargo run -p tools --bin tool_call -- scene.create "{\"name\":\"S23 Material Smoke\"}"
cargo run -p tools --bin tool_call -- entity.create "{\"name\":\"CrateS23\",\"mesh\":\"cube\",\"translation\":[0,0,0]}"
cargo run -p tools --bin tool_call -- asset.create_material "{\"name\":\"crate_mat_s23\",\"preset\":\"pbr_default\",\"params\":{\"base_color\":[0.8,0.4,0.2]}}"
cargo run -p tools --bin tool_call -- asset.assign_material "{\"entity_id\":\"CrateS23\",\"material_id\":\"crate_mat_s23\",\"slot\":\"base\"}"
cargo run -p tools --bin tool_call -- render.assign_material "{\"entity_id\":\"CrateS23\",\"material_id\":\"crate_mat_s23\",\"slot\":\"overlay\"}"
```

S24 extra checks:

```bash
cargo run -p tools --bin tool_call -- scene.create "{\"name\":\"S24 Entity Lifecycle\"}"
cargo run -p tools --bin tool_call -- entity.create "{\"name\":\"RootS24\",\"mesh\":\"cube\",\"translation\":[0,0,0]}"
cargo run -p tools --bin tool_call -- entity.create "{\"name\":\"ChildS24\",\"mesh\":\"capsule\",\"translation\":[1,0,0]}"
cargo run -p tools --bin tool_call -- entity.parent "{\"child_id\":\"ChildS24\",\"parent_id\":\"RootS24\"}"
cargo run -p tools --bin tool_call -- entity.clone "{\"entity_id\":\"ChildS24\",\"name\":\"ChildS24Clone\",\"translation_offset\":[0.5,0,0],\"copy_components\":true,\"copy_parent\":true}"
cargo run -p tools --bin tool_call -- entity.rename "{\"entity_id\":\"ChildS24Clone\",\"name\":\"ChildS24Renamed\"}"
cargo run -p tools --bin tool_call -- entity.find_by_name "{\"query\":\"ChildS24\"}"
cargo run -p tools --bin tool_call -- entity.unparent "{\"child_id\":\"ChildS24Renamed\"}"
cargo run -p tools --bin tool_call -- entity.delete "{\"entity_id\":\"ChildS24Renamed\"}"
```

S25 extra checks:

```bash
cargo run -p tools --bin tool_call -- scene.create "{\"name\":\"S25 Transform+Component\"}"
cargo run -p tools --bin tool_call -- entity.create "{\"name\":\"ActorS25\",\"mesh\":\"cube\",\"translation\":[0,0,0]}"
cargo run -p tools --bin tool_call -- entity.translate "{\"entity_id\":\"ActorS25\",\"delta\":[2,1,-3]}"
cargo run -p tools --bin tool_call -- entity.rotate "{\"entity_id\":\"ActorS25\",\"delta\":[0,90,0]}"
cargo run -p tools --bin tool_call -- entity.scale "{\"entity_id\":\"ActorS25\",\"factor\":[2,1.5,0.5]}"
cargo run -p tools --bin tool_call -- entity.set_component "{\"entity_id\":\"ActorS25\",\"component_type\":\"Health\",\"data\":{\"value\":120}}"
cargo run -p tools --bin tool_call -- entity.get_component "{\"entity_id\":\"ActorS25\",\"component_type\":\"Health\"}"
cargo run -p tools --bin tool_call -- entity.get_transform "{\"entity_id\":\"ActorS25\"}"
cargo run -p tools --bin tool_call -- entity.remove_component "{\"entity_id\":\"ActorS25\",\"component_type\":\"Health\"}"
```

S26 extra checks:

```bash
cargo run -p tools --bin tool_call -- scene.create "{\"name\":\"S26 Asset Create\"}"
cargo run -p tools --bin tool_call -- entity.create "{\"name\":\"CrateS26\",\"mesh\":\"cube\",\"translation\":[0,0,0]}"
cargo run -p tools --bin tool_call -- asset.create_texture "{\"name\":\"crate_albedo_s26\",\"texture_id\":\"crate_albedo_s26\",\"width\":1024,\"height\":1024,\"format\":\"rgba8\",\"params\":{\"mipmaps\":true}}"
cargo run -p tools --bin tool_call -- asset.create_shader "{\"name\":\"crate_shader_s26\",\"shader_id\":\"crate_shader_s26\",\"template\":\"pbr_lit\",\"params\":{\"use_normal_map\":true}}"
cargo run -p tools --bin tool_call -- asset.create_prefab "{\"name\":\"crate_prefab_s26\",\"prefab_id\":\"crate_prefab_s26\",\"entity_id\":\"CrateS26\",\"metadata\":{\"category\":\"props\"}}"
cargo run -p tools --bin tool_call -- asset.save_prefab "{\"prefab_id\":\"crate_prefab_s26\"}"
```

S27 extra checks:

```bash
cargo run -p tools --bin tool_call -- asset.rebuild_import "{\"asset_id\":\"assets/imported/crate.glb\"}"
cargo run -p tools --bin tool_call -- asset.generate_lods "{\"mesh_id\":\"cube\",\"levels\":3,\"reduction\":0.5}"
cargo run -p tools --bin tool_call -- asset.mesh_optimize "{\"mesh_id\":\"cube\",\"profile\":\"aggressive\"}"
cargo run -p tools --bin tool_call -- asset.compress_textures "{\"asset_id\":\"crate_albedo_s26\",\"format\":\"bc7\",\"quality\":\"high\"}"
cargo run -p tools --bin tool_call -- asset.bake_lightmaps "{\"params\":{\"resolution\":1024}}"
cargo run -p tools --bin tool_call -- asset.bake_reflection_probes "{\"params\":{\"probe_count\":4}}"
```
