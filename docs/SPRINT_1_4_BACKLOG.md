# Sprint Backlog (Sprint 1-4)

## Sprint 1 (Weeks 1-3) - Runtime Foundations

KPI target: 60 FPS stable at 1080p in demo scene.

- [x] S1-ENG-01 Window + game loop
  - Owner: engine
  - Done: App opens window, runs continuous update/render loop, closes cleanly.
  - Risks: Event loop API churn across winit versions.
  - Mitigation: Pin crate versions in workspace and upgrade in controlled branch.

- [x] S1-REND-01 Basic mesh render (triangle)
  - Owner: render
  - Done: GPU pipeline created, visible geometry rendered every frame.
  - Risks: Backend incompatibility on some drivers.
  - Mitigation: Keep default backend and add adapter diagnostics logs.

- [x] S1-ENG-02 Basic profiler (FPS/frame time)
  - Owner: engine
  - Done: Frame time + FPS computed and shown in window title.
  - Risks: Noisy readings on startup.
  - Mitigation: Rolling 1s FPS window and periodic title updates.

- [x] S1-ASSET-01 Scene load simple
  - Owner: engine
  - Done: Scene JSON loads at startup with fallback to empty scene.
  - Risks: Invalid JSON breaks startup.
  - Mitigation: Parse errors logged, fallback scene created.

- [x] S1-ENG-03 Camera orbit + WASD
  - Owner: engine
  - Done: Orbit mouse drag + wheel zoom + WASD movement active in scene view.
  - Risks: Input mapping conflicts and jitter.
  - Mitigation: Central input state + smoothing + configurable sensitivity.

- [x] S1-SAMPLE-01 Demo scene
  - Owner: render
  - Done: `samples/demo_scene.json` exists and is loaded.
  - Risks: Scene format drift.
  - Mitigation: Version scene schema and validate in loader.

## Sprint 2 (Weeks 4-8) - Vertical Slice WOW

KPI target: cinematic interior+exterior demo with stable frame time budget.

- [x] S2-REND-01 PBR pipeline
  - Owner: render
  - Done: Metallic/Roughness materials render correctly with IBL baseline.
  - Progress: PBR shader now includes hemispheric IBL baseline (sky/ground colors + intensity) exposed through `render.set_ibl`.
  - Risks: Shader complexity and material mismatches.
  - Mitigation: Start with single reference material and golden renders.

- [x] S2-REND-02 HDR + tone mapping
  - Owner: render
  - Done: HDR render target and tone mapper integrated.
  - Progress: PR #4 implements 2-pass render (`HDR -> tone map -> swapchain`) with exposure/gamma uniform.
  - Risks: Over/under exposure across scenes.
  - Mitigation: Auto-exposure clamp + histogram debug view.

- [x] S2-REND-03 Shadow maps
  - Owner: render
  - Done: Directional light shadows with configurable cascade count.
  - Progress: Cascaded directional shadows implemented (up to 3 cascades) with PCF sampling and runtime control of cascade count, bias and strength.
  - Risks: Acne/peter-panning artifacts.
  - Mitigation: Depth bias tuning presets + PCF filtering.

- [x] S2-REND-04 Bloom + fog + color grading presets
  - Owner: render
  - Done: Post stack includes bloom, light fog, LUT/preset grading.
  - Progress: Tone-map pass now applies bloom threshold/radius/intensity, fog overlay, white balance, tint, saturation and contrast controls.
  - Risks: Post stack blows frame budget.
  - Mitigation: Quality tiers and per-effect toggles in editor.

- [x] S2-EDITOR-01 Cinematic preset system
  - Owner: editor
  - Done: One-click presets (Natural Day, Filmic Sunset, Noir Indoor).
  - Progress: `F8` Natural Day, `F9` Filmic Sunset, `F10` Noir Indoor; each preset updates light, postprocess grading, sky/time-of-day and fog tools.
  - Risks: Presets look inconsistent across displays.
  - Mitigation: Standardized reference captures and calibration notes.

