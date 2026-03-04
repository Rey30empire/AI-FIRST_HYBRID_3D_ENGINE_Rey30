use glam::{Mat4, Vec3};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct FrameStats {
    pub delta: Duration,
    pub frame_time_ms: f32,
    pub fps: f32,
}

pub struct FrameClock {
    last_frame: Instant,
    fps_window_start: Instant,
    frames_in_window: u32,
    fps: f32,
}

impl Default for FrameClock {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameClock {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            last_frame: now,
            fps_window_start: now,
            frames_in_window: 0,
            fps: 0.0,
        }
    }

    pub fn tick(&mut self) -> FrameStats {
        let now = Instant::now();
        let delta = now.saturating_duration_since(self.last_frame);
        self.last_frame = now;

        self.frames_in_window += 1;
        let elapsed = now.saturating_duration_since(self.fps_window_start);
        if elapsed >= Duration::from_secs(1) {
            self.fps = self.frames_in_window as f32 / elapsed.as_secs_f32();
            self.frames_in_window = 0;
            self.fps_window_start = now;
        }

        FrameStats {
            delta,
            frame_time_ms: delta.as_secs_f32() * 1000.0,
            fps: self.fps,
        }
    }
}

#[derive(Debug, Clone)]
pub struct OrbitCamera {
    target: Vec3,
    yaw: f32,
    pitch: f32,
    radius: f32,
    move_speed: f32,
    orbit_sensitivity: f32,
    zoom_sensitivity: f32,
    min_radius: f32,
    max_radius: f32,
}

impl Default for OrbitCamera {
    fn default() -> Self {
        Self::new()
    }
}

impl OrbitCamera {
    pub fn new() -> Self {
        Self {
            target: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.25,
            radius: 4.0,
            move_speed: 4.0,
            orbit_sensitivity: 0.004,
            zoom_sensitivity: 0.35,
            min_radius: 1.0,
            max_radius: 40.0,
        }
    }

    pub fn orbit(&mut self, delta_x: f32, delta_y: f32) {
        self.yaw -= delta_x * self.orbit_sensitivity;
        self.pitch = (self.pitch - delta_y * self.orbit_sensitivity).clamp(-1.45, 1.45);
    }

    pub fn zoom(&mut self, scroll_delta: f32) {
        self.radius = (self.radius - scroll_delta * self.zoom_sensitivity)
            .clamp(self.min_radius, self.max_radius);
    }

    pub fn translate_local(&mut self, right: f32, up: f32, forward: f32, dt: f32) {
        let planar_forward = self.planar_forward();
        let planar_right = Vec3::new(-planar_forward.z, 0.0, planar_forward.x);
        let translation =
            (planar_right * right + Vec3::Y * up + planar_forward * forward) * self.move_speed * dt;
        self.target += translation;
    }

    pub fn eye(&self) -> [f32; 3] {
        self.eye_vec3().to_array()
    }

    pub fn target(&self) -> [f32; 3] {
        self.target.to_array()
    }

    pub fn view_proj_matrix(&self, aspect_ratio: f32) -> [[f32; 4]; 4] {
        let eye = self.eye_vec3();
        let up = Vec3::Y;
        let view = Mat4::look_at_rh(eye, self.target, up);
        let proj = Mat4::perspective_rh_gl(60.0_f32.to_radians(), aspect_ratio, 0.1, 500.0);
        (proj * view).to_cols_array_2d()
    }

    fn eye_vec3(&self) -> Vec3 {
        let cos_pitch = self.pitch.cos();
        let x = self.radius * self.yaw.cos() * cos_pitch;
        let y = self.radius * self.pitch.sin();
        let z = self.radius * self.yaw.sin() * cos_pitch;
        self.target + Vec3::new(x, y, z)
    }

    fn planar_forward(&self) -> Vec3 {
        let forward = (self.target - self.eye_vec3()).normalize_or_zero();
        let planar = Vec3::new(forward.x, 0.0, forward.z);
        if planar.length_squared() > 1e-6 {
            planar.normalize()
        } else {
            Vec3::Z
        }
    }
}
