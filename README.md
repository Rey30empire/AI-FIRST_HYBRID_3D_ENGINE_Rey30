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

## Workspace layout

- `engine_core/`
- `render/`
- `ecs/`
- `editor/`
- `assets/`
- `tools/`
- `samples/`
- `docs/`
