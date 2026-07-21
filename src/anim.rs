use std::time::Duration;

/// Animation tick interval (~60 fps).
pub const ANIM_INTERVAL: Duration = Duration::from_millis(16);

/// Exponential smoothing speed.  ~12.0 ⇒ ~300 ms to reach 95 % of target.
const ANIM_SPEED: f64 = 12.0;

/// Below this delta the value snaps to target (avoids endless micro-ticks).
const ANIM_SNAP_EPSILON: f32 = 0.05;

/// Lightweight per-value interpolator — no allocations, no traits, no async.
#[derive(Clone, Debug)]
pub struct AnimatedValue {
    pub current: f32,
    pub target: f32,
}

impl AnimatedValue {
    pub const fn new(value: f32) -> Self {
        Self {
            current: value,
            target: value,
        }
    }

    /// Advance one frame.  Returns `true` when `current` moved meaningfully.
    #[inline]
    pub fn tick(&mut self) -> bool {
        let diff = self.target - self.current;
        if diff.abs() < ANIM_SNAP_EPSILON {
            let moved = self.current != self.target;
            self.current = self.target;
            return moved;
        }
        self.current += diff * (1.0 - (-ANIM_SPEED * ANIM_INTERVAL.as_secs_f64()).exp()) as f32;
        true
    }
}
