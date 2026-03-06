# Sprint 6-7 (Weeks 21-28) - Physics + Gameplay Baseline

KPI target: tool-calling can assemble and validate a minimal combat loop (collider/rigidbody + weapon/ammo + health/damage) with history/rollback.

## Sprint 6 - Physics Foundation (`phys.*`)

- [x] S6-PHYS-01 Collider + rigidbody command tools
  - Owner: ai/ecs
  - Done: `phys.add_collider` and `phys.add_rigidbody` create or update physics state linked to scene entities.
  - Progress: command bus now persists `PhysicsRuntimeState` (`gravity`, `colliders`, `rigidbodies`) and mirrors collider/rigidbody data into dynamic components.
  - Risks: simplified physics model can diverge from future full simulation backend.
  - Mitigation: keep command contracts stable and treat this as deterministic baseline layer.

- [x] S6-PHYS-02 Gravity, impulse, and ray queries
  - Owner: ai/ecs
  - Done: `phys.set_gravity`, `phys.apply_impulse`, and `phys.raycast` available through tool registry.
  - Progress: impulse updates rigidbody velocity and applies a small deterministic transform step; raycast performs lightweight hit testing over registered colliders.
  - Risks: raycast is approximation (AABB over collider extents), not full narrow-phase.
  - Mitigation: explicit lightweight semantics and extensible helper path for future backend replacement.

- [x] S6-PHYS-03 Runtime snapshot and regression tests
  - Owner: ai
  - Done: `tool.get_engine_state` exposes physics snapshot and tests cover happy-path behavior.
  - Progress: physics counts, gravity, colliders, and rigidbodies are returned in engine state and validated by integration-style tests in `tool_registry`.
  - Risks: schema drift as more physics fields are added.
  - Mitigation: maintain tests around tool payload contracts.

## Sprint 7 - Gameplay Combat Foundation (`game.*`)

- [x] S7-GAME-01 Weapon registry tools
  - Owner: ai/gameplay
  - Done: `game.create_weapon` stores baseline weapon definitions (`rate/recoil/spread/ammo`).
  - Progress: runtime `GameplayRuntimeState` now tracks weapon records plus ammo state.
  - Risks: schema may need extensions for reload/projectiles.
  - Mitigation: additive fields on existing command payload to preserve compatibility.

- [x] S7-GAME-02 Weapon attachment + firing loop
  - Owner: ai/gameplay
  - Done: `game.attach_weapon` and `game.fire_weapon` implemented with validation and undo.
  - Progress: fire events increase telemetry counters, consume ammo, and update scene runtime messaging.
  - Risks: no per-frame fire rate gating yet.
  - Mitigation: keep command deterministic now; enforce timing in future runtime systems.

- [x] S7-GAME-03 Health and damage baseline
  - Owner: ai/gameplay
  - Done: `game.add_health_component` and `game.apply_damage` mutate canonical health values and damage counters.
  - Progress: health is represented in dynamic components; total applied damage is tracked in gameplay runtime state.
  - Risks: missing armor/resistance layers.
  - Mitigation: extend command payload with optional modifiers in future phases.

- [x] S7-GAME-04 Planner integration for shooter prompts
  - Owner: ai
  - Done: `gen.plan_from_prompt` (shooter path) now emits physics and gameplay setup steps in addition to template/graph/render.
  - Progress: generated shooter plans include pawn creation, collider/rigidbody setup, health setup, weapon create/attach/fire steps.
  - Risks: generated names may vary per run.
  - Mitigation: dynamic names avoid collisions and preserve plan execution success.
