//! View > Text Size: how much the whole window is scaled.
//!
//! Explorer has no setting of its own for this. Windows 2000 scaled every
//! program at once through Display Properties > Settings > Advanced > Font
//! Size ("Small Fonts" 96 DPI, "Large Fonts" 120 DPI = 125%, or a custom
//! percentage), which enlarged text together with the layout around it;
//! build 10240 has the same idea as Settings > Display > "Change the size of
//! text, apps, and other items" (100 to 200%). Macsplorer scales its own
//! window the same way (egui's zoom factor), as a View menu choice like
//! Internet Explorer 5's View > Text Size. The steps are our choice: 100%,
//! the Win2000 "Large Fonts" 125%, then 150% and 200%.

/// The choices, smallest first, with their menu labels.
pub const SIZES: [(f32, &str); 4] = [(1.0, "Normal (100%)"), (1.25, "Large (125%)"), (1.5, "Larger (150%)"), (2.0, "Largest (200%)")];

pub const NORMAL: f32 = 1.0;

/// The choice closest to `zoom`, so a value edited by hand in the settings
/// file still lands on one of the menu items.
pub fn snap(zoom: f32) -> f32 {
    SIZES.iter().map(|&(z, _)| z).min_by(|a, b| (a - zoom).abs().total_cmp(&(b - zoom).abs())).unwrap_or(NORMAL)
}

/// The next larger choice (Cmd +), or the largest.
pub fn larger(zoom: f32) -> f32 {
    let i = index(zoom);
    SIZES[(i + 1).min(SIZES.len() - 1)].0
}

/// The next smaller choice (Cmd -), or the smallest.
pub fn smaller(zoom: f32) -> f32 {
    SIZES[index(zoom).saturating_sub(1)].0
}

fn index(zoom: f32) -> usize {
    let z = snap(zoom);
    SIZES.iter().position(|&(s, _)| s == z).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_up_and_down_through_the_choices() {
        assert_eq!(larger(1.0), 1.25);
        assert_eq!(larger(1.25), 1.5);
        assert_eq!(larger(1.5), 2.0);
        assert_eq!(larger(2.0), 2.0);
        assert_eq!(smaller(2.0), 1.5);
        assert_eq!(smaller(1.25), 1.0);
        assert_eq!(smaller(1.0), 1.0);
    }

    #[test]
    fn odd_values_snap_to_the_nearest_choice() {
        assert_eq!(snap(1.1), 1.0);
        assert_eq!(snap(1.2), 1.25);
        assert_eq!(snap(1.8), 2.0);
        assert_eq!(snap(9.0), 2.0);
        assert_eq!(snap(0.1), 1.0);
        assert_eq!(snap(f32::NAN), 1.0);
        assert_eq!(larger(1.3), 1.5);
    }
}
