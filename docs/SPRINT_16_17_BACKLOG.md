# Sprint 16-17 (Weeks 61-68) - UI/HUD + Audio

KPI target: tool-calling can create HUD/UI layouts and baseline audio routing/playback with command-bus undo/redo.

## Sprint 16 - UI / HUD (`ui.*`)

- [x] S16-UI-01 UI runtime state model
  - Owner: ai/editor
  - Done: runtime now tracks canvases, UI elements, UI data bindings, and active HUD template.
  - Progress: `tool.get_engine_state` now exposes a dedicated `ui` section with counts and records.
  - Risks: no renderer-backed widget draw pipeline in this phase.
  - Mitigation: keep deterministic authoring contract while UI backend is integrated later.

- [x] S16-UI-02 UI authoring tool surface
  - Owner: ai
  - Done: added `ui.create_canvas`, `ui.add_panel`, `ui.add_text`, `ui.add_button`, `ui.bind_to_data`, `ui.create_hud_template`.
  - Progress: all operations execute via command bus and support undo by snapshot restore.
  - Risks: field schemas are intentionally flexible to support prompt-driven layouts.
  - Mitigation: tighten schema constraints once editor widget UX stabilizes.

- [x] S16-UI-03 Planner coverage
  - Owner: ai
  - Done: `gen.plan_from_prompt` now emits `ui.*` steps for HUD/UI/interface prompts.
  - Progress: planner path creates canvas/elements and sample binding flow.
  - Risks: prompt intent classification can overlap with other gameplay branches.
  - Mitigation: maintain prompt fixture regression tests.

## Sprint 17 - Audio Baseline (`audio.*`)

- [x] S17-AUDIO-01 Audio runtime state model
  - Owner: ai/audio
  - Done: runtime now tracks clips, sources, mixers, and play event counters.
  - Progress: `tool.get_engine_state` now includes `audio` snapshot sections.
  - Risks: no real DSP/mixer runtime execution in this phase.
  - Mitigation: preserve stable routing/playback metadata contract.

- [x] S17-AUDIO-02 Audio tool surface
  - Owner: ai
  - Done: added `audio.import_clip`, `audio.create_source`, `audio.play`, `audio.set_spatial`, `audio.create_mixer`, `audio.route`.
  - Progress: clip/source/mixer relationships are validated and mutation paths are undo-safe.
  - Risks: imported clip validation currently depends on filesystem availability.
  - Mitigation: keep clear validation errors and deterministic IDs.

- [x] S17-AUDIO-03 Planner + regression coverage
  - Owner: ai
  - Done: planner now emits `audio.*` steps for audio/sound/music prompts; tests cover phase16/17 state updates.
  - Progress: catalog, planner, and integration tests are extended through phase 17.
  - Risks: prompt vocabulary drift over time.
  - Mitigation: expand prompt fixture set as new phrasing appears.
