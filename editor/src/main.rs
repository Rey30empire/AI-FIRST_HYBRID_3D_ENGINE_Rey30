mod input;

use anyhow::Context;
use input::InputState;
use serde_json::json;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use winit::dpi::PhysicalSize;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::keyboard::KeyCode;
use winit::window::WindowBuilder;

const INITIAL_SIZE: PhysicalSize<u32> = PhysicalSize::new(1280, 720);
const DEMO_SCENE_PATH: &str = "samples/demo_scene.json";
const GENERATED_SCENE_PATH: &str = "samples/generated_scene.json";

#[derive(Debug, Clone, Copy)]
enum CinematicPreset {
    NaturalDay,
    FilmicSunset,
    NoirIndoor,
}

impl CinematicPreset {
    fn id(self) -> &'static str {
        match self {
            Self::NaturalDay => "natural_day",
            Self::FilmicSunset => "filmic_sunset",
            Self::NoirIndoor => "noir_indoor",
        }
    }
}

fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let event_loop = EventLoop::new().context("failed to create event loop")?;
    let window = WindowBuilder::new()
        .with_title("AI-First Hybrid 3D Engine | PR #6")
        .with_inner_size(INITIAL_SIZE)
        .build(&event_loop)
        .context("failed to create window")?;

    let window: &'static winit::window::Window = Box::leak(Box::new(window));

    let current_scene = match assets::load_scene(DEMO_SCENE_PATH) {
        Ok(scene) => scene,
        Err(err) => {
            log::warn!("{err}");
            assets::SceneFile::default()
        }
    };

    let mut world = ecs::SceneWorld::from_scene(&current_scene);
    log::info!(
        "Loaded scene '{}' with {} entities",
        current_scene.name,
        world.entity_count()
    );

    let mut ai_config = ai::EngineAiConfig::from_env();
    let mut ai_orchestrator = match ai::AiOrchestrator::new(ai_config.clone(), "logs/ai_tool_calls")
    {
        Ok(orchestrator) => orchestrator,
        Err(err) => {
            log::error!(
                "failed to initialize AI mode {}: {err}. Falling back to OFF.",
                ai_config.mode.as_str()
            );
            ai_config.mode = ai::AiMode::Off;
            ai::AiOrchestrator::new(ai_config, "logs/ai_tool_calls")
                .context("failed to initialize AI in OFF mode")?
        }
    };
    log::info!("AI runtime: {}", ai_orchestrator.status());
    if let Err(err) = ai_orchestrator.sync_scene_from_editor(
        current_scene.clone(),
        Some(std::path::PathBuf::from(DEMO_SCENE_PATH)),
    ) {
        log::warn!("failed to seed tool runtime scene from editor: {err}");
    }
    let mut current_scene = current_scene;
    let mut last_tool_scene_revision = ai_orchestrator.tool_scene_revision();
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let cache_capacity_mb = std::env::var("ASSET_CACHE_MB")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(128);
    let mut asset_cache =
        assets::AsyncAssetCache::new(project_root, cache_capacity_mb * 1024 * 1024);
    queue_scene_asset_prefetch(&mut asset_cache, &current_scene);

    let mut renderer = pollster::block_on(render::Renderer::new(window))
        .context("failed to initialize renderer")?;
    renderer.set_scene_instances(&build_render_instances(&current_scene));
    let mut frame_clock = engine_core::FrameClock::new();
    let mut input = InputState::default();
    let mut camera = engine_core::OrbitCamera::new();
    let mut last_title_update = Instant::now();
    let mut profiler_panel_enabled = true;
    let mut last_profiler_log = Instant::now();

    #[allow(deprecated)]
    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Poll);

        match event {
            Event::WindowEvent {
                window_id,
                ref event,
            } if window_id == window.id() => {
                input.handle_window_event(event);
                match event {
                    WindowEvent::CloseRequested => {
                        elwt.exit();
                    }
                    WindowEvent::Resized(size) => {
                        renderer.resize(*size);
                    }
                    WindowEvent::ScaleFactorChanged { .. } => {
                        renderer.resize(window.inner_size());
                    }
                    WindowEvent::RedrawRequested => {
                        let stats = frame_clock.tick();
                        ai_orchestrator.set_frame_stats(stats.fps);
                        let dt = stats.delta.as_secs_f32().min(0.1);
                        let completed_cache_loads = asset_cache.poll();
                        if completed_cache_loads > 0 {
                            log::debug!("asset cache completed {} load(s)", completed_cache_loads);
                        }

                        if input.consume_key_press(KeyCode::F1)
                            && let Err(err) = ai_orchestrator.set_mode(ai::AiMode::Off) {
                                log::error!("failed to set AI mode OFF: {err}");
                            }
                        if input.consume_key_press(KeyCode::F2)
                            && let Err(err) = ai_orchestrator.set_mode(ai::AiMode::Api) {
                                log::error!("failed to set AI mode API: {err}");
                            }
                        if input.consume_key_press(KeyCode::F3)
                            && let Err(err) = ai_orchestrator.set_mode(ai::AiMode::Local) {
                                log::error!("failed to set AI mode LOCAL: {err}");
                            }
                        if input.consume_key_press(KeyCode::F4) {
                            match apply_lowcode_template(
                                &mut ai_orchestrator,
                                "template_shooter_arena",
                            ) {
                                Ok(()) => log::info!("Applied low-code template: shooter_arena"),
                                Err(err) => log::warn!(
                                    "failed to apply low-code template shooter_arena: {err}"
                                ),
                            }
                        }
                        if input.consume_key_press(KeyCode::F5) {
                            match apply_lowcode_template(
                                &mut ai_orchestrator,
                                "template_medieval_island",
                            ) {
                                Ok(()) => log::info!("Applied low-code template: medieval_island"),
                                Err(err) => log::warn!(
                                    "failed to apply low-code template medieval_island: {err}"
                                ),
                            }
                        }
                        if input.consume_key_press(KeyCode::F6) {
                            let prompt = std::env::var("WORLD_BUILDER_PROMPT")
                                .unwrap_or_else(|_| "create a medieval island map".to_string());
                            match ai_orchestrator.world_builder(&prompt) {
                                Ok(generated_scene) => {
                                    if let Err(err) =
                                        save_scene_json(&generated_scene, GENERATED_SCENE_PATH)
                                    {
                                        log::error!("failed to save generated scene: {err}");
                                    } else {
                                        log::info!(
                                            "world builder generated '{}' to '{}'",
                                            generated_scene.name,
                                            GENERATED_SCENE_PATH
                                        );
                                        if let Err(err) = ai_orchestrator.sync_scene_from_editor(
                                            generated_scene,
                                            Some(std::path::PathBuf::from(GENERATED_SCENE_PATH)),
                                        ) {
                                            log::warn!(
                                                "failed to sync generated scene to tool runtime: {err}"
                                            );
                                        }
                                    }
                                }
                                Err(err) => log::warn!("world builder call failed: {err}"),
                            }
                        }
                        if input.consume_key_press(KeyCode::F7) {
                            let prompt = std::env::var("GEN_PLAN_PROMPT")
                                .unwrap_or_else(|_| "create a shooter arena demo".to_string());
                            let plan = ai_orchestrator
                                .execute_tool("gen.plan_from_prompt", json!({ "prompt": prompt }));
                            match plan {
                                Ok(task_graph) => {
                                    match ai_orchestrator.execute_tool(
                                        "gen.execute_plan",
                                        json!({ "task_graph": task_graph }),
                                    ) {
                                        Ok(result) => log::info!("gen.execute_plan result: {result}"),
                                        Err(err) => log::warn!("gen.execute_plan failed: {err}"),
                                    }
                                }
                                Err(err) => log::warn!("gen.plan_from_prompt failed: {err}"),
                            }
                        }
                        if input.consume_key_press(KeyCode::F8) {
                            match apply_cinematic_preset(
                                &mut ai_orchestrator,
                                CinematicPreset::NaturalDay,
                            ) {
                                Ok(()) => log::info!("Applied cinematic preset: natural_day"),
                                Err(err) => {
                                    log::warn!("failed to apply cinematic preset natural_day: {err}")
                                }
                            }
                        }
                        if input.consume_key_press(KeyCode::F9) {
                            match apply_cinematic_preset(
                                &mut ai_orchestrator,
                                CinematicPreset::FilmicSunset,
                            ) {
                                Ok(()) => log::info!("Applied cinematic preset: filmic_sunset"),
                                Err(err) => {
                                    log::warn!(
                                        "failed to apply cinematic preset filmic_sunset: {err}"
                                    )
                                }
                            }
                        }
                        if input.consume_key_press(KeyCode::F10) {
                            match apply_cinematic_preset(
                                &mut ai_orchestrator,
                                CinematicPreset::NoirIndoor,
                            ) {
                                Ok(()) => log::info!("Applied cinematic preset: noir_indoor"),
                                Err(err) => {
                                    log::warn!("failed to apply cinematic preset noir_indoor: {err}")
                                }
                            }
                        }
                        if input.consume_key_press(KeyCode::F12) {
                            match apply_lowcode_template(
                                &mut ai_orchestrator,
                                "template_platform_runner",
                            ) {
                                Ok(()) => log::info!("Applied low-code template: platform_runner"),
                                Err(err) => log::warn!(
                                    "failed to apply low-code template platform_runner: {err}"
                                ),
                            }
                        }
                        if input.consume_key_press(KeyCode::F11) {
                            profiler_panel_enabled = !profiler_panel_enabled;
                            log::info!(
                                "profiler panel {}",
                                if profiler_panel_enabled {
                                    "enabled"
                                } else {
                                    "disabled"
                                }
                            );
                        }
                        if input.consume_key_press(KeyCode::KeyG) {
                            match ai_orchestrator
                                .execute_tool("graph.run", json!({ "events": ["OnUpdate"] }))
                            {
                                Ok(result) => log::info!("graph.run result: {result}"),
                                Err(err) => log::warn!("graph.run failed: {err}"),
                            }
                        }
                        if input.consume_key_press(KeyCode::KeyV) {
                            match ai_orchestrator.execute_tool("graph.validate", json!({})) {
                                Ok(report) => log::info!("graph.validate report: {report}"),
                                Err(err) => log::warn!("graph.validate failed: {err}"),
                            }
                        }

                        if let Err(err) = ai_orchestrator.tick() {
                            log::warn!("AI runtime tick failed: {err}");
                        }

                        let runtime_revision = ai_orchestrator.tool_scene_revision();
                        if runtime_revision != last_tool_scene_revision {
                            current_scene = ai_orchestrator.tool_scene_snapshot();
                            world.rebuild_from_scene(&current_scene);
                            renderer.set_scene_instances(&build_render_instances(&current_scene));
                            queue_scene_asset_prefetch(&mut asset_cache, &current_scene);
                            last_tool_scene_revision = runtime_revision;
                            log::info!(
                                "tool runtime scene synced: '{}' ({} entities, revision={})",
                                current_scene.name,
                                world.entity_count(),
                                runtime_revision
                            );
                        }

                        let render_settings = ai_orchestrator.tool_render_settings();
                        let lowcode_state = ai_orchestrator.tool_lowcode_state();
                        renderer.set_directional_light(render::DirectionalLightParams {
                            direction: render_settings.light_direction,
                            color: render_settings.light_color,
                            intensity: render_settings.light_intensity,
                            shadow_bias: render_settings.shadow_bias,
                            shadow_strength: render_settings.shadow_strength,
                            shadow_cascade_count: render_settings.shadow_cascade_count,
                        });
                        renderer.set_lod_settings(render::LodParams {
                            transition_distances: render_settings.lod_transition_distances,
                            hysteresis: render_settings.lod_hysteresis,
                        });
                        renderer.set_ibl(render::IblParams {
                            sky_color: render_settings.ibl_sky_color,
                            ground_color: render_settings.ibl_ground_color,
                            intensity: render_settings.ibl_intensity,
                        });
                        renderer.set_postprocess(render::ToneMapParams {
                            exposure: render_settings.exposure,
                            gamma: render_settings.gamma,
                            bloom_intensity: render_settings.bloom_intensity,
                            bloom_threshold: render_settings.bloom_threshold,
                            bloom_radius: render_settings.bloom_radius,
                            fog_density: render_settings.fog_density,
                            fog_color: render_settings.fog_color,
                            saturation: render_settings.saturation,
                            contrast: render_settings.contrast,
                            white_balance: render_settings.white_balance,
                            grade_tint: render_settings.grade_tint,
                        });

                        let (move_right, move_up, move_forward) = input.movement_axes();
                        camera.translate_local(move_right, move_up, move_forward, dt);

                        let (orbit_dx, orbit_dy) = input.take_orbit_delta();
                        if orbit_dx != 0.0 || orbit_dy != 0.0 {
                            camera.orbit(orbit_dx, orbit_dy);
                        }

                        let zoom_delta = input.take_scroll_delta();
                        if zoom_delta != 0.0 {
                            camera.zoom(zoom_delta);
                        }

                        let size = window.inner_size();
                        let aspect_ratio = size.width.max(1) as f32 / size.height.max(1) as f32;
                        renderer.update_camera(camera.view_proj_matrix(aspect_ratio), camera.eye());

                        match renderer.render() {
                            Ok(()) => {}
                            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                                renderer.resize(window.inner_size());
                            }
                            Err(wgpu::SurfaceError::OutOfMemory) => elwt.exit(),
                            Err(wgpu::SurfaceError::Timeout) => log::warn!("surface timeout"),
                        }
                        let render_stats = renderer.stats();
                        let cache_stats = asset_cache.stats();
                        let active_template = lowcode_state
                            .active_template_id
                            .clone()
                            .unwrap_or_else(|| "none".to_string());
                        let graph_nodes =
                            lowcode_state.graph.as_ref().map(|graph| graph.nodes.len()).unwrap_or(0);
                        let graph_edges =
                            lowcode_state.graph.as_ref().map(|graph| graph.edges.len()).unwrap_or(0);
                        let graph_last_exec_nodes = lowcode_state
                            .last_execution
                            .as_ref()
                            .map(|summary| summary.executed_node_ids.len())
                            .unwrap_or(0);

                        if last_title_update.elapsed() >= Duration::from_millis(300) {
                            let eye = camera.eye();
                            let target = camera.target();
                            let base_title = format!(
                                "AI-First Hybrid 3D Engine | {} | {:.1} FPS | {:.2} ms | Entities {} | Scene '{}' | Rev {} | Eye [{:.1},{:.1},{:.1}] -> Target [{:.1},{:.1},{:.1}]",
                                ai_orchestrator.mode().as_str(),
                                stats.fps,
                                stats.frame_time_ms,
                                world.entity_count(),
                                current_scene.name,
                                last_tool_scene_revision,
                                eye[0],
                                eye[1],
                                eye[2],
                                target[0],
                                target[1],
                                target[2]
                            );
                            if profiler_panel_enabled {
                                window.set_title(&format!(
                                    "{} | Prof CPU {:.2}ms | Cull {:.2}ms | Draw {} | Inst {}/{} | LOD [{},{},{}] t[{:.0},{:.0}] h{:.1} | Graph {} n:{} e:{} run:{} | GPU~ {:.1}MB | Cache {:.1}/{:.1}MB p:{}",
                                    base_title,
                                    render_stats.frame_cpu_ms,
                                    render_stats.cull_cpu_ms,
                                    render_stats.draw_calls_total,
                                    render_stats.visible_instances,
                                    render_stats.total_instances,
                                    render_stats.lod_visible_counts[0],
                                    render_stats.lod_visible_counts[1],
                                    render_stats.lod_visible_counts[2],
                                    render_settings.lod_transition_distances[0],
                                    render_settings.lod_transition_distances[1],
                                    render_settings.lod_hysteresis,
                                    active_template,
                                    graph_nodes,
                                    graph_edges,
                                    graph_last_exec_nodes,
                                    render_stats.gpu_buffer_mb_estimate,
                                    cache_stats.used_bytes as f32 / (1024.0 * 1024.0),
                                    cache_stats.capacity_bytes as f32 / (1024.0 * 1024.0),
                                    cache_stats.pending_requests
                                ));
                            } else {
                                window.set_title(&base_title);
                            }
                            last_title_update = Instant::now();
                        }
                        if profiler_panel_enabled
                            && last_profiler_log.elapsed() >= Duration::from_secs(1)
                        {
                            log::info!(
                                "profiler | cpu_frame_ms={:.2} cull_ms={:.2} draw_calls={} visible={}/{} culled={} lod=[{},{},{}] lod_dist=[{:.1},{:.1}] lod_h={:.1} graph_template={} graph_nodes={} graph_edges={} graph_last_exec={} gpu_buffer_mb_est={:.2}",
                                render_stats.frame_cpu_ms,
                                render_stats.cull_cpu_ms,
                                render_stats.draw_calls_total,
                                render_stats.visible_instances,
                                render_stats.total_instances,
                                render_stats.culled_instances,
                                render_stats.lod_visible_counts[0],
                                render_stats.lod_visible_counts[1],
                                render_stats.lod_visible_counts[2],
                                render_settings.lod_transition_distances[0],
                                render_settings.lod_transition_distances[1],
                                render_settings.lod_hysteresis,
                                active_template,
                                graph_nodes,
                                graph_edges,
                                graph_last_exec_nodes,
                                render_stats.gpu_buffer_mb_estimate
                            );
                            log::info!(
                                "asset_cache | used_mb={:.2} capacity_mb={:.2} cached={} pending={} hits={} misses={} evictions={}",
                                cache_stats.used_bytes as f32 / (1024.0 * 1024.0),
                                cache_stats.capacity_bytes as f32 / (1024.0 * 1024.0),
                                cache_stats.cached_assets,
                                cache_stats.pending_requests,
                                cache_stats.cache_hits,
                                cache_stats.cache_misses,
                                cache_stats.evictions
                            );
                            last_profiler_log = Instant::now();
                        }

                        input.end_frame();
                    }
                    _ => {}
                }
            }
            Event::AboutToWait => {
                window.request_redraw();
            }
            _ => {}
        }
    })?;

    Ok(())
}

