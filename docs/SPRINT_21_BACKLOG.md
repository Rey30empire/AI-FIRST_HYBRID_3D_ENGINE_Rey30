# Sprint 21 (Weeks 81-84) - Macro Generator Tools

KPI target: high-level generator tools can create complete playable baselines and package-ready outputs through deterministic tool plans.

## S21 - Macro Tools (`gen.create_*` + `gen.package_demo_build`)

- [x] S21-GEN-01 Template/game macro entrypoints
  - Owner: ai
  - Done: added `gen.create_game_from_template`, `gen.create_platformer_level`, `gen.create_shooter_arena`, `gen.create_island_adventure`.
  - Progress: macro-tools execute multi-step plans through `gen.execute_plan` semantics and return structured execution payloads.
  - Risks: macro output quality depends on planner branch coverage.
  - Mitigation: keep prompt fixtures and tool-contract tests as guardrails.

- [x] S21-GEN-02 Demo packaging macro
  - Owner: ai/build
  - Done: added `gen.package_demo_build` with build target/version/bundle/features/export/installer orchestration.
  - Progress: macro supports optional `build.build_and_run` step while enforcing non-auto-transaction execution for side-effect safety.
  - Risks: real platform packagers remain external to this metadata pipeline.
  - Mitigation: preserve deterministic manifests and clear parameters for downstream tooling.

- [x] S21-GEN-03 Registry/tests/docs closure
  - Owner: ai/docs
  - Done: tool registry contract extended to phase 21 and new integration tests added for macro flows and package artifacts.
  - Progress: README/setup/implementation map updated with S21 coverage.
  - Risks: future macro additions can drift from docs.
  - Mitigation: keep backlog + registry contract list updated per sprint.
