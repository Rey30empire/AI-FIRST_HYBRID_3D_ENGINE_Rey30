# Sprint 8-9 (Weeks 29-36) - Physics Controls + Gameplay Framework

KPI target: tool-calling can configure a controllable character loop (movement/jump/overlap) plus input/interactions (actions, trigger, pickup, inventory, interactable) with undo/redo-safe commands.

## Sprint 8 - Physics Controls (`phys.*`)

- [x] S8-PHYS-01 Collider lifecycle + rigidbody tuning tools
  - Owner: ai/ecs
  - Done: added `phys.set_collider`, `phys.remove_collider`, `phys.set_mass`, `phys.set_friction`, `phys.set_restitution`.
  - Progress: collider and rigidbody properties are mutable through command bus with full `validate/execute/undo`.
  - Risks: simplified per-entity state does not represent full solver constraints.
  - Mitigation: preserve tool contracts while backend solver evolves.

- [x] S8-PHYS-02 Force + overlap query tools
  - Owner: ai/ecs
  - Done: added `phys.apply_force` and `phys.overlap`.
  - Progress: force updates velocity/position deterministically; overlap supports `box` and `sphere` against registered colliders.
  - Risks: overlap uses broad-phase style approximations.
  - Mitigation: keep result semantics explicit and deterministic.

- [x] S8-PHYS-03 Character controller tools
  - Owner: ai/ecs
  - Done: added `phys.add_character_controller`, `phys.character_move`, `phys.character_jump`, `phys.character_set_state`.
  - Progress: runtime tracks controller state (`speed`, `jump_strength`, `grounded`, `state`) in physics state and entity components.
  - Risks: movement model is intentionally lightweight.
  - Mitigation: additive compatibility path for future kinematic controller backend.

## Sprint 9 - Gameplay Framework (`game.*`)

- [x] S9-GAME-01 Input action mapping tools
  - Owner: ai/gameplay
  - Done: added `game.create_input_action`, `game.bind_action`, `game.set_rebind`.
  - Progress: gameplay runtime now stores input actions, bindings, and target events for script linkage.
  - Risks: no per-device deadzone/sensitivity tuning yet.
  - Mitigation: keep schema extensible for provider-specific options.

- [x] S9-GAME-02 Interaction tools
  - Owner: ai/gameplay
  - Done: added `game.add_trigger`, `game.add_pickup`, `game.add_inventory`, `game.add_interactable`.
  - Progress: interaction metadata is persisted in gameplay runtime maps and mirrored to entity dynamic components.
  - Risks: interactions are metadata-only in this phase.
  - Mitigation: bind these records to runtime systems in next gameplay phase.

- [x] S9-GAME-03 Planner + state snapshot integration
  - Owner: ai
  - Done: shooter planning path now includes input + character-controller setup.
  - Progress: `tool.get_engine_state` exposes counts/details for controllers, input actions, triggers, pickups, inventories, and interactables.
  - Risks: larger state payload size.
  - Mitigation: maintain clear sectioned snapshot format and only include canonical fields.
