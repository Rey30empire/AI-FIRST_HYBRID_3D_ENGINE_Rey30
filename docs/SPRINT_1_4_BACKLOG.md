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

- [ ] S2-REND-01 PBR pipeline
  - Owner: render
  - Done: Metallic/Roughness materials render correctly with IBL baseline.
  - Progress: PR #3 adds non-textured PBR baseline (metallic/roughness + directional light); IBL pending.
  - Risks: Shader complexity and material mismatches.
  - Mitigation: Start with single reference material and golden renders.

- [ ] S2-REND-02 HDR + tone mapping
  - Owner: render
  - Done: HDR render target and tone mapper integrated.
  - Risks: Over/under exposure across scenes.
  - Mitigation: Auto-exposure clamp + histogram debug view.

- [ ] S2-REND-03 Shadow maps
  - Owner: render
  - Done: Directional light shadows with configurable cascade count.
  - Risks: Acne/peter-panning artifacts.
  - Mitigation: Depth bias tuning presets + PCF filtering.

- [ ] S2-REND-04 Bloom + fog + color grading presets
  - Owner: render
  - Done: Post stack includes bloom, light fog, LUT/preset grading.
  - Risks: Post stack blows frame budget.
  - Mitigation: Quality tiers and per-effect toggles in editor.

- [ ] S2-EDITOR-01 Cinematic preset system
  - Owner: editor
  - Done: One-click presets (Natural Day, Filmic Sunset, Noir Indoor).
  - Risks: Presets look inconsistent across displays.
  - Mitigation: Standardized reference captures and calibration notes.

## Sprint 3 (Weeks 9-12) - Performance & Scale

KPI target: large scene with no major spikes and predictable frame pacing.

- [ ] S3-REND-01 GPU instancing + frustum culling
  - Owner: render
  - Done: Repeated meshes rendered via instancing with camera culling.
  - Risks: CPU overhead in cull stage.
  - Mitigation: SIMD-friendly bounding volumes + job batching.

- [ ] S3-REND-02 LOD system
  - Owner: render
  - Done: Automatic LOD switching with hysteresis.
  - Risks: Visible popping.
  - Mitigation: Cross-fade and distance thresholds per asset class.

- [ ] S3-ENG-01 Streaming-ready asset cache
  - Owner: engine
  - Done: Async load/unload path with bounded memory pool.
  - Risks: Stutters from IO bursts.
  - Mitigation: Background IO queue + prefetch hints.

- [ ] S3-EDITOR-01 In-editor profiler panel
  - Owner: editor
  - Done: CPU/GPU/memory/draw-call metrics panel.
  - Risks: Instrumentation overhead.
  - Mitigation: Sampling mode and debug-only deep counters.

## Sprint 4 (Weeks 13-16) - Low-code MVP

KPI target: playable prototype assembled in less than 30 minutes without coding.

- [ ] S4-EDITOR-01 Node graph MVP
  - Owner: editor
  - Done: Node canvas with create/connect/delete and validation.
  - Risks: Graph UX friction.
  - Mitigation: Template graphs and inline node docs.

- [ ] S4-ENG-01 Runtime graph executor
  - Owner: engine
  - Done: Event-driven node runtime executes deterministically per frame.
  - Risks: Non-deterministic order bugs.
  - Mitigation: Topological sort + explicit update phases.

- [ ] S4-EDITOR-02 1-click templates
  - Owner: editor
  - Done: Shooter/Island/Platform templates generate playable scene setup.
  - Risks: Template entropy over time.
  - Mitigation: Template tests + versioned schema migration.

- [ ] S4-ASSET-01 Template asset bundles
  - Owner: engine
  - Done: Each template resolves required meshes/materials/audio.
  - Risks: Missing dependencies at runtime.
  - Mitigation: Build-time bundle validation.
