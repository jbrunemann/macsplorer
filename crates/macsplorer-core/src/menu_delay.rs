//! How long the pointer rests on a menu item before its submenu opens.
//!
//! Windows' menus (user32) wait `SPI_GETMENUSHOWDELAY` before a hovered
//! submenu item drops its menu: `HKCU\Control Panel\Desktop\MenuShowDelay`,
//! 400 ms unless changed. A click or the Right arrow opens it at once, and
//! leaving the item before the delay is up cancels it. From memory: user32 is
//! not among the traced binaries.

/// `MenuShowDelay`'s default.
pub const SHOW_DELAY_MS: u64 = 400;

/// Seconds still to wait for a submenu item the pointer has rested on for
/// `hovered_for` seconds, or `None` once it may open.
pub fn wait_left(hovered_for: f64) -> Option<f64> {
    let left = SHOW_DELAY_MS as f64 / 1000.0 - hovered_for;
    (left > 0.0).then_some(left)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_after_the_delay() {
        assert_eq!(wait_left(0.0), Some(0.4));
        assert!((wait_left(0.1).unwrap() - 0.3).abs() < 1e-9);
        assert_eq!(wait_left(0.4), None);
        assert_eq!(wait_left(2.0), None);
    }
}
