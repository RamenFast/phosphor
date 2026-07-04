// SPDX-License-Identifier: GPL-3.0-or-later
//! The 3D modes' shared orbit camera and projection, ported from
//! phosphor_signal._project_3d. Math runs in f64 (the Python reference
//! promotes to f64 on every call after the first — all inputs are exact
//! f32 widenings, so uniform f64 here lands well inside the 0.05 px
//! parity tolerance) and casts to f32 at segment emission.

pub const CAMERA_MIN_DOLLY: f64 = 1.6;
pub const CAMERA_MAX_DOLLY: f64 = 8.0;
const TAU: f64 = std::f64::consts::TAU;

#[derive(Clone, Copy)]
pub struct Camera {
    pub yaw: f64,
    pub pitch: f64,
    pub dolly: f64,
}

impl Default for Camera {
    fn default() -> Self {
        // a pleasing three-quarter view until the mouse says otherwise
        Camera { yaw: 0.55, pitch: 0.35, dolly: 3.0 }
    }
}

impl Camera {
    /// Aim; None leaves an axis alone. Clamps mirror v3 exactly.
    pub fn set(&mut self, yaw: Option<f64>, pitch: Option<f64>, dolly: Option<f64>) {
        if let Some(yaw) = yaw {
            self.yaw = yaw.rem_euclid(TAU);
        }
        if let Some(pitch) = pitch {
            self.pitch = pitch.clamp(-1.45, 1.45);
        }
        if let Some(dolly) = dolly {
            self.dolly = dolly.clamp(CAMERA_MIN_DOLLY, CAMERA_MAX_DOLLY);
        }
    }

    /// Signal-space point (each axis ~[-1, 1]) through the camera to
    /// (screen x, screen y, fog 0..1). Fog dims what sits behind the
    /// figure — far phosphor is dim phosphor.
    #[inline]
    pub fn project(&self, a: f64, b: f64, c: f64, gain: f64,
                   width: f64, height: f64) -> (f64, f64, f64) {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let x1 = a * cos_yaw + c * sin_yaw;
        let z1 = c * cos_yaw - a * sin_yaw;
        let y1 = b * cos_pitch - z1 * sin_pitch;
        let z2 = b * sin_pitch + z1 * cos_pitch;
        let focal = self.dolly;
        let scale = focal / (focal + z2);
        let radius = width.min(height) * 0.45 * gain;
        (width / 2.0 + x1 * scale * radius,
         height / 2.0 - y1 * scale * radius,
         (0.9 - 0.3 * z2).clamp(0.15, 1.0))
    }
}
