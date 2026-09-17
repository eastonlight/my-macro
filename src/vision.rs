//! Pixel detectors for the 1920×1080 Remastered HUD.
//!
//! Two independent readings are implemented here, both pure and host
//! testable:
//!
//! * [`detect_selection`] classifies the bottom selection HUD as a group of
//!   `2..=12` drones, a single selected drone, or "not a valid drone
//!   selection". Wireframe slots are detected with the selected-unit frame
//!   colour and every *occupied* slot is then verified against a small
//!   embedded drone-body mask, so a colony, a tooltip or a mixed selection can
//!   never be counted as drones.
//! * [`detect_placement`] looks for the green/red placement preview square
//!   near a screen position.
//!
//! Calibration comes from `tests/fixtures/remastered-1080` (real 1920×1080
//! captures). Only that single profile is supported: anything else fails
//! closed instead of guessing a scale.

use crate::frame::{Frame, Point, Rect, Rgb};

/// Client size of the only supported profile.
pub const CLIENT_WIDTH: u32 = 1920;
/// Client height of the only supported profile.
pub const CLIENT_HEIGHT: u32 = 1080;
/// Footprint width of a Creep Colony, and the exact centre-to-centre spacing
/// of a left-to-right row (`64 logical px * 1080 / 480`).
/// Screen footprint of a 2×2 Creep Colony at this profile, and the reference
/// for the live-verified preview calibration.
pub const FOOTPRINT: i32 = 144;
/// Screen footprint of a 3×3 Spire (`96 logical px * 1080 / 480`). The tile math
/// matches the verified colony calibration; the preview itself has not been
/// captured live, so a wrong value fails the green-preview check instead of
/// clicking.
pub const SPIRE_FOOTPRINT: i32 = 216;
/// A selection can hold at most 12 units.
pub const MAX_DRONES: u8 = 12;

/// Screen centre of wireframe slot 0 (column 0, row 0).
const SLOT0: Point = Point::new(653, 930);
/// Horizontal distance between wireframe columns.
const COL_STEP: i32 = 81;
/// Vertical distance between wireframe rows.
const ROW_STEP: i32 = 83;
/// Wireframe columns and rows in the group panel.
const COLUMNS: u8 = 6;
const ROWS: u8 = 2;
/// Half size of one wireframe box, used for the occupancy frame scan.
const SLOT_HALF_X: i32 = 37;
const SLOT_HALF_Y: i32 = 38;
/// A selected-unit frame contributes hundreds of bright blue pixels.
const MIN_FRAME_BLUE_PIXELS: usize = 60;

/// Coarse mask grid: 16×16 cells of 4×4 pixels, centred on the feature.
const MASK_SIDE: i32 = 16;
const MASK_CELL: i32 = 4;
const MASK_HALF: i32 = MASK_SIDE * MASK_CELL / 2;

/// The mask offsets tried when aligning a feature, in pixels.
const ALIGN_OFFSETS: [i32; 5] = [-6, -3, 0, 3, 6];

/// Minimum mask agreement (IoU of the red-body cells) for a wireframe slot to
/// be accepted as a drone. Across the real fixtures the same drone scores
/// 0.96–1.00 while a tooltip, an empty slot or a Creep Colony scores 0.00.
const DRONE_MIN_AGREEMENT: f32 = 0.6;
/// Minimum number of red-body cells; rejects tiny red speckles.
const DRONE_MIN_CELLS: usize = 25;
/// Same idea for the single-unit information panel portrait.
const SINGLE_MIN_AGREEMENT: f32 = 0.6;
const SINGLE_MIN_CELLS: usize = 40;

/// Screen centre of the large portrait in the single-unit information panel.
const SINGLE_PORTRAIT: Point = Point::new(696, 950);

/// Red-body silhouette of a drone wireframe, derived from
/// `drones-2.png` (slot 0) with the same grid the detector uses at run time.
const DRONE_WIREFRAME_MASK: [&str; MASK_SIDE as usize] = [
    "...###..........",
    "...####.#.......",
    "..###########...",
    "..############..",
    ".##############.",
    ".###############",
    "########.##.#..#",
    "########.####...",
    "##############..",
    "#####.#####.###.",
    "####.#####..###.",
    "..##.#####...##.",
    "..##..#####..#..",
    ".......#####....",
    "................",
    "................",
];

