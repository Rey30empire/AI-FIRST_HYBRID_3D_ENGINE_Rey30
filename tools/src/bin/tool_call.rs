use anyhow::Context;
use std::env;

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = env::args().skip(1).collect::<Vec<String>>();
    if args.is_empty() {
        anyhow::bail!("usage: cargo run -p tools --bin tool_call -- <tool.name> [json_params]");
    }
    let tool_name = &args[0];
    let params = if args.len() >= 2 {
        serde_json::from_str::<serde_json::Value>(&args[1])
            .with_context(|| "failed to parse json_params argument")?
    } else {
        serde_json::json!({})
    };

    let mut orchestrator = ai::AiOrchestrator::from_env("logs/ai_tool_calls")
        .context("failed to initialize AI runtime from env")?;
    let result = orchestrator
        .execute_tool(tool_name, params)
        .with_context(|| format!("tool '{}' failed", tool_name))?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
