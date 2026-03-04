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
