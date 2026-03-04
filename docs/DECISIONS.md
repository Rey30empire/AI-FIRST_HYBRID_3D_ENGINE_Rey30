# Decisions (Frozen for PR #1)

Date: 2026-03-04

## 1) Core language

Decision: Rust

Why:
- Memory safety by default for a long-lived engine codebase
- Strong performance with low-level control
- Good async/tooling ecosystem for future AI orchestration services

## 2) Rendering API layer

Decision: `wgpu`

Why:
- Single Rust API mapped to Vulkan/DX12/Metal
- Faster iteration than managing a multi-language render stack on day 1
- Keeps path open to later native backend specialization if needed

## 3) ECS

Decision: `bevy_ecs`

Why:
- Mature scheduler and query model
- High adoption and maintained ecosystem
- Easy parallel system execution roadmap
- Portable as standalone ECS without pulling full Bevy engine runtime

Rejected for now:
- `hecs`: simpler but fewer built-in scheduling features
- `legion`: solid architecture but lower ecosystem momentum today

## 4) Editor UI

Decision: `egui` (initial)

Why:
- Fastest path to working editor tooling and docking-like panels
- Native Rust stack, no web bridge at MVP stage
- Allows focus on core/render performance first

Migration plan:
- Evaluate `tauri + react` once core/render/ECS stabilize and panel UX requirements exceed `egui` limits.

## 5) Asset ingestion order

Decision:
- Phase A: glTF first
- Phase B: FBX later (via conversion/import pipeline)

Why:
- glTF is open, modern, and tooling-friendly for PBR pipelines
- FBX support often requires extra complexity/licensing/toolchain glue

## 6) AI modes policy baseline

Decision:
- OFF: AI subsystem not loaded
- API: remote provider calls via gateway
- LOCAL: separate process runtime

Why:
- Keeps manual mode lightweight
- Prevents UI stalls and simplifies fault isolation in LOCAL mode