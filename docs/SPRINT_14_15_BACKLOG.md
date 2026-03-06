# Sprint 14-15 (Weeks 53-60) - Mounts + NPC AI

KPI target: tool-calling can set up horse mounts and baseline NPC AI navigation/behavior trees with full command-bus undo/redo.

## Sprint 14 - Mount System (`mount.*`)

- [x] S14-MOUNT-01 Mount runtime state model
  - Owner: ai/gameplay
  - Done: runtime now tracks horse templates, spawned horses, and rider-to-horse bindings.
  - Progress: `tool.get_engine_state` exposes `mount` counts and detailed records.
  - Risks: no advanced horse locomotion solver yet.
  - Mitigation: keep deterministic command contract and stable ids.

- [x] S14-MOUNT-02 Mount tool surface
  - Owner: ai
  - Done: added `mount.create_horse_template`, `mount.spawn_horse`, `mount.mount_rider`, `mount.dismount`, `mount.set_gait`, `mount.set_path_follow`.
  - Progress: all operations execute through command bus and support undo snapshots.
  - Risks: spawned horse mesh/rig is metadata-level in this phase.
  - Mitigation: preserve payload schema for later runtime backend wiring.

- [x] S14-MOUNT-03 Planner integration
  - Owner: ai
  - Done: horse prompts now include mount setup steps in `gen.plan_from_prompt`.
  - Progress: planner keeps animation flow and augments with mount template/spawn/mount/gait.
  - Risks: prompt heuristics may miss some horse synonyms.
  - Mitigation: keep adding fixtures in regression tests.

## Sprint 15 - NPC AI Baseline (`ai.*`)

- [x] S15-AI-01 NPC AI runtime state model
  - Owner: ai
  - Done: runtime now tracks navmeshes, agents, entity->agent map, behavior trees, and blackboard entries.
  - Progress: state is serialized and visible in `tool.get_engine_state` under `ai`.
  - Risks: no pathfinding worker execution loop yet.
  - Mitigation: deterministic metadata baseline with stable graph contracts.

- [x] S15-AI-02 Navigation + behavior-tree tools
  - Owner: ai
  - Done: added `ai.create_navmesh`, `ai.bake_navmesh`, `ai.add_agent`, `ai.set_destination`, `ai.create_behavior_tree`, `ai.bt_add_node`, `ai.bt_connect`, `ai.assign_behavior`, `ai.set_blackboard`.
  - Progress: operations validate dependencies and mutate canonical AI state through command bus.
  - Risks: BT execution is not running in runtime tick yet.
  - Mitigation: command payloads and node/edge data are backend-ready.

- [x] S15-AI-03 Planner + regression coverage
  - Owner: ai
  - Done: planner now emits AI steps for NPC/navmesh/behavior prompts; tests validate S14/S15 catalog + state mutation.
  - Progress: new integration and planner tests pass in workspace suite.
  - Risks: vocabulary drift in free-form prompts.
  - Mitigation: keep prompt fixtures aligned with user language.
