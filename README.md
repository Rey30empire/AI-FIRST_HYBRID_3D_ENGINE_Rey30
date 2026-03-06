# AI-First Hybrid 3D Engine

Starter workspace for the hybrid game engine roadmap.

## Current PR #1 scope

- Workspace + crate structure
- Window + main loop
- `wgpu` 3D render baseline (lit cube + depth)
- Simple scene loading from JSON
- Basic FPS and frame-time title update

## Requirements

- Rust toolchain (`rustup`, `cargo`, `rustc`)
- GPU driver with Vulkan/DX12/Metal backend support through `wgpu`

## Run

```bash
cargo run -p editor
```

The app opens a 1280x720 window and renders a lit 3D cube.

## Camera Controls (PR #2)

- Hold right mouse button + move mouse: orbit camera
- Mouse wheel: zoom
- `W/A/S/D`: move on ground plane
- `Space` / `Shift`: move up/down
- `E` / `Q`: alternate up/down controls

## Render Baseline (PR #3)

- Perspective camera uniform in shader
- Depth testing enabled
- Metallic/Roughness PBR shading with hemispheric IBL baseline (sky/ground ambient)

## HDR + Tone Mapping (PR #4)

- PBR pass renders to HDR offscreen target (`RGBA16F`)
- Fullscreen tone-mapping pass resolves HDR to swapchain output
- Exposure/gamma controls are centralized in a dedicated tone-map uniform

## Bloom + Fog + Grading (PR #7)

- Tone-map pass now includes bloom (intensity/threshold/radius)
- Postprocess stack supports fog blend color/density
- Color grading controls: white balance, tint, saturation, contrast
- Preset-ready grading pipeline (`natural_day`, `filmic_sunset`, `noir_indoor`)

## Directional Shadows (PR #6)

- Dedicated shadow-map pass before PBR lighting
- Cascaded depth shadow map (up to 3 cascades) sampled as depth array texture
- 4-tap PCF filtering with configurable bias/strength/cascade count

## Instancing + Culling (PR #8)

- Scene entities are rendered through GPU instancing
- CPU frustum culling filters instances per frame before draw
- Instanced draws are used in both shadow and PBR passes

## LOD + Profiler + Asset Cache (PR #9)

- Automatic 3-level LOD switching with hysteresis (LOD0 cube, LOD1 octa, LOD2 tetra)
- Runtime LOD tuning via `render.set_lod_settings` (`transition_distances`, `hysteresis`)
- Async streaming-ready asset cache with bounded memory + LRU-style eviction
- In-editor profiler toggle (`F11`) showing CPU/cull/draw/LOD/GPU-buffer/cache metrics

## Low-code MVP (S4)

- Node graph authoring tools: `graph.create/add_node/connect/delete_node/delete_edge/set_node_params/validate`
- Deterministic runtime graph execution: `graph.run` with event phases + side-effect commit
- One-click templates: `template_shooter_arena`, `template_medieval_island`, `template_platform_runner`
- Template asset bundles: `asset.get_template_bundle` + `asset.validate_template_bundle`

## AI Hybrid Bridge (S5)

- API remote tool-calling bridge (`AI_API_REMOTE_TOOL_CALLS=true`)
- LOCAL loopback RPC tool-calling bridge (`LOCAL_MLL_RPC_TOOL_CALLS=true`)
- Shared tool RPC schema (`schema_version/session_id/mode/tool_name/params/timestamp_utc`)
- Strict/fallback policies per mode (`*_STRICT=true|false`)

## Physics + Gameplay Baseline (S6/S7)

- `phys.*`: `add_collider`, `add_rigidbody`, `apply_impulse`, `set_gravity`, `raycast`
- `game.*`: `create_weapon`, `attach_weapon`, `fire_weapon`, `apply_damage`, `add_health_component`
- `tool.get_engine_state` now includes `physics` and `gameplay` runtime snapshots
- Shooter prompt planning (`gen.plan_from_prompt`) now emits baseline `phys.*` + `game.*` setup steps

## Physics + Gameplay Expansion (S8/S9)

- Extended `phys.*`: `set_collider`, `remove_collider`, `set_mass`, `set_friction`, `set_restitution`, `apply_force`, `overlap`
- Character controller tools: `phys.add_character_controller`, `phys.character_move`, `phys.character_jump`, `phys.character_set_state`
- Input mapping tools: `game.create_input_action`, `game.bind_action`, `game.set_rebind`
- Interaction tools: `game.add_trigger`, `game.add_pickup`, `game.add_inventory`, `game.add_interactable`
- `tool.get_engine_state` now includes controller/input/interaction sections and counters

## Animation + Modeling Workflow (S10/S11)

