# Sprint 12-13 (Weeks 45-52) - VFX + Water Systems

KPI target: tool-calling can author particle systems, compile a baseline VFX graph, and configure ocean/river/waterfall setups with buoyancy/drag helpers and undo/redo.

## Sprint 12 - VFX / Particles (`vfx.*`)

- [x] S12-VFX-01 VFX runtime state model
  - Owner: ai/render
  - Done: `CommandContext` now includes canonical VFX state (`particle_systems`, `graphs`) with deterministic serialization.
  - Progress: new records for emitters/forces/collision/renderer and graph nodes/edges/compile status are exposed via `tool.get_engine_state`.
  - Risks: graph compile is metadata-only in this phase.
  - Mitigation: keep compile outputs stable so runtime backend can be wired later without changing tool contracts.

- [x] S12-VFX-02 Particle authoring tool surface
  - Owner: ai
  - Done: added `vfx.create_particle_system`, `vfx.set_emitter`, `vfx.set_forces`, `vfx.set_collision`, `vfx.set_renderer`, `vfx.attach_to_entity`.
  - Progress: all operations go through command-bus commands with `validate/execute/undo`.
  - Risks: emitter/force params are schema-light and can vary by prompt.
  - Mitigation: permissive JSON params now; tighten field schemas as editor UX hardens.

- [x] S12-VFX-03 VFX graph baseline
  - Owner: ai
  - Done: added `vfx.create_graph`, `vfx.add_node`, `vfx.connect`, `vfx.compile_graph`.
  - Progress: graph topology and compile artifacts are tracked in runtime state for inspection and planning.
  - Risks: no GPU graph execution yet.
  - Mitigation: deterministic graph metadata allows staged backend implementation.

## Sprint 13 - Water / Ocean (`water.*`)

- [x] S13-WATER-01 Water runtime state model
  - Owner: ai/render/physics
  - Done: runtime state now tracks `oceans`, `rivers`, `waterfalls`, `buoyancy`, and `drag`.
  - Progress: reset/rebuild flows retain only valid entity-linked buoyancy/drag records.
  - Risks: no fluid simulation solver yet.
  - Mitigation: maintain stable tool contract over a lightweight deterministic model.

- [x] S13-WATER-02 Water authoring tool surface
  - Owner: ai
  - Done: added `water.create_ocean`, `water.create_river`, `water.create_waterfall`, `water.set_waves`, `water.enable_foam`, `water.enable_refraction`, `water.enable_caustics`.
  - Progress: water features are mutable with undo support and visible in `tool.get_engine_state`.
  - Risks: visuals are config-level only at this phase.
  - Mitigation: preserve field naming and ids for direct render backend wiring.

- [x] S13-WATER-03 Boat helpers + water sampling
  - Owner: ai/physics
  - Done: added `water.add_buoyancy`, `water.add_drag`, and read-only `water.sample_height`.
  - Progress: helper records are linked to entities; `sample_height` provides deterministic sinusoidal height from configured wave params.
  - Risks: sampled height is simplified versus production ocean spectra.
  - Mitigation: function is intentionally predictable for tests and plan generation.

- [x] S13-WATER-04 Planner + regression coverage
  - Owner: ai
  - Done: `gen.plan_from_prompt` now emits `water.*` steps for water/ocean/boat prompts and `vfx.*` steps for particle/smoke prompts.
  - Progress: integration and planner tests validate S12/S13 tool presence and state mutations.
  - Risks: prompt heuristics may miss edge vocabulary.
  - Mitigation: keep adding prompt fixtures as regressions.
