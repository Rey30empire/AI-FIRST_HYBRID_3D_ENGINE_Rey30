use std::collections::HashSet;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::keyboard::{KeyCode, PhysicalKey};

#[derive(Default)]
pub struct InputState {
    pressed_keys: HashSet<KeyCode>,
    just_pressed_keys: HashSet<KeyCode>,
    orbit_drag_active: bool,
    last_cursor: Option<(f64, f64)>,
    orbit_delta: (f32, f32),
    scroll_delta: f32,
}

impl InputState {
    pub fn handle_window_event(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(code) = event.physical_key {
                    match event.state {
                        ElementState::Pressed => {
                            if self.pressed_keys.insert(code) {
                                self.just_pressed_keys.insert(code);
                            }
                        }
                        ElementState::Released => {
                            self.pressed_keys.remove(&code);
                        }
                    }
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if *button == MouseButton::Right {
                    self.orbit_drag_active = *state == ElementState::Pressed;
                    if !self.orbit_drag_active {
                        self.last_cursor = None;
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let current = (position.x, position.y);
                if self.orbit_drag_active
                    && let Some(previous) = self.last_cursor
                {
                    self.orbit_delta.0 += (current.0 - previous.0) as f32;
                    self.orbit_delta.1 += (current.1 - previous.1) as f32;
                }
                self.last_cursor = Some(current);
            }
            WindowEvent::MouseWheel { delta, .. } => match delta {
                MouseScrollDelta::LineDelta(_, y) => {
                    self.scroll_delta += *y;
                }
                MouseScrollDelta::PixelDelta(pos) => {
                    self.scroll_delta += (pos.y as f32) * 0.05;
                }
            },
            _ => {}
        }
    }

    pub fn movement_axes(&self) -> (f32, f32, f32) {
        let right = axis_from_keys(&self.pressed_keys, KeyCode::KeyD, KeyCode::KeyA);
        let forward = axis_from_keys(&self.pressed_keys, KeyCode::KeyW, KeyCode::KeyS);
        let up = axis_from_keys(&self.pressed_keys, KeyCode::Space, KeyCode::ShiftLeft)
            + axis_from_keys(&self.pressed_keys, KeyCode::KeyE, KeyCode::KeyQ);
        (right, up.clamp(-1.0, 1.0), forward)
    }

    pub fn take_orbit_delta(&mut self) -> (f32, f32) {
        let delta = self.orbit_delta;
        self.orbit_delta = (0.0, 0.0);
        delta
    }

    pub fn take_scroll_delta(&mut self) -> f32 {
        let delta = self.scroll_delta;
        self.scroll_delta = 0.0;
        delta
    }

    pub fn consume_key_press(&mut self, key: KeyCode) -> bool {
        self.just_pressed_keys.remove(&key)
    }

    pub fn end_frame(&mut self) {
        self.just_pressed_keys.clear();
    }
}

fn axis_from_keys(keys: &HashSet<KeyCode>, positive: KeyCode, negative: KeyCode) -> f32 {
    let mut axis = 0.0;
    if keys.contains(&positive) {
        axis += 1.0;
    }
    if keys.contains(&negative) {
        axis -= 1.0;
    }
    axis
}