- `anim.*`: state machine authoring, animator binding, playback/blending, IK metadata, retarget/bake job queues
- `model.*`: primitive creation, edit mode/selection, topology ops, modifier stack, UV workflow, sculpt metadata
- `tool.get_engine_state` now includes `animation` and `modeling` sections
- `gen.plan_from_prompt` now includes animation plans for horse prompts and modeling plans for model/sculpt prompts

## VFX + Water Workflow (S12/S13)

- `vfx.*`: particle system authoring, emitter/forces/collision/renderer setup, entity attachment, VFX graph create/connect/compile
- `water.*`: ocean/river/waterfall creation, waves/foam/refraction/caustics toggles, buoyancy/drag helpers, water height sampling
- `tool.get_engine_state` now includes `vfx` and `water` sections
- `gen.plan_from_prompt` now includes VFX plans for smoke/particle prompts and water plans for ocean/river/boat prompts

## Mount + NPC AI Workflow (S14/S15)

- `mount.*`: horse template creation, horse spawn, rider mount/dismount, gait control, path-follow assignment
- `ai.*`: navmesh create/bake, agent setup, destination, behavior-tree authoring, behavior assignment, blackboard values
- `tool.get_engine_state` now includes `mount` and `ai` sections
- `gen.plan_from_prompt` now includes mount steps for horse prompts and AI steps for npc/navmesh/behavior prompts

## UI + Audio Workflow (S16/S17)

- `ui.*`: canvas creation, panel/text/button authoring, UI-to-component bindings, built-in HUD templates
- `audio.*`: clip import, source creation, playback, spatial params, mixer buses, source routing
- `tool.get_engine_state` now includes `ui` and `audio` sections
- `gen.plan_from_prompt` now includes UI/HUD and audio/sound planning branches

## Networking + Build + Debug Workflow (S18/S19/S20)

- `net.*`: server/client setup, entity replication map, prediction mode, rollback params
- `build.*`: target/bundle/version/feature configuration plus export and installer manifest generation
- `debug.*`: collider/navmesh/wireframe toggles, frame capture, profiler snapshots, hotspot summaries
- `tool.get_engine_state` now includes `networking`, `build`, and `debug` sections
- `gen.plan_from_prompt` now includes networking, build/export, and debug/profiler planning branches

## Macro Generator Workflow (S21)

- Template macro-tools: `gen.create_game_from_template`, `gen.create_platformer_level`, `gen.create_shooter_arena`, `gen.create_island_adventure`
- Packaging macro-tool: `gen.package_demo_build` (build config + export/installer orchestration)
- Macro-tools execute through `gen.execute_plan` semantics and keep transaction-aware behavior
- Registry contract and integration tests now validate tool coverage through phase 21

## AI Context Loop Workflow (S22)

- New context tools: `tool.get_cycle_context`, `tool.get_rules`, `tool.get_project_memory`, `tool.set_project_memory`, `tool.get_constraints`, `tool.set_constraints`, `tool.set_objective`
- Diagnostic loop tools: `tool.get_diagnostics`, `tool.clear_diagnostics` and warning/error capture via `tool.log` and failed tool calls
- `tool.get_engine_state` now includes `project_memory`, `constraints`, and diagnostics summary for cycle feedback

## MVP Quickstart Completion (S23)

- Added `asset.assign_material` for slot-aware material binding on entities
- Added `render.assign_material` alias for render-driven workflows
- Contract/tests/docs now validate tool coverage through phase 23

## Entity Lifecycle Completion (S24)

- Added lifecycle/hierarchy/query tools: `entity.clone`, `entity.delete`, `entity.rename`, `entity.parent`, `entity.unparent`, `entity.find_by_name`, `entity.find_by_tag`
- Entity rename now remaps runtime references (selection/components/physics/gameplay/UI/audio/networking keys)
- Entity delete now supports undo-safe restoration via command snapshots

## Entity Transform + Component Completion (S25)

- Added transform mutation tools: `entity.translate`, `entity.rotate`, `entity.scale`
- Added component tools: `entity.remove_component`, `entity.get_component`, `entity.set_component`
- `entity.get_transform` now returns translation + rotation + scale (rotation/scale from dynamic transform components)
- Contract/tests/docs now validate tool coverage through phase 25

## Asset Pipeline Creation Completion (S26)

- Added creation/import tools: `asset.import_url`, `asset.create_texture`, `asset.create_shader`, `asset.create_prefab`, `asset.save_prefab`
- Added `assets` section in `tool.get_engine_state` with imported/material/texture/shader/prefab registries
- Asset mutations stay command-bus based with `undo` support for runtime state and generated descriptor files

## Asset Pipeline Process Completion (S27)

- Added process/bake tools: `asset.rebuild_import`, `asset.generate_lods`, `asset.mesh_optimize`, `asset.compress_textures`, `asset.bake_lightmaps`, `asset.bake_reflection_probes`
- Added `assets.pipeline` runtime state to track rebuild/LOD/optimization/compression/bake history
- Contract/tests/docs now validate tool coverage through phase 27

