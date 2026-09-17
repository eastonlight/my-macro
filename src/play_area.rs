//! Calibrated StarCraft: Remastered 1920×1080 playfield geometry.
//!
//! Replaces flat rectangular bounds with a conservative piecewise Zerg HUD
//! contour, top-right resource bar mask, and screen-edge scroll guard.
//! Calibrated against user-permitted background captures (`spire-screen-1080`
//! and `spire-dense-1080`).

use crate::frame::Point;

/// Client width of the supported Remastered profile.
pub const CLIENT_WIDTH: i32 = 1920;
/// Client height of the supported Remastered profile.
pub const CLIENT_HEIGHT: i32 = 1080;

/// Screen-edge guard margin (in pixels) to avoid camera scrolling.
///
/// In StarCraft: Remastered, moving the mouse to the border (within ~1-2 px of
/// the window edges) triggers camera scrolling. A conservative 24 px margin
/// guarantees that clicks never scroll the view.
pub const EDGE_GUARD: i32 = 24;

/// Returns the conservative maximum playable `y` (exclusive) for a given `x`,
/// keeping clicks safely above the Zerg HUD consoles and decorations.
///
/// Calibrated against both calibration and dense screenshots:
/// - `x < 120`: minimap horn/claw decoration rises up to y ≈ 643; guard ceiling = 640.
/// - `120..540`: minimap console top ridge sits at y ≈ 677..708; guard ceiling = 670.
/// - `540..660`: transition slope from minimap to central console (670 down to 770).
/// - `660..1260`: central unit wireframe / portrait console dips down (y ≈ 781..827);
///   guard ceiling = 770 (playable lower corridor).
/// - `1260..1340`: transition slope from central console to command card (770 up to 710).
/// - `1340..1840`: command card console top edge sits at y ≈ 719..738; guard ceiling = 710.
/// - `x >= 1840`: far-right console corner sits at y ≈ 707..715; guard ceiling = 700.
pub fn hud_skyline_y(x: i32) -> i32 {
    if x < 120 {
        640
    } else if x < 540 {
        670
    } else if x < 660 {
        // Ramp from (540, 670) to (660, 770)
        670 + (x - 540) * 100 / 120
    } else if x < 1260 {
        770
    } else if x < 1340 {
        // Ramp from (1260, 770) to (1340, 710)
        770 - (x - 1260) * 60 / 80
    } else if x < 1840 {
        710
    } else {
        700
    }
}

/// Returns true if `point` is a safe, playable cursor clickpoint.
///
/// A point is playable if:
/// 1. It is within `[EDGE_GUARD, CLIENT_WIDTH - EDGE_GUARD)` horizontally.
/// 2. It is `>= EDGE_GUARD` vertically (clears top edge scroll trigger).
/// 3. It clears the top-right resource bar (`x >= 1440` and `y < 48`).
/// 4. It is strictly above the bottom HUD skyline (`y < hud_skyline_y(x)`).
pub fn is_playable_point(point: Point) -> bool {
    if point.x < EDGE_GUARD || point.x >= CLIENT_WIDTH - EDGE_GUARD {
        return false;
    }
    if point.y < EDGE_GUARD {
        return false;
    }
    // Top-right resource bar (minerals, gas, supply widgets occupy x in 1460..1920, y in 0..40).
    if point.x >= 1440 && point.y < 48 {
        return false;
    }
    point.y < hud_skyline_y(point.x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edge_guards_reject_border_clicks() {
        assert!(!is_playable_point(Point::new(0, 400)));
        assert!(!is_playable_point(Point::new(EDGE_GUARD - 1, 400)));
        assert!(!is_playable_point(Point::new(
            CLIENT_WIDTH - EDGE_GUARD,
            400
        )));
        assert!(!is_playable_point(Point::new(CLIENT_WIDTH, 400)));
        assert!(!is_playable_point(Point::new(400, 0)));
        assert!(!is_playable_point(Point::new(400, EDGE_GUARD - 1)));
    }

    #[test]
    fn test_resource_bar_rejection() {
        // Inside resource bar:
        assert!(!is_playable_point(Point::new(1450, 30)));
        assert!(!is_playable_point(Point::new(1600, 20)));
        assert!(!is_playable_point(Point::new(1800, 40)));
        // Below resource bar:
        assert!(is_playable_point(Point::new(1450, 50)));
        assert!(is_playable_point(Point::new(1600, 60)));
        // Left of resource bar near top:
        assert!(is_playable_point(Point::new(1400, 30)));
        assert!(is_playable_point(Point::new(400, 30)));
    }

    #[test]
    fn test_hud_skyline_calibrated_points() {
        // Minimap horn area (x < 120):
        assert!(is_playable_point(Point::new(60, 630)));
        assert!(!is_playable_point(Point::new(60, 645)));

        // Minimap console (120..540):
        assert!(is_playable_point(Point::new(300, 660)));
        assert!(!is_playable_point(Point::new(300, 675)));
        assert!(!is_playable_point(Point::new(200, 740)));

        // Central corridor (660..1260):
        assert!(is_playable_point(Point::new(800, 750)));
        assert!(is_playable_point(Point::new(862, 765)));
        assert!(!is_playable_point(Point::new(862, 780)));
        assert!(!is_playable_point(Point::new(1000, 800)));

        // Command card (1340..1840):
        assert!(is_playable_point(Point::new(1600, 700)));
        assert!(!is_playable_point(Point::new(1600, 715)));
    }
}
