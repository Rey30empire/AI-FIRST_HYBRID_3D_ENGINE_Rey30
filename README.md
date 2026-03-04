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
- Metallic/Roughness PBR baseline shading (single directional light)

## HDR + Tone Mapping (PR #4)

- PBR pass renders to HDR offscreen target (`RGBA16F`)
- Fullscreen tone-mapping pass resolves HDR to swapchain output
- Exposure/gamma controls are centralized in a dedicated tone-map uniform

## Directional Shadows (PR #6)

- Dedicated shadow-map pass before PBR lighting
- Depth shadow texture (`Depth32Float`) sampled with comparison sampler
- Basic 4-tap PCF filtering and bias controls in scene uniform

## AI Hybrid Runtime (PR #5)

- `OFF` mode: AI runtime not initialized
- `API` mode: API runtime enabled with env-driven provider/key
- `LOCAL` mode: local MLL launched in separate process (`llama.cpp` compatible)
- Tool-calling audit logs saved to `logs/ai_tool_calls/YYYY-MM-DD.log`
- World Builder agent generates scene JSON from prompt

## AI Controls (Editor)

- `F1`: switch AI mode to `OFF`
- `F2`: switch AI mode to `API`
- `F3`: switch AI mode to `LOCAL`
- `F6`: run World Builder prompt and save `samples/generated_scene.json`

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