## AI Hybrid Runtime (PR #5)

- `OFF` mode: AI runtime not initialized
- `API` mode: API runtime enabled with env-driven provider/key
- `LOCAL` mode: local MLL launched in separate process (`llama.cpp` compatible)
- Tool-calling audit logs saved to `logs/ai_tool_calls/YYYY-MM-DD.log`
- World Builder agent generates scene JSON from prompt

## Command Bus + Tool Registry (Baseline)

- AI tool execution now routes through a command bus with:
  - `validate/execute/undo`
  - transaction flow (`begin/commit/rollback/checkpoint`)
  - history flow (`undo/redo/mark/jump_to`)
- MVP tools include:
  - `tool.*` context/transactions/tasks/logging
  - `scene.*` create/open/save/save_as/duplicate/close + sky/time/fog/streaming
  - `entity.*` lifecycle/hierarchy/search + transform/component operations
  - `asset.*` import/url/create/material/texture/shader/prefab + process/optimize/bake + template bundle tools
  - `render.*` directional light + IBL + postprocess + LOD controls
  - `phys.*` collider/rigidbody/gravity/impulse/raycast + overlap/controller controls
  - `game.*` weapon/attach/fire/damage/health + input/interactions baseline
  - `anim.*` animator/controller/ik/retarget/bake baseline
  - `model.*` primitive/edit/topology/modifier/uv/sculpt baseline
  - `vfx.*` particles + VFX graph baseline
  - `water.*` ocean/river/waterfall + buoyancy/drag baseline
  - `mount.*` horse mounts baseline
  - `ai.*` npc ai/navmesh/behavior baseline
  - `ui.*` canvas/hud baseline
  - `audio.*` clip/source/mixer baseline
  - `net.*` multiplayer/networking baseline
  - `build.*` export/build orchestration baseline
  - `debug.*` debug/profiler baseline
  - `graph.*` low-code graph authoring/execution tools
  - `template.*` list/apply one-click gameplay templates
  - `gen.*` macro generation, prompt planning, plan execution, and demo packaging

## AI Controls (Editor)

- `F1`: switch AI mode to `OFF`
- `F2`: switch AI mode to `API`
- `F3`: switch AI mode to `LOCAL`
- `F4`: apply low-code template `Shooter Arena`
- `F5`: apply low-code template `Medieval Island`
- `F6`: run World Builder prompt and save `samples/generated_scene.json`
- `F7`: run `gen.plan_from_prompt` + `gen.execute_plan` (prompt from `GEN_PLAN_PROMPT`)
- `F8`: apply cinematic preset `Natural Day`
- `F9`: apply cinematic preset `Filmic Sunset`
- `F10`: apply cinematic preset `Noir Indoor`
- `F11`: toggle profiler panel (title/log metrics)
- `F12`: apply low-code template `Platform Runner`
- `G`: run graph event tick (`OnUpdate`)
- `V`: validate active low-code graph

## AI Setup

```bash
copy .env.example .env
```

- Fill local secrets only in `.env` (never commit keys/tokens)
- For local MLL mode set at least `LOCAL_MLL_BIN` and `LOCAL_MLL_MODEL`
- Example local command target: `llama-server` from `llama.cpp`

CLI world builder:

```bash
cargo run -p tools --bin world_builder -- "create a medieval island map"
```

The world builder now runs in `OFF`/`API`/`LOCAL` modes using deterministic local generation.

CLI tool-calling:

```bash
cargo run -p tools --bin tool_call -- tool.get_engine_state
cargo run -p tools --bin tool_call -- scene.create "{\"name\":\"Sandbox\"}"
```

PowerShell JSON example:

```powershell
cargo run -p tools --bin tool_call -- scene.create '{"name":"Sandbox"}'
```

## Workspace layout

- `ai/`
- `engine_core/`
- `render/`
- `ecs/`
- `editor/`
- `assets/`
- `tools/`
- `samples/`
- `docs/`

See also:
- `docs/AI_SETUP.md`
- `docs/SPRINT_5_BACKLOG.md`
- `docs/SPRINT_6_7_BACKLOG.md`
- `docs/SPRINT_8_9_BACKLOG.md`
- `docs/SPRINT_10_11_BACKLOG.md`
- `docs/SPRINT_12_13_BACKLOG.md`
- `docs/SPRINT_14_15_BACKLOG.md`
- `docs/SPRINT_16_17_BACKLOG.md`
- `docs/SPRINT_18_19_20_BACKLOG.md`
- `docs/SPRINT_21_BACKLOG.md`
- `docs/SPRINT_22_23_BACKLOG.md`
- `docs/SPRINT_24_25_BACKLOG.md`
- `docs/SPRINT_26_27_BACKLOG.md`
