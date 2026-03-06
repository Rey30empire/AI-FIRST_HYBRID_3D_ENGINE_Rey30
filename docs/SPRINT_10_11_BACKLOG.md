# Sprint 10-11 (Weeks 37-44) - Animation + Modeling Workflow

KPI target: tool-calling can author a basic animation controller and perform core modeling edits (topology/modifiers/uv/sculpt) with command-bus undo/redo.

## Sprint 10 - Animation & Rigging (`anim.*`)

- [x] S10-ANIM-01 Animator/controller state model
  - Owner: ai/gameplay
  - Done: runtime state now includes state machines, animator bindings, active clips, blends, IK solvers, retarget jobs, and bake jobs.
  - Progress: added `AnimationRuntimeState` and related records, exposed in `tool.get_engine_state`.
  - Risks: no real skeleton playback backend yet.
  - Mitigation: keep command contracts deterministic and backend-agnostic.

- [x] S10-ANIM-02 Tool surface for controller authoring
  - Owner: ai
  - Done: `anim.create_state_machine`, `anim.add_state`, `anim.add_transition`, `anim.set_parameter`, `anim.add_animator`.
  - Progress: each operation routes through command bus and supports undo via full state restore.
  - Risks: large state snapshots on heavy scenes.
  - Mitigation: optimize delta storage in future phases.

- [x] S10-ANIM-03 Playback/IK/retarget/bake baseline
  - Owner: ai
  - Done: `anim.play`, `anim.blend`, `anim.add_ik`, `anim.retarget`, `anim.bake_animation`.
  - Progress: playback and blend metadata sync to dynamic components; retarget/bake are queued as tracked jobs.
  - Risks: queued jobs are metadata-only currently.
  - Mitigation: wire to runtime workers in next animation phase.

## Sprint 11 - Modeling Tools (`model.*`)

- [x] S11-MODEL-01 Primitive + edit session tools
  - Owner: ai/editor
  - Done: `model.create_primitive`, `model.enter_edit_mode`, `model.exit_edit_mode`, `model.select`.
  - Progress: primitive creation spawns scene entity + mesh record; edit/selection state tracked in modeling runtime.
  - Risks: no visual edit gizmos yet.
  - Mitigation: keep command APIs stable for editor UI integration.

- [x] S11-MODEL-02 Topology/modifier/uv/sculpt operations
  - Owner: ai/editor
  - Done: topology ops (`extrude/inset/bevel/loop_cut/knife/merge/subdivide/triangulate/voxel_remesh/decimate/smooth`), modifier stack, UV tools, sculpt brush/mask.
  - Progress: operations update modeling state and append operation log for replay/debug.
  - Risks: geometry ops are heuristic metadata edits, not full mesh kernel.
  - Mitigation: swap internals for real mesh kernel while preserving tool contract.

- [x] S11-MODEL-03 Planner and regression coverage
  - Owner: ai
  - Done: prompt planner now emits animation/modeling plans for horse/modeling prompts; tests cover phase 10/11 end-to-end state updates.
  - Progress: added new `gen.plan_from_prompt` branches and integration tests in `tool_registry`.
  - Risks: plan heuristics can drift with new prompt classes.
  - Mitigation: add prompt fixtures as regression cases.
