# AI Hybrid Minimum Real Spec (Phase 5)

## Toggle API contract

Rust enum:

```rust
pub enum AiMode {
    Off,
    Api,
    Local,
}
```

Rules:

- `Off`: AI services are not initialized, no model process, no API client.
- `Api`: request pipeline enabled through remote provider connector.
- `Local`: model runtime launched as separate process and managed by supervisor.

## Local mode process isolation

- Editor process and local model process are separate.
- IPC channel: local loopback HTTP or stdio RPC.
- Supervisor restarts crashed model process with capped retries.
- UI remains responsive if local model stalls.

## Tool-calling with auditable logs

Each tool call writes append-only log entries:

- timestamp
- user/session id
- agent id
- tool name
- input hash + redacted preview
- result status
- duration_ms

Storage:
- `logs/ai_tool_calls/YYYY-MM-DD.log`

## World Builder agent (minimum)

Input:
- prompt text
- optional style preset

Output:
- base scene JSON (terrain/layout/entities/lights placeholders)

Flow:
1. Prompt parsed into scene intent.
2. Planner creates tasks (`terrain`, `structures`, `spawn points`, `lighting`).
3. Builder emits scene JSON and asset references.
4. Editor imports scene and opens it.

## Done criteria

- User toggles OFF/API/LOCAL live in editor settings
- OFF mode proves zero AI initialization
- LOCAL mode runs in separate process and can be terminated independently
- One `World Builder` call creates a valid base scene from prompt

## Implementation Status

- Implemented in PR #5:
  - `AiOrchestrator` with runtime switching (`OFF/API/LOCAL`)
  - Local MLL supervisor in separate process (`llama-server` compatible)
  - Audit logs in `logs/ai_tool_calls/YYYY-MM-DD.log`
  - World Builder generation + scene export to `samples/generated_scene.json`
- Implemented in current iteration (S5):
  - Remote HTTP tool-calling bridge in `API` mode (JSON schema v1 payload, configurable endpoint, timeout, strict/fallback behavior)
  - Local RPC tool-calling bridge for `LOCAL` mode via loopback HTTP endpoint
  - Shared RPC request/response schema for tool invocation:
    - request: `schema_version`, `session_id`, `mode`, `tool_name`, `params`, `timestamp_utc`
    - response: `status`, `result`, `error`, `trace_id`

## S5 Env Flags

- API remote tools:
  - `AI_API_REMOTE_TOOL_CALLS` (`true/false`)
  - `AI_API_REMOTE_TOOL_CALLS_STRICT` (`true/false`)
  - `AI_API_TOOL_ENDPOINT` (optional explicit endpoint)
  - `AI_API_TIMEOUT_MS` (default `8000`)

- LOCAL RPC tools:
  - `LOCAL_MLL_RPC_TOOL_CALLS` (`true/false`)
  - `LOCAL_MLL_RPC_TOOL_CALLS_STRICT` (`true/false`)
  - `LOCAL_MLL_RPC_PATH` (default `/tool-call`)
  - `LOCAL_MLL_RPC_TIMEOUT_MS` (default `5000`)