/// Red-body silhouette of the single-unit information panel portrait,
/// derived from `drone-single.png`.
const SINGLE_DRONE_PORTRAIT_MASK: [&str; MASK_SIDE as usize] = [
    "################",
    "####....########",
    "###.############",
    "################",
    "############....",
    "######...###...#",
    "#####....#######",
    "######...#######",
    "########.##.####",
    "###########..###",
    "#.##########..##",
    "..###.#####...##",
    ".####..###....##",
    "######........##",
    "#######........#",
    "..########......",
];

/// Smallest search radius around the cursor for a placement preview. The real
/// radius grows with the footprint, so a 3×3 building is still found.
const PREVIEW_SEARCH_MIN: i32 = 176;
/// Accepted preview square side, relative to the expected footprint: the game
/// snaps the preview to the tile grid, so a little slack is needed.
const PREVIEW_SIDE_MIN_NUM: i32 = 3;
const PREVIEW_SIDE_MAX_NUM: i32 = 5;
const PREVIEW_SIDE_DEN: i32 = 4;
/// How much of its bounding box the overlay must fill.
const PREVIEW_MIN_FILL: f32 = 0.30;
/// Width of the terrain ring checked directly outside the candidate square.
const PREVIEW_RING: i32 = 10;
/// The ring outside a real preview is terrain, never overlay coloured.
const PREVIEW_MAX_RING_FILL: f32 = 0.15;

/// What the selection HUD contains.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionRead {
    /// A group of `2..=12` drones.
    Drones { count: u8 },
    /// Exactly one drone, shown as the single-unit information panel.
    SingleDrone,
    /// Anything else; the caller must not inject input.
    Rejected(RejectReason),
}

/// Why a selection was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectReason {
    /// The capture is not the supported 1920×1080 client.
    UnsupportedProfile,
    /// `PrintWindow` produced a black/empty frame.
    BlankCapture,
    /// No drone wireframes and no drone portrait.
    NoDroneSelection,
    /// An occupied wireframe slot that does not look like a drone
    /// (another unit type, or a tooltip/overlay covering the HUD).
    OccupiedNonDrone { slot: u8 },
    /// A lone wireframe, which the game does not use for a single unit.
    SingleWireframe,
    /// The information panel portrait is not a drone (e.g. a Creep Colony).
    UncertainSingle,
}

impl RejectReason {
    /// English detail that is appended to the localized status line.
    pub const fn detail(self) -> &'static str {
        match self {
            Self::UnsupportedProfile => {
                "only the 1920x1080 Remastered HUD is supported; no input was sent"
            }
            Self::BlankCapture => "the game window capture was blank; no input was sent",
            Self::NoDroneSelection => {
                "no drone selection was recognized; select 2-12 drones (or one drone) first"
            }
            Self::OccupiedNonDrone { .. } => {
                "the selection HUD has slots that are not drones (or a tooltip covers \
                 it); refusing to build"
            }
            Self::SingleWireframe => {
                "unexpected single wireframe; refusing to guess a drone selection"
            }
            Self::UncertainSingle => "the selected single unit is not a drone; refusing to build",
        }
    }
}

/// What the placement preview looks like near a target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Placement {
    /// Green preview: the build order may be issued.
    Valid { center: Point },
    /// Red preview: blocked terrain or an invalid position.
    Invalid { center: Point },
    /// No preview found.
    Absent,
    /// Both colours found; do not click.
    Ambiguous,
}

/// One wireframe slot's raw readout, exposed for diagnostics and tests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotReadout {
    pub slot: u8,
    pub center: Point,
    pub occupied: bool,
    pub drone: bool,
    pub agreement: f32,
    pub cells: usize,
}

/// Probes every wireframe slot of the selection HUD.
///
/// [`detect_selection`] is built on this, and the diagnostic example prints it,
/// so the calibration numbers can always be inspected.
pub fn read_slots(frame: &Frame) -> Vec<SlotReadout> {
    (0..SLOT_COUNT)
        .map(|slot| {
            let center = slot_center(slot);
            let occupied = slot_occupied(frame, slot);
            let (agreement, cells) = if occupied {
                best_mask_match(frame, center, DRONE_WIREFRAME_MASK)
            } else {
                (0.0, 0)
            };
            SlotReadout {
                slot,
                center,
                occupied,
                drone: agreement >= DRONE_MIN_AGREEMENT && cells >= DRONE_MIN_CELLS,
                agreement,
                cells,
            }
        })
        .collect()
}

/// Reads the selection HUD of a full client frame.
pub fn detect_selection(frame: &Frame) -> SelectionRead {
    if frame.width() != CLIENT_WIDTH || frame.height() != CLIENT_HEIGHT {
        return SelectionRead::Rejected(RejectReason::UnsupportedProfile);
    }
    if frame.is_blank() {
        return SelectionRead::Rejected(RejectReason::BlankCapture);
    }

    let occupied: Vec<SlotReadout> = read_slots(frame)
        .into_iter()
        .filter(|slot| slot.occupied)
        .collect();

    // The game always fills the group grid column-major, so a real group's
    // occupied slots are exactly `0..count`. Anything else cannot be a group.
    let contiguous = occupied
        .iter()
        .enumerate()
        .all(|(expected, slot)| slot.slot as usize == expected);

    if contiguous && occupied.len() >= 2 {
        for slot in &occupied {
            if !slot.drone {
                return SelectionRead::Rejected(RejectReason::OccupiedNonDrone { slot: slot.slot });
            }
        }
        return SelectionRead::Drones {
            count: occupied.len() as u8,
        };
    }

    // A single selected unit never uses the wireframe grid: the HUD shows the
    // large information-panel portrait, which must match the drone template.
    if single_portrait_is_drone(frame) {
        return SelectionRead::SingleDrone;
    }

    match occupied.first() {
        Some(slot) if occupied.len() == 1 && slot.drone => {
            SelectionRead::Rejected(RejectReason::SingleWireframe)
        }
        Some(slot) => SelectionRead::Rejected(RejectReason::OccupiedNonDrone { slot: slot.slot }),
        None => SelectionRead::Rejected(RejectReason::NoDroneSelection),
    }
}

/// Screen centre of a wireframe slot, column-major (top, bottom, next column).
pub fn slot_center(slot: u8) -> Point {
    let column = i32::from(slot / ROWS);
    let row = i32::from(slot % ROWS);
    Point::new(SLOT0.x + COL_STEP * column, SLOT0.y + ROW_STEP * row)
}

/// Number of wireframe slots in the group panel.
pub const SLOT_COUNT: u8 = COLUMNS * ROWS;

/// True when a selected-unit frame is drawn in this slot.
fn slot_occupied(frame: &Frame, slot: u8) -> bool {
    let center = slot_center(slot);
    let mut blue = 0usize;
    for y in (center.y - SLOT_HALF_Y)..=(center.y + SLOT_HALF_Y) {
        for x in (center.x - SLOT_HALF_X)..=(center.x + SLOT_HALF_X) {
            if frame
                .pixel_at_screen(Point::new(x, y))
                .is_some_and(Rgb::is_frame_blue)
            {
                blue += 1;
                if blue >= MIN_FRAME_BLUE_PIXELS {
                    return true;
                }
            }
        }
    }
    false
}

/// True when the single-unit information panel shows a drone portrait.
fn single_portrait_is_drone(frame: &Frame) -> bool {
    let (agreement, cells) = best_mask_match(frame, SINGLE_PORTRAIT, SINGLE_DRONE_PORTRAIT_MASK);
    agreement >= SINGLE_MIN_AGREEMENT && cells >= SINGLE_MIN_CELLS
}

/// Best (agreement, red cells) over the small alignment offsets.
fn best_mask_match(frame: &Frame, center: Point, template: [&str; 16]) -> (f32, usize) {
    let template = template_rows(&template);
    let mut best = (0.0f32, 0usize);
    for dy in ALIGN_OFFSETS {
        for dx in ALIGN_OFFSETS {
            let cells = red_cells(frame, center.offset(dx, dy));
            let red = cells.iter().filter(|cell| **cell).count();
            let agreement = agreement(&template, &cells);
            if agreement > best.0 {
                best = (agreement, red);
            }
        }
    }
    best
}

/// Flattens the readable string masks into a boolean template.
fn template_rows(rows: &[&str; 16]) -> Vec<bool> {
    let mut out = Vec::with_capacity(MASK_SIDE as usize * MASK_SIDE as usize);
    for row in rows {
        for cell in row.chars() {
            out.push(cell == '#');
        }
    }
    out
}

/// Red-body cells of a 16×16 cell grid centred on `center`, 4×4 pixels each.
fn red_cells(frame: &Frame, center: Point) -> Vec<bool> {
    let mut cells = Vec::with_capacity(MASK_SIDE as usize * MASK_SIDE as usize);
    let left = center.x - MASK_HALF;
    let top = center.y - MASK_HALF;
    for gy in 0..MASK_SIDE {
        for gx in 0..MASK_SIDE {
            let mut red = false;
            'cell: for dy in 0..MASK_CELL {
                for dx in 0..MASK_CELL {
                    let x = left + gx * MASK_CELL + dx;
                    let y = top + gy * MASK_CELL + dy;
                    if frame
                        .pixel_at_screen(Point::new(x, y))
                        .is_some_and(Rgb::is_body_red)
                    {
                        red = true;
                        break 'cell;
                    }
                }
            }
            cells.push(red);
        }
    }
    cells
}

/// Intersection over union of two equally sized boolean masks.
fn agreement(template: &[bool], cells: &[bool]) -> f32 {
    let mut both = 0usize;
    let mut either = 0usize;
    for (expected, actual) in template.iter().zip(cells.iter()) {
        if *expected && *actual {
            both += 1;
        }
        if *expected || *actual {
            either += 1;
        }
    }
    if either == 0 {
        1.0
    } else {
        both as f32 / either as f32
    }
}

/// Reads the placement preview near `target` (screen coordinates).
///
/// `footprint_px` is the expected building size on screen (144 for a 2×2 Creep
/// Colony, 216 for a 3×3 Spire), so both the search window and the accepted
/// square size scale with the selected building.
pub fn detect_placement(frame: &Frame, target: Point, footprint_px: i32) -> Placement {
    let search = PREVIEW_SEARCH_MIN.max(footprint_px);
    let window = Rect::new(target.x - search, target.y - search, search * 2, search * 2);
    let green = overlay_square(frame, window, Rgb::is_preview_green, footprint_px);
    let red = overlay_square(frame, window, Rgb::is_preview_red, footprint_px);
    match (green, red) {
        (Some(center), None) => Placement::Valid { center },
        (None, Some(center)) => Placement::Invalid { center },
        (Some(_), Some(_)) => Placement::Ambiguous,
        (None, None) => Placement::Absent,
    }
}

/// Finds a preview-sized square of `wanted` coloured pixels inside `window`.
fn overlay_square(
    frame: &Frame,
    window: Rect,
    wanted: fn(Rgb) -> bool,
    footprint_px: i32,
) -> Option<Point> {
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    let mut count = 0usize;
    for y in window.y..window.bottom() {
        for x in window.x..window.right() {
            if frame.pixel_at_screen(Point::new(x, y)).is_some_and(wanted) {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                count += 1;
            }
        }
    }
    if count == 0 {
        return None;
    }

    let width = max_x - min_x + 1;
    let height = max_y - min_y + 1;
    let side_min = footprint_px * PREVIEW_SIDE_MIN_NUM / PREVIEW_SIDE_DEN;
    let side_max = footprint_px * PREVIEW_SIDE_MAX_NUM / PREVIEW_SIDE_DEN;
    if !(side_min..=side_max).contains(&width) || !(side_min..=side_max).contains(&height) {
        return None;
    }
    if (width - height).abs() > width.max(height) / 5 {
        return None;
    }
    let fill = count as f32 / (width * height) as f32;
    if fill < PREVIEW_MIN_FILL {
        return None;
    }

    // A real preview is an overlay on terrain: the ring just outside the
    // square must not have the same colour, otherwise this is a naturally
    // coloured area (green grass, red tile) and not a preview.
    let ring = Rect::new(
        min_x - PREVIEW_RING,
        min_y - PREVIEW_RING,
        width + 2 * PREVIEW_RING,
        height + 2 * PREVIEW_RING,
    );
    let mut ring_count = 0usize;
    let mut ring_area = 0usize;
    for y in ring.y..ring.bottom() {
        for x in ring.x..ring.right() {
            if (min_x..=max_x).contains(&x) && (min_y..=max_y).contains(&y) {
                continue;
            }
            ring_area += 1;
            if frame.pixel_at_screen(Point::new(x, y)).is_some_and(wanted) {
                ring_count += 1;
            }
        }
    }
    if ring_area > 0 && ring_count as f32 / ring_area as f32 > PREVIEW_MAX_RING_FILL {
        return None;
    }

    Some(Point::new((min_x + max_x) / 2, (min_y + max_y) / 2))
}

/// Test-only frame painter and fixture loader.
///
/// Crate-internal so the row state-machine tests can drive the real detector
/// through synthetic captures without shipping full screenshots.
#[cfg(test)]
pub(crate) mod synthetic {
    use super::*;
    use crate::frame::Rgb;
    use std::path::Path;

    /// Bright selected-unit frame colour, taken from the live capture.
    pub(crate) const FRAME_BLUE: Rgb = Rgb::new(12, 73, 206);
    /// Dark slot background between the drone's legs.
    pub(crate) const SLOT_BG: Rgb = Rgb::new(0, 16, 51);
    /// Drone body colour (red-dominant natural palette).
    pub(crate) const DRONE_BODY: Rgb = Rgb::new(200, 24, 24);
    /// A non-drone portrait colour (blue/purple, like a Creep Colony).
    pub(crate) const NON_DRONE_BODY: Rgb = Rgb::new(40, 24, 120);
    /// Valid placement preview overlay.
    pub(crate) const PREVIEW_GREEN: Rgb = Rgb::new(30, 180, 30);
    /// Blocked placement preview overlay.
    pub(crate) const PREVIEW_RED: Rgb = Rgb::new(200, 30, 30);

    /// A selection crop (550×200 at `(600, 880)`) placed in a client frame.
    pub(crate) fn selection_fixture(name: &str) -> Frame {
        load_fixture(name, 600, 880)
    }

    /// A placement crop (430×300 at `(160, 200)`) placed in a client frame.
    pub(crate) fn placement_fixture(name: &str) -> Frame {
        load_fixture(name, 160, 200)
    }