## Sprint 3 (Weeks 9-12) - Performance & Scale

KPI target: large scene with no major spikes and predictable frame pacing.

- [x] S3-REND-01 GPU instancing + frustum culling
  - Owner: render
  - Done: Repeated meshes rendered via instancing with camera culling.
  - Progress: Renderer now consumes scene instance list, performs CPU frustum-sphere culling per frame, uploads visible instances to GPU instance buffer, and renders with instanced draw calls for shadow and PBR passes.
  - Risks: CPU overhead in cull stage.
  - Mitigation: SIMD-friendly bounding volumes + job batching.

- [x] S3-REND-02 LOD system
  - Owner: render
  - Done: Automatic LOD switching with hysteresis.
  - Progress: Renderer now selects per-instance LOD buckets (LOD0/LOD1/LOD2) with distance+hysteresis, renders each bucket with dedicated mesh buffers, reports LOD counters in profiler stats, and exposes runtime tuning through `render.set_lod_settings`.
  - Risks: Visible popping.
  - Mitigation: Cross-fade and distance thresholds per asset class.

- [x] S3-ENG-01 Streaming-ready asset cache
  - Owner: engine
  - Done: Async load/unload path with bounded memory pool.
  - Progress: `assets::AsyncAssetCache` provides background IO queue, bounded capacity, LRU-style eviction, scene prefetch hooks, and live cache stats (pending/hits/misses/evictions) consumed by editor profiler.
  - Risks: Stutters from IO bursts.
  - Mitigation: Background IO queue + prefetch hints.

- [x] S3-EDITOR-01 In-editor profiler panel
  - Owner: editor
  - Done: CPU/GPU/memory/draw-call metrics panel.
  - Progress: `F11` toggles profiler overlay in title + logs with frame/cull CPU, draw calls, visible/culled instances, LOD buckets, GPU buffer estimate, and asset-cache memory/queue counters.
  - Risks: Instrumentation overhead.
  - Mitigation: Sampling mode and debug-only deep counters.

## Sprint 4 (Weeks 13-16) - Low-code MVP

KPI target: playable prototype assembled in less than 30 minutes without coding.

- [x] S4-EDITOR-01 Node graph MVP
  - Owner: editor
  - Done: Node canvas with create/connect/delete and validation.
  - Progress: Low-code graph workflow is exposed in-editor via hotkeys and tools (`graph.create/add_node/connect/delete_node/delete_edge/set_node_params/validate`), with live graph status in title/profiler and command-bus undo/redo support.
  - Risks: Graph UX friction.
  - Mitigation: Template graphs and inline node docs.

- [x] S4-ENG-01 Runtime graph executor
  - Owner: engine
  - Done: Event-driven node runtime executes deterministically per frame.
  - Progress: `ecs::execute_runtime_graph` validates DAG + builds deterministic order + executes event/flow/commit phases producing side effects (`SpawnEntity`, `MoveEntity`, `ApplyDamage`, `SetLightPreset`, `SetWeather`, UI objective/message).
  - Risks: Non-deterministic order bugs.
  - Mitigation: Topological sort + explicit update phases.

- [x] S4-EDITOR-02 1-click templates
  - Owner: editor
  - Done: Shooter/Island/Platform templates generate playable scene setup.
  - Progress: Built-in templates (`template_shooter_arena`, `template_medieval_island`, `template_platform_runner`) can be listed/applied through tools and hotkeys (`F4`, `F5`, `F12`) and auto-run `OnStart` graph.
  - Risks: Template entropy over time.
  - Mitigation: Template tests + versioned schema migration.

- [x] S4-ASSET-01 Template asset bundles
  - Owner: engine
  - Done: Each template resolves required meshes/materials/audio.
  - Progress: Asset bundles are declared per template in `assets`, surfaced by `asset.get_template_bundle`, and validated through `asset.validate_template_bundle` with persisted validation state in tool engine snapshot.
  - Risks: Missing dependencies at runtime.
  - Mitigation: Build-time bundle validation.
