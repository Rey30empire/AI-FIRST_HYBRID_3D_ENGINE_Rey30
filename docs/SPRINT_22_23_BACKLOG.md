# Sprint 22-23 (Weeks 85-92) - AI Context Loop + MVP Quickstart Completion

KPI target: the MLL receives a stable cycle context (memory/constraints/rules/diagnostics) and the recommended MVP material assignment path is fully available in tool-calling.

## Sprint 22 - Cycle Context Contract (`tool.*`)

- [x] S22-CONTEXT-01 Context snapshot tool surface
  - Owner: ai
  - Done: added `tool.get_cycle_context` with scene summary, selection, resources, objective, constraints, project memory, diagnostics and recent command feedback.
  - Progress: cycle payload is lightweight and parameterized (`max_entities`, `recent_commands`, `diagnostics_last_n`).
  - Risks: context can still grow if large scenes are requested without limits.
  - Mitigation: keep bounded defaults and explicit truncation flags.

- [x] S22-CONTEXT-02 Memory + constraints + objective state
  - Owner: ai
  - Done: added `tool.get_project_memory`, `tool.set_project_memory`, `tool.get_constraints`, `tool.set_constraints`, `tool.set_objective`.
  - Progress: merge/replace update modes supported for memory/constraints while objective writes directly into scene runtime.
  - Risks: schema is intentionally flexible and can drift per project.
  - Mitigation: keep canonical field suggestions in docs and smoke tests.

- [x] S22-CONTEXT-03 Rules + diagnostics feedback loop
  - Owner: ai
  - Done: added `tool.get_rules`, `tool.get_diagnostics`, `tool.clear_diagnostics`; diagnostics now capture warn/error logs and failed tool calls.
  - Progress: `tool.get_engine_state` includes `project_memory`, `constraints`, and diagnostics summary.
  - Risks: diagnostics are runtime-local (not persisted across process restart).
  - Mitigation: rely on existing audit logs for long-term trace persistence.

## Sprint 23 - MVP Quickstart Completion (`asset.*` / `render.*`)

- [x] S23-MVP-01 Material assignment tooling
  - Owner: ai/render
  - Done: added `asset.assign_material` and `render.assign_material` alias.
  - Progress: tools validate entity/material existence and apply `MaterialOverride` component with slot metadata.
  - Risks: no dedicated render-material binding subsystem yet (component-based baseline only).
  - Mitigation: preserve deterministic component contract for future renderer integration.

- [x] S23-MVP-02 Contract/tests/docs closure
  - Owner: ai/docs
  - Done: tool registry contract advanced through phase 23 and integration tests added for phase22 cycle tools and phase23 material assignment.
  - Progress: README/setup/map docs updated with S22/S23 coverage.
  - Risks: future tool additions can desync docs.
  - Mitigation: keep sprint backlog docs and contract list synchronized per phase.
