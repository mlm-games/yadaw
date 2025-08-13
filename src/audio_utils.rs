/// Calculate stereo gain values from volume and pan using equal-power panning
#[inline]
pub fn calculate_stereo_gains(volume: f32, pan: f32) -> (f32, f32) {
    let pan_normalized = (pan.clamp(-1.0, 1.0) + 1.0) / 2.0;
    let angle = pan_normalized * std::f32::consts::FRAC_PI_2;
    (volume * angle.cos(), volume * angle.sin())
}

/// Convert linear gain to decibels
#[inline]
pub fn linear_to_db(linear: f32) -> f32 {
    20.0 * linear.max(0.0001).log10()
}

/// Convert decibels to linear gain
#[inline]
pub fn db_to_linear(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Format pan value as a string (C for center, L/R with percentage)
#[inline]
pub fn format_pan(pan: f32) -> String {
    if pan.abs() < 0.01 {
        "C".to_string()
    } else if pan < 0.0 {
        format!("L{:.0}", -pan * 100.0)
    } else {
        format!("R{:.0}", pan * 100.0)
    }
}

/// Apply soft clipping to prevent harsh distortion
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    if x.abs() <= 0.5 {
        x
    } else {
        let sign = x.signum();
        sign * (0.5 + (x.abs() - 0.5).tanh() * 0.5)
    }
}
