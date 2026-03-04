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