# Sprint 24-25 (Weeks 93-100) - Entity Lifecycle + Transform/Component Contract Completion

KPI target: `entity.*` reaches full lifecycle + hierarchy + transform + component baseline from `nuevas_Ideas.txt` section 5.

## Sprint 24 - Entity Lifecycle/Hierarchy/Search (`entity.*`)

- [x] S24-ENTITY-01 Lifecycle operations
  - Owner: ai
  - Done: added `entity.clone`, `entity.delete`, `entity.rename`.
  - Progress: clone supports optional component/hierarchy copy and translation offset.
  - Risks: snapshot-based undo for delete/rename is memory-heavy on very large runtime states.
  - Mitigation: keep command scope bounded and migrate to diff-based snapshots in a future optimization pass.

- [x] S24-ENTITY-02 Hierarchy operations
  - Owner: ai
  - Done: added `entity.parent`, `entity.unparent` with cycle validation.
  - Progress: hierarchy persisted through `HierarchyParent` component and validated against cycles.
  - Risks: hierarchy is component-backed (not native scene JSON field yet).
  - Mitigation: preserve deterministic component contract so migration to native hierarchy remains compatible.

- [x] S24-ENTITY-03 Search operations
  - Owner: ai
  - Done: added `entity.find_by_name`, `entity.find_by_tag`.
  - Progress: supports name query/exact mode and Tag/Tags component lookup.
  - Risks: tag conventions can vary between teams (`Tag` vs `Tags` payload style).
  - Mitigation: support both forms and keep docs/examples aligned.

## Sprint 25 - Transform + Components Completion (`entity.*`)

- [x] S25-ENTITY-01 Transform mutation tools
  - Owner: ai
  - Done: added `entity.translate`, `entity.rotate`, `entity.scale`.
  - Progress: translation mutates scene entity; rotation/scale persist in dynamic transform components.
  - Risks: no quaternion/native transform stack yet (Euler-only baseline for tool-calling).
  - Mitigation: keep payload explicit and migration-friendly (`TransformRotation`/`TransformScale` components).

- [x] S25-ENTITY-02 Component IO parity
  - Owner: ai
  - Done: added `entity.remove_component`, `entity.get_component`, `entity.set_component`.
  - Progress: command bus now differentiates add vs set/remove semantics for traceability.
  - Risks: component schema remains intentionally open.
  - Mitigation: rely on per-tool domain validation where strict schemas are needed.

- [x] S25-ENTITY-03 Contract/tests/docs closure
  - Owner: ai/docs
  - Done: registry contract now validates through phase 25 and integration tests added for S24/S25 flows.
  - Progress: README/setup/map docs updated with phase24/phase25 coverage.
  - Risks: tool contract/docs can drift with future additions.
  - Mitigation: keep sprint backlog and registry-required list updated per phase.