    fn load_fixture(name: &str, x0: i32, y0: i32) -> Frame {
        let path = Path::new("tests/fixtures/remastered-1080").join(name);
        let crop = Frame::from_png(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
        blit(&crop, x0, y0)
    }

    /// Copies `src` into a blank client-sized frame at `(x0, y0)`.
    pub(crate) fn blit(src: &Frame, x0: i32, y0: i32) -> Frame {
        let mut frame = Frame::blank(CLIENT_WIDTH, CLIENT_HEIGHT);
        for y in 0..src.height() as i32 {
            for x in 0..src.width() as i32 {
                if let Some(pixel) = src.pixel(x, y) {
                    frame.set_pixel(x0 + x, y0 + y, pixel);
                }
            }
        }
        frame
    }

    /// Paints one occupied group-panel slot, drone or non-drone.
    pub(crate) fn paint_slot(frame: &mut Frame, slot: u8, drone: bool) {
        let center = slot_center(slot);
        for y in (center.y - SLOT_HALF_Y)..=(center.y + SLOT_HALF_Y) {
            for x in (center.x - SLOT_HALF_X)..=(center.x + SLOT_HALF_X) {
                let on_frame = (x - center.x).abs() >= SLOT_HALF_X - 4
                    || (y - center.y).abs() >= SLOT_HALF_Y - 4;
                frame.set_pixel(x, y, if on_frame { FRAME_BLUE } else { SLOT_BG });
            }
        }
        if drone {
            paint_mask(frame, center, DRONE_WIREFRAME_MASK, DRONE_BODY);
        } else {
            // A round, non-drone silhouette; the detector must reject it.
            for dy in -22..=22 {
                for dx in -22..=22 {
                    if dx * dx / 4 + dy * dy / 4 <= 100 {
                        frame.set_pixel(center.x + dx, center.y + dy, NON_DRONE_BODY);
                    }
                }
            }
        }
    }

    /// Paints the single-unit information panel portrait.
    pub(crate) fn paint_single(frame: &mut Frame, drone: bool) {
        let color = if drone { DRONE_BODY } else { NON_DRONE_BODY };
        paint_mask(frame, SINGLE_PORTRAIT, SINGLE_DRONE_PORTRAIT_MASK, color);
        if !drone {
            for dy in -34..=34 {
                for dx in -34..=34 {
                    if dx * dx + dy * dy <= 34 * 34 {
                        frame.set_pixel(SINGLE_PORTRAIT.x + dx, SINGLE_PORTRAIT.y + dy, color);
                    }
                }
            }
        }
    }

    /// Paints a placement preview square centred on `center`.
    pub(crate) fn paint_preview(frame: &mut Frame, center: Point, green: bool, footprint_px: i32) {
        let color = if green { PREVIEW_GREEN } else { PREVIEW_RED };
        let half = footprint_px / 2;
        for y in (center.y - half)..(center.y + half) {
            for x in (center.x - half)..(center.x + half) {
                frame.set_pixel(x, y, color);
            }
        }
    }

    fn paint_mask(frame: &mut Frame, center: Point, mask: [&str; MASK_SIDE as usize], color: Rgb) {
        for (gy, row) in mask.iter().enumerate() {
            for (gx, cell) in row.chars().enumerate() {
                if cell != '#' {
                    continue;
                }
                let left = center.x - MASK_HALF + gx as i32 * MASK_CELL;
                let top = center.y - MASK_HALF + gy as i32 * MASK_CELL;
                for dy in 0..MASK_CELL {
                    for dx in 0..MASK_CELL {
                        frame.set_pixel(left + dx, top + dy, color);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::synthetic::*;
    use super::*;

    #[test]
    fn embedded_masks_are_square() {
        for row in DRONE_WIREFRAME_MASK {
            assert_eq!(row.chars().count(), MASK_SIDE as usize, "{row}");
        }
        for row in SINGLE_DRONE_PORTRAIT_MASK {
            assert_eq!(row.chars().count(), MASK_SIDE as usize, "{row}");
        }
    }

    #[test]
    fn column_major_slot_centres_match_the_hud() {
        assert_eq!(slot_center(0), Point::new(653, 930));
        assert_eq!(slot_center(1), Point::new(653, 1013));
        assert_eq!(slot_center(2), Point::new(734, 930));
        assert_eq!(slot_center(11), Point::new(1058, 1013));
    }

    #[test]
    fn real_captures_detect_two_to_five_drones() {
        for (name, count) in [
            ("drones-2.png", 2),
            ("drones-3.png", 3),
            ("drones-4.png", 4),
            ("drones-5.png", 5),
        ] {
            let frame = selection_fixture(name);
            assert_eq!(
                detect_selection(&frame),
                SelectionRead::Drones { count },
                "{name}"
            );
        }
    }

    #[test]
    fn a_tooltip_covering_the_hud_is_rejected() {
        // Real capture of a 5-drone selection where an in-game hover tooltip
        // covers the fifth wireframe; the detector must fail closed instead of
        // counting only the four visible portraits.
        let frame = selection_fixture("drones-5-tooltip.png");
        assert!(!matches!(
            detect_selection(&frame),
            SelectionRead::Drones { .. }
        ));
        assert_ne!(detect_selection(&frame), SelectionRead::SingleDrone);
    }

    #[test]
    fn a_single_drone_information_panel_is_recognized() {
        let frame = selection_fixture("drone-single.png");
        assert_eq!(detect_selection(&frame), SelectionRead::SingleDrone);
    }

    #[test]
    fn a_single_non_drone_is_rejected() {
        let frame = selection_fixture("not-drone-colony.png");
        assert!(matches!(
            detect_selection(&frame),
            SelectionRead::Rejected(_)
        ));
    }

    #[test]
    fn synthetic_six_to_twelve_drones_are_counted() {
        for count in 6..=MAX_DRONES {
            let mut frame = Frame::blank(CLIENT_WIDTH, CLIENT_HEIGHT);
            for slot in 0..count {
                paint_slot(&mut frame, slot, true);
            }
            assert_eq!(
                detect_selection(&frame),
                SelectionRead::Drones { count },
                "count {count}"
            );
        }
    }

    #[test]
    fn a_mixed_selection_is_rejected() {
        let mut frame = Frame::blank(CLIENT_WIDTH, CLIENT_HEIGHT);
        paint_slot(&mut frame, 0, true);
        paint_slot(&mut frame, 1, false);
        assert_eq!(
            detect_selection(&frame),
            SelectionRead::Rejected(RejectReason::OccupiedNonDrone { slot: 1 })
        );
    }

    #[test]
    fn an_empty_or_black_capture_is_rejected() {
        assert_eq!(
            detect_selection(&Frame::blank(CLIENT_WIDTH, CLIENT_HEIGHT)),
            SelectionRead::Rejected(RejectReason::BlankCapture)
        );
    }

    #[test]
    fn an_unsupported_profile_is_rejected() {
        assert_eq!(
            detect_selection(&Frame::blank(1280, 720)),
            SelectionRead::Rejected(RejectReason::UnsupportedProfile)
        );
    }

    #[test]
    fn real_green_placement_previews_are_valid() {
        for (name, target, center) in [
            (
                "preview-first.png",
                Point::new(320, 340),
                Point::new(310, 357),
            ),
            (
                "preview-second.png",
                Point::new(464, 340),
                Point::new(454, 357),
            ),
        ] {
            let frame = placement_fixture(name);
            match detect_placement(&frame, target, FOOTPRINT) {
                Placement::Valid { center: found } => {
                    assert!(
                        (found.x - center.x).abs() <= 2 && (found.y - center.y).abs() <= 2,
                        "{name}: got {found:?}, expected {center:?}"
                    );
                }
                other => panic!("{name}: expected a valid preview, got {other:?}"),
            }
        }
    }

    #[test]
    fn no_preview_is_absent() {
        let frame = placement_fixture("no-preview.png");
        assert_eq!(
            detect_placement(&frame, Point::new(320, 340), FOOTPRINT),
            Placement::Absent
        );
    }

    #[test]
    fn a_placement_preview_is_matched_against_the_selected_footprint() {
        // A 3x3 Spire preview is 216 px; the 2x2 colony detector must not accept
        // it, and the Spire detector must not accept a 144 px colony preview.
        let target = Point::new(600, 400);
        let mut spire = Frame::blank(CLIENT_WIDTH, CLIENT_HEIGHT);
        paint_preview(&mut spire, target, true, SPIRE_FOOTPRINT);
        assert!(matches!(
            detect_placement(&spire, target, SPIRE_FOOTPRINT),
            Placement::Valid { .. }
        ));
        assert!(!matches!(
            detect_placement(&spire, target, FOOTPRINT),
            Placement::Valid { .. }
        ));

        let mut colony = Frame::blank(CLIENT_WIDTH, CLIENT_HEIGHT);
        paint_preview(&mut colony, target, true, FOOTPRINT);
        assert!(matches!(
            detect_placement(&colony, target, FOOTPRINT),
            Placement::Valid { .. }
        ));
        assert!(!matches!(
            detect_placement(&colony, target, SPIRE_FOOTPRINT),
            Placement::Valid { .. }
        ));
    }

    #[test]
    fn a_snapped_preview_is_still_found_for_a_large_footprint() {
        // The game snaps the preview to the tile grid, so it can sit off centre.
        let target = Point::new(700, 430);
        let snapped = Point::new(700 - 40, 430 + 30);
        let mut frame = Frame::blank(CLIENT_WIDTH, CLIENT_HEIGHT);
        paint_preview(&mut frame, snapped, true, SPIRE_FOOTPRINT);
        match detect_placement(&frame, target, SPIRE_FOOTPRINT) {
            Placement::Valid { center } => assert!(
                (center.x - snapped.x).abs() <= 2 && (center.y - snapped.y).abs() <= 2,
                "got {center:?}, expected {snapped:?}"
            ),
            other => panic!("expected the snapped Spire preview, got {other:?}"),
        }
    }

    #[test]
    fn a_red_preview_is_reported_as_invalid() {
        let target = Point::new(320, 340);
        let mut frame = Frame::blank(CLIENT_WIDTH, CLIENT_HEIGHT);
        paint_preview(&mut frame, target, false, FOOTPRINT);
        assert!(matches!(
            detect_placement(&frame, target, FOOTPRINT),
            Placement::Invalid { .. }
        ));
    }

    #[test]
    fn a_green_preview_clears_after_placement() {
        let target = Point::new(320, 340);
        let mut frame = Frame::blank(CLIENT_WIDTH, CLIENT_HEIGHT);
        paint_preview(&mut frame, target, true, FOOTPRINT);
        assert!(matches!(
            detect_placement(&frame, target, FOOTPRINT),
            Placement::Valid { .. }
        ));
        let cleared = Frame::blank(CLIENT_WIDTH, CLIENT_HEIGHT);
        assert_eq!(
            detect_placement(&cleared, target, FOOTPRINT),
            Placement::Absent
        );
    }
}