fn save_scene_json(scene: &assets::SceneFile, path: &str) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(scene).context("failed to serialize scene to json")?;
    fs::write(path, json)
        .with_context(|| format!("failed to write generated scene to '{}'", path))?;
    Ok(())
}

fn build_render_instances(scene: &assets::SceneFile) -> Vec<render::SceneInstance> {
    let mut out = scene
        .entities
        .iter()
        .map(|entity| render::SceneInstance {
            translation: entity.translation,
            bounding_radius: 1.732,
        })
        .collect::<Vec<render::SceneInstance>>();
    if out.is_empty() {
        out.push(render::SceneInstance::default());
    }
    out
}

fn queue_scene_asset_prefetch(cache: &mut assets::AsyncAssetCache, scene: &assets::SceneFile) {
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut seen = HashSet::<String>::new();
    for entity in &scene.entities {
        let mesh_id = entity.mesh.trim();
        if mesh_id.is_empty() || !seen.insert(mesh_id.to_string()) {
            continue;
        }
        let mesh_path = PathBuf::from(mesh_id);
        let resolved = if mesh_path.is_absolute() {
            mesh_path.clone()
        } else {
            project_root.join(&mesh_path)
        };
        if resolved.is_file() {
            let _ = cache.request_load(mesh_id.to_string(), mesh_path);
        }
    }
}

