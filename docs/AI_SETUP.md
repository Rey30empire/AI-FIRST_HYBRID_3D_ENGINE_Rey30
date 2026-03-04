# AI Setup (OFF / API / LOCAL)

Date: 2026-03-04

## Security First

- Never commit secrets to git.
- Keep keys only in local `.env`.
- If any key was shared in plain text, rotate/revoke it immediately.

## Quick Start

1. Copy `.env.example` to `.env`.
2. Set `AI_MODE` to one of:
   - `OFF`
   - `API`
   - `LOCAL`
3. Launch editor:
   - `cargo run -p editor`

## API Mode

Required:

- `AI_MODE=API`
- `AI_API_PROVIDER` (example: `openai`, `anthropic`)
- `AI_API_KEY`

Optional:

- `AI_API_BASE_URL`

## LOCAL Mode (llama.cpp recommended)

Required:

- `AI_MODE=LOCAL`
- `LOCAL_MLL_BIN` (example: `C:\tools\llama.cpp\llama-server.exe`)
- `LOCAL_MLL_MODEL` (path to `.gguf`)

Optional:

- `LOCAL_MLL_HOST` (default: `127.0.0.1`)
- `LOCAL_MLL_PORT` (default: `8080`)
- `LOCAL_MLL_EXTRA_ARGS` (default example: `--ctx-size 4096`)
- `LOCAL_MLL_MAX_RESTARTS` (default: `2`)

The local model is executed in a separate process and supervised with restart limits.

## Editor Controls

- `F1`: switch to `OFF`
- `F2`: switch to `API`
- `F3`: switch to `LOCAL`
- `F6`: run World Builder and save `samples/generated_scene.json`

## Audit Logs

- Tool calls are append-only JSONL entries under:
  - `logs/ai_tool_calls/YYYY-MM-DD.log`
