//! Courbes de fondu.

use std::f64::consts::FRAC_PI_2;

/// Fondu à puissance constante (`powerCurves` de la version Electron) : à l'avancement `x` (0..1), gains du morceau
/// sortant et du morceau entrant. cos² + sin² = 1 : la somme de deux morceaux non corrélés garde la même énergie.
#[inline]
pub fn equal_power(x: f64) -> (f32, f32) {
    let x = x.clamp(0.0, 1.0);
    let (rise, fall) = (x * FRAC_PI_2).sin_cos();
    (fall as f32, rise as f32)
}

/// Courbe en S (0 → 1) : la vitesse du morceau entrant se détend ainsi après un mixage AutoMix (`easeRate`).
#[inline]
pub fn smoothstep(k: f64) -> f64 {
    let k = k.clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

/// Interpolation linéaire par morceaux entre `(x0, y0)` et `(x1, y1)`, constante au-dehors.
#[inline]
pub fn ramp_between(x: f64, x0: f64, y0: f64, x1: f64, y1: f64) -> f64 {
    if x <= x0 {
        y0
    } else if x >= x1 {
        y1
    } else {
        y0 + (y1 - y0) * (x - x0) / (x1 - x0)
    }
}
