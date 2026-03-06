# Sprint 18-20 (Weeks 69-80) - Networking + Build/Export + Debug/Profiling

KPI target: tool-calling covers multiplayer baseline, packaging flow, and profiler/debug loops with command-bus undo/redo where applicable.

## Sprint 18 - Networking Baseline (`net.*`)

- [x] S18-NET-01 Networking runtime state model
  - Owner: ai/net
  - Done: runtime now tracks server config, connected clients, replication map, prediction mode, and rollback params.
  - Progress: `tool.get_engine_state` now exposes a dedicated `networking` section with counts and records.
  - Risks: no transport/runtime net loop in this phase.
  - Mitigation: preserve deterministic metadata contract for later engine integration.

- [x] S18-NET-02 Networking tool surface
  - Owner: ai
  - Done: added `net.create_server`, `net.connect_client`, `net.enable_replication`, `net.set_prediction`, `net.set_rollback`.
  - Progress: all networking mutations execute via command bus and support undo via state snapshot restore.
  - Risks: replication schema is currently component-name based (string contract).
  - Mitigation: keep stable schema now, bind to real component registries in next pass.

- [x] S18-NET-03 Planner + tests coverage
  - Owner: ai
  - Done: `gen.plan_from_prompt` now emits `net.*` steps for multiplayer/network prompts.
  - Progress: added planner + integration regression tests for networking state transitions.
  - Risks: prompt intent overlap with AI/gameplay branches.
  - Mitigation: keep explicit keyword fixtures and expand regression set.

## Sprint 19 - Build/Export Baseline (`build.*`)

- [x] S19-BUILD-01 Build runtime state model
  - Owner: ai/build
  - Done: runtime now tracks target, bundle id, version, enabled features, and last export/installer paths.
  - Progress: build state is surfaced in `tool.get_engine_state`.
  - Risks: real platform toolchains are still external to this layer.
  - Mitigation: keep command-bus manifest pipeline deterministic and auditable.

- [x] S19-BUILD-02 Build tool surface
  - Owner: ai
  - Done: added `build.set_target`, `build.set_bundle_id`, `build.set_version`, `build.enable_feature`, `build.export_project`, `build.generate_installer`.
  - Progress: export/installer operations write manifests and support undo via file backup/restore.
  - Risks: generated manifests are baseline metadata only.
  - Mitigation: evolve manifests as real packager integration lands.

- [x] S19-BUILD-03 Planner + transaction safety update
  - Owner: ai
  - Done: planner now emits build/export steps for build/package prompts.
  - Progress: `gen.execute_plan` now blocks only `build.build_and_run` in auto-transaction mode (non-rollback-safe path).
  - Risks: `build.build_and_run` remains side-effectful by design.
  - Mitigation: keep explicit guard and user override (`auto_transaction=false`).

## Sprint 20 - Debug/Profiling Baseline (`debug.*`)

- [x] S20-DEBUG-01 Debug runtime state model
  - Owner: ai/debug
  - Done: runtime now tracks collider/navmesh/wireframe toggles, captured frames counter, and profiler snapshots.
  - Progress: `tool.get_engine_state` now includes dedicated `debug` state and latest snapshot summary.
  - Risks: snapshots are lightweight synthetic profiler samples for now.
  - Mitigation: maintain stable schema while wiring real GPU/CPU captures later.

- [x] S20-DEBUG-02 Debug tool surface
  - Owner: ai
  - Done: added `debug.show_colliders`, `debug.show_navmesh`, `debug.toggle_wireframe`, `debug.capture_frame`, `debug.get_profiler_snapshot`, `debug.find_performance_hotspots`.
  - Progress: write tools route through command bus; read tools summarize recent snapshots and hotspot frequencies.
  - Risks: hotspot analysis is heuristic.
  - Mitigation: keep deterministic output and enrich with engine counters in next iterations.

- [x] S20-DEBUG-03 Catalog/integration regression updates
  - Owner: ai
  - Done: registry contract test extended through phase 20 and integration test validates phase18-20 state evolution end-to-end.
  - Progress: planner tests now cover network/build/debug branches.
  - Risks: future tool additions can drift from docs/tests.
  - Mitigation: keep contract test list and backlog doc updated per phase.
