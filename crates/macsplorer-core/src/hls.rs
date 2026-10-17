//! Hue, Lum and Sat as the Windows Color dialog shows them: hue 0 to 239,
//! sat and lum 0 to 240 (`HLSMAX`). The conversions follow Microsoft's
//! published sample (KB 29240), the same integer math with rounding, not
//! comdlg32's code, which isn't analyzed.

const HLSMAX: i32 = 240;
const RGBMAX: i32 = 255;
/// The hue a gray gets (the sample's `UNDEFINED`).
const UNDEFINED: i32 = HLSMAX * 2 / 3;

/// `[hue, sat, lum]` of an sRGB color.
pub fn from_rgb([r, g, b]: [u8; 3]) -> [i32; 3] {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let lum = ((max + min) * HLSMAX + RGBMAX) / (2 * RGBMAX);
    if max == min {
        return [UNDEFINED, 0, lum];
    }
    let sat = if lum <= HLSMAX / 2 {
        ((max - min) * HLSMAX + (max + min) / 2) / (max + min)
    } else {
        ((max - min) * HLSMAX + (2 * RGBMAX - max - min) / 2) / (2 * RGBMAX - max - min)
    };
    let delta = |c: i32| ((max - c) * (HLSMAX / 6) + (max - min) / 2) / (max - min);
    let mut hue = if r == max {
        delta(b) - delta(g)
    } else if g == max {
        HLSMAX / 3 + delta(r) - delta(b)
    } else {
        2 * HLSMAX / 3 + delta(g) - delta(r)
    };
    if hue < 0 {
        hue += HLSMAX;
    }
    if hue > HLSMAX {
        hue -= HLSMAX;
    }
    [hue.min(HLSMAX - 1), sat, lum]
}

/// The sRGB color of `[hue, sat, lum]`; out of range values are clamped.
pub fn to_rgb([hue, sat, lum]: [i32; 3]) -> [u8; 3] {
    let (hue, sat, lum) = (hue.clamp(0, HLSMAX - 1), sat.clamp(0, HLSMAX), lum.clamp(0, HLSMAX));
    if sat == 0 {
        let v = (lum * RGBMAX / HLSMAX) as u8;
        return [v, v, v];
    }
    let m2 = if lum <= HLSMAX / 2 { (lum * (HLSMAX + sat) + HLSMAX / 2) / HLSMAX } else { lum + sat - (lum * sat + HLSMAX / 2) / HLSMAX };
    let m1 = 2 * lum - m2;
    let channel = |h: i32| ((hue_to_rgb(m1, m2, h) * RGBMAX + HLSMAX / 2) / HLSMAX).clamp(0, RGBMAX) as u8;
    [channel(hue + HLSMAX / 3), channel(hue), channel(hue - HLSMAX / 3)]
}

fn hue_to_rgb(n1: i32, n2: i32, hue: i32) -> i32 {
    let hue = hue.rem_euclid(HLSMAX);
    if hue < HLSMAX / 6 {
        n1 + ((n2 - n1) * hue + HLSMAX / 12) / (HLSMAX / 6)
    } else if hue < HLSMAX / 2 {
        n2
    } else if hue < HLSMAX * 2 / 3 {
        n1 + ((n2 - n1) * (HLSMAX * 2 / 3 - hue) + HLSMAX / 12) / (HLSMAX / 6)
    } else {
        n1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_colors() {
        assert_eq!(from_rgb([255, 0, 0]), [0, 240, 120]);
        assert_eq!(from_rgb([0, 255, 0]), [80, 240, 120]);
        assert_eq!(from_rgb([0, 0, 255]), [160, 240, 120]);
        assert_eq!(from_rgb([255, 255, 255]), [160, 0, 240]);
        assert_eq!(from_rgb([0, 0, 0]), [160, 0, 0]);
        assert_eq!(to_rgb([0, 240, 120]), [255, 0, 0]);
        assert_eq!(to_rgb([160, 0, 240]), [255, 255, 255]);
        assert_eq!(to_rgb([40, 240, 120]), [255, 255, 0]);
    }

    /// Going to HLS and back moves a color by at most a few steps.
    #[test]
    fn round_trips_closely() {
        for r in (0..=255).step_by(17) {
            for g in (0..=255).step_by(51) {
                for b in (0..=255).step_by(85) {
                    let back = to_rgb(from_rgb([r, g, b]));
                    let off = (0..3).map(|i| (back[i] as i32 - [r, g, b][i] as i32).abs()).max().unwrap();
                    assert!(off <= 3, "{:?} -> {:?}", [r, g, b], back);
                }
            }
        }
    }
}
