use anyhow::Context;
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
        .with_title("AI-First Hybrid 3D Engine | PR #1")
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
    let mut last_title_update = Instant::now();

    #[allow(deprecated)]
    event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Poll);

        match event {
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
            } if window_id == window.id() => {
                elwt.exit();
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::Resized(size),
            } if window_id == window.id() => {
                renderer.resize(size);
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::ScaleFactorChanged { .. },
            } if window_id == window.id() => {
                renderer.resize(window.inner_size());
            }
            Event::WindowEvent {
                window_id,
                event: WindowEvent::RedrawRequested,
            } if window_id == window.id() => {
                let stats = frame_clock.tick();

                match renderer.render() {
                    Ok(()) => {}
                    Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                        renderer.resize(window.inner_size());
                    }
                    Err(wgpu::SurfaceError::OutOfMemory) => elwt.exit(),
                    Err(wgpu::SurfaceError::Timeout) => log::warn!("surface timeout"),
                }

                if last_title_update.elapsed() >= Duration::from_millis(300) {
                    window.set_title(&format!(
                        "AI-First Hybrid 3D Engine | {:.1} FPS | {:.2} ms",
                        stats.fps, stats.frame_time_ms
                    ));
                    last_title_update = Instant::now();
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
