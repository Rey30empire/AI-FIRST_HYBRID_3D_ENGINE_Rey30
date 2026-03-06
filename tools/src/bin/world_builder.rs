use anyhow::Context;
use std::env;
use std::fs;

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let prompt = env::args().skip(1).collect::<Vec<String>>().join(" ");
    if prompt.trim().is_empty() {
        anyhow::bail!(
            "usage: cargo run -p tools --bin world_builder -- \"create a medieval island map\""
        );
    }

    let mut orchestrator = ai::AiOrchestrator::from_env("logs/ai_tool_calls")
        .context("failed to initialize AI runtime from env")?;
    log::info!(
        "world builder requested in mode {}",
        orchestrator.mode().as_str()
    );
    let scene = orchestrator
        .world_builder(&prompt)
        .context("world builder failed")?;
    let output_path = "samples/generated_scene.json";
    let json =
        serde_json::to_string_pretty(&scene).context("failed to serialize generated scene")?;
    fs::write(output_path, json).with_context(|| format!("failed to write '{}'", output_path))?;
    log::info!("generated scene '{}' into {}", scene.name, output_path);
    Ok(())
}
