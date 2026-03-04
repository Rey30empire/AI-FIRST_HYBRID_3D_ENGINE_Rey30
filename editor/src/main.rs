mod input;

use anyhow::Context;
use input::InputState;
use std::time::{Duration, Instant};
use winit::dpi::PhysicalSize;
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::WindowBuilder;

const INITIAL_SIZE: PhysicalSize<u32> = PhysicalSize::new(1280, 720);
const DEMO_SCENE_PATH: &str = "samples/demo_scene.json";

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let event_loop = EventLoop::new().context("failed to create event loop")?;
    let window = WindowBuilder::new()
        .with_title("AI-First Hybrid 3D Engine | PR #4")
        .with_inner_size(INITIAL_SIZE)
        .build(&event_loop)
        .context("failed to create window")?;

    let window: &'static winit::window::Window = Box::leak(Box::new(window));

    let scene = match assets::load_scene(DEMO_SCENE_PATH) {
        Ok(scene) => scene,
        Err(err) => {
            log::warn!("{err}");
            assets::SceneFile::default()
        }
    };

    let world = ecs::SceneWorld::from_scene(&scene);
    log::info!(
        "Loaded scene '{}' with {} entities",
        scene.name,
        world.entity_count()
    );

    let mut renderer = pollster::block_on(render::Renderer::new(window))
        .context("failed to initialize renderer")?;
    let mut frame_clock = engine_core::FrameClock::new();
    let mut input = InputState::default();
    let mut camera = engine_core::OrbitCamera::new();
    let mut last_title_update = Instant::now();

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
                        let dt = stats.delta.as_secs_f32().min(0.1);

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

                        if last_title_update.elapsed() >= Duration::from_millis(300) {
                            let eye = camera.eye();
                            let target = camera.target();
                            window.set_title(&format!(
                                "AI-First Hybrid 3D Engine | {:.1} FPS | {:.2} ms | Eye [{:.1},{:.1},{:.1}] -> Target [{:.1},{:.1},{:.1}]",
                                stats.fps,
                                stats.frame_time_ms,
                                eye[0],
                                eye[1],
                                eye[2],
                                target[0],
                                target[1],
                                target[2]
                            ));
                            last_title_update = Instant::now();
                        }
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