fn apply_cinematic_preset(
    ai_orchestrator: &mut ai::AiOrchestrator,
    preset: CinematicPreset,
) -> anyhow::Result<()> {
    match preset {
        CinematicPreset::NaturalDay => {
            ai_orchestrator.execute_tool(
                "render.set_light_params",
                json!({
                    "direction": [-0.42, -1.0, -0.28],
                    "color": [1.0, 0.98, 0.95],
                    "intensity": 5.8,
                    "shadow_cascade_count": 3
                }),
            )?;
            ai_orchestrator.execute_tool(
                "render.set_ibl",
                json!({
                    "intensity": 0.65,
                    "sky_color": [0.70, 0.82, 1.0],
                    "ground_color": [0.24, 0.22, 0.18]
                }),
            )?;
            ai_orchestrator.execute_tool(
                "render.set_postprocess",
                json!({
                    "preset": preset.id(),
                    "exposure": 1.0,
                    "gamma": 2.2
                }),
            )?;
            ai_orchestrator.execute_tool("scene.set_sky", json!({ "preset": "clear_day" }))?;
            ai_orchestrator.execute_tool("scene.set_time_of_day", json!({ "value": 13.0 }))?;
            ai_orchestrator.execute_tool(
                "scene.add_fog",
                json!({
                    "density": 0.01,
                    "color": [0.78, 0.84, 0.92],
                    "start": 35.0,
                    "end": 260.0
                }),
            )?;
        }
        CinematicPreset::FilmicSunset => {
            ai_orchestrator.execute_tool(
                "render.set_light_params",
                json!({
                    "direction": [-0.24, -0.66, -0.18],
                    "color": [1.0, 0.70, 0.42],
                    "intensity": 6.5,
                    "shadow_cascade_count": 3
                }),
            )?;
            ai_orchestrator.execute_tool(
                "render.set_ibl",
                json!({
                    "intensity": 0.72,
                    "sky_color": [1.0, 0.78, 0.54],
                    "ground_color": [0.32, 0.24, 0.19]
                }),
            )?;
            ai_orchestrator.execute_tool(
                "render.set_postprocess",
                json!({
                    "preset": preset.id(),
                    "exposure": 1.08,
                    "gamma": 2.15
                }),
            )?;
            ai_orchestrator.execute_tool("scene.set_sky", json!({ "preset": "sunset_hazy" }))?;
            ai_orchestrator.execute_tool("scene.set_time_of_day", json!({ "value": 18.4 }))?;
            ai_orchestrator.execute_tool(
                "scene.add_fog",
                json!({
                    "density": 0.08,
                    "color": [0.97, 0.64, 0.45],
                    "start": 12.0,
                    "end": 120.0
                }),
            )?;
        }
        CinematicPreset::NoirIndoor => {
            ai_orchestrator.execute_tool(
                "render.set_light_params",
                json!({
                    "direction": [-0.18, -1.0, -0.12],
                    "color": [0.70, 0.78, 0.96],
                    "intensity": 3.0,
                    "shadow_cascade_count": 2
                }),
            )?;
            ai_orchestrator.execute_tool(
                "render.set_ibl",
                json!({
                    "intensity": 0.45,
                    "sky_color": [0.36, 0.42, 0.56],
                    "ground_color": [0.12, 0.12, 0.14]
                }),
            )?;
            ai_orchestrator.execute_tool(
                "render.set_postprocess",
                json!({
                    "preset": preset.id(),
                    "exposure": 0.75,
                    "gamma": 2.4
                }),
            )?;
            ai_orchestrator.execute_tool("scene.set_sky", json!({ "preset": "studio_noir" }))?;
            ai_orchestrator.execute_tool("scene.set_time_of_day", json!({ "value": 22.0 }))?;
            ai_orchestrator.execute_tool(
                "scene.add_fog",
                json!({
                    "density": 0.04,
                    "color": [0.20, 0.22, 0.28],
                    "start": 3.0,
                    "end": 48.0
                }),
            )?;
        }
    }
    Ok(())
}

fn apply_lowcode_template(
    ai_orchestrator: &mut ai::AiOrchestrator,
    template_id: &str,
) -> anyhow::Result<()> {
    ai_orchestrator.execute_tool(
        "template.apply",
        json!({
            "template_id": template_id
        }),
    )?;
    ai_orchestrator.execute_tool("graph.run", json!({ "events": ["OnStart"] }))?;
    Ok(())
}
