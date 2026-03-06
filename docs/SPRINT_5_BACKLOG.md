# Sprint 5 (Weeks 17-20) - AI Hybrid Production Bridge

KPI target: tool-calling works in `OFF/API/LOCAL` with auditable behavior and safe fallback.

- [x] S5-AI-01 Remote provider tool-calling bridge
  - Owner: ai
  - Done: `API` mode can POST tool-calls to a remote endpoint using a stable JSON schema and parse structured responses.
  - Progress: `AiOrchestrator::execute_tool` now attempts remote calls first when enabled, logs remote calls, then falls back to local `ToolRuntime` when non-strict.
  - Risks: remote endpoint instability.
  - Mitigation: timeout + strict/fallback flags + payload truncation in error surface.

- [x] S5-AI-02 LOCAL RPC tool-calling schema
  - Owner: ai
  - Done: `LOCAL` mode supports loopback RPC with the same request/response schema as API mode.
  - Progress: local bridge uses `http://<host>:<port>/<rpc_path>` and can be strict or fallback.
  - Risks: local model endpoint mismatch.
  - Mitigation: explicit `LOCAL_MLL_RPC_PATH` and integration tests for request schema.

- [x] S5-AI-03 Runtime safety controls
  - Owner: ai
  - Done: runtime exposes per-mode flags for enabling/disabling remote tool-calling and strict failure policy.
  - Progress: added env-driven controls for API and LOCAL RPC plus per-mode timeout controls.
  - Risks: misconfiguration across dev machines.
  - Mitigation: documented env flags and defaults in AI specs/setup docs.
