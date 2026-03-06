# Sprint 26-27 (Weeks 101-108) - Asset Pipeline Create + Process Completion

KPI target: `asset.*` reaches create/import/process baseline from `nuevas_Ideas.txt` section 6.

## Sprint 26 - Asset Import/Create (`asset.*`)

- [x] S26-ASSET-01 URL + descriptor creation tools
  - Owner: ai/assets
  - Done: added `asset.import_url`, `asset.create_texture`, `asset.create_shader`.
  - Progress: generated descriptors are written under `assets/textures` and `assets/shaders`.
  - Risks: `asset.import_url` depends on external network availability.
  - Mitigation: keep `asset.import_file` as deterministic local fallback and preserve undo snapshots.

- [x] S26-ASSET-02 Prefab authoring tools
  - Owner: ai/assets
  - Done: added `asset.create_prefab` and `asset.save_prefab`.
  - Progress: prefab snapshots capture source entity + dynamic components and are persisted under `assets/prefabs`.
  - Risks: prefab schema is lightweight and may need extension for advanced overrides.
  - Mitigation: keep metadata field open and backward-compatible.

- [x] S26-ASSET-03 Runtime visibility
  - Owner: ai
  - Done: added `assets` section in `tool.get_engine_state` for imported/material/texture/shader/prefab registries.
  - Progress: MLL can inspect current asset registry without scanning disk.
  - Risks: state payload size can grow in large projects.
  - Mitigation: use cycle-context limits and keep heavy operations off critical loops.

## Sprint 27 - Asset Process/Optimize/Bake (`asset.*`)

- [x] S27-ASSET-01 Rebuild + optimization tools
  - Owner: ai/assets
  - Done: added `asset.rebuild_import`, `asset.generate_lods`, `asset.mesh_optimize`, `asset.compress_textures`.
  - Progress: operations are tracked in runtime pipeline state with timestamps and parameters.
  - Risks: these are metadata/runtime passes (not full DCC-grade processing yet).
  - Mitigation: preserve deterministic contract so engine-specific processors can replace internals later.

- [x] S27-ASSET-02 Bake tools
  - Owner: ai/assets
  - Done: added `asset.bake_lightmaps`, `asset.bake_reflection_probes`.
  - Progress: bake jobs are tracked with bounded history buffers.
  - Risks: current implementation tracks jobs, not full offline bake output assets.
  - Mitigation: integrate baker backend in future while preserving tool API.

- [x] S27-ASSET-03 Contract/tests/docs closure
  - Owner: ai/docs
  - Done: registry contract now validates through phase 27 and integration tests added for S26/S27 flows.
  - Progress: README/setup/map docs updated with phase 26/27 coverage.
  - Risks: future additions can desync docs/contracts.
  - Mitigation: keep sprint backlog and registry required-tool list updated each phase.
