//! A decoded client-area frame and the PNG loader used by the F6 detector.
//!
//! Everything here is host independent: the Windows adapter fills a [`Frame`]
//! from a `PrintWindow` capture, tests build one from a fixture PNG or
//! synthetically. Coordinates are *physical screen pixels* on Windows; the
//! frame remembers where its own `(0, 0)` sits on screen so that the detector
//! can translate a cursor position into frame-local coordinates without
//! assuming the client sits at the desktop origin.

use std::io::BufReader;
use std::path::Path;

/// A position in physical screen pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    pub const fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// This point translated by `offset`.
    pub const fn offset(self, dx: i32, dy: i32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
        }
    }
}

/// One 8-bit RGB sample.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    fn r_i(self) -> i32 {
        i32::from(self.r)
    }

    fn g_i(self) -> i32 {
        i32::from(self.g)
    }

    fn b_i(self) -> i32 {
        i32::from(self.b)
    }

    /// Bright blue pixels of the selected-unit wireframe / tooltip frame.
    pub fn is_frame_blue(self) -> bool {
        self.b_i() > 120 && self.b_i() - self.r_i() > 40 && self.b_i() - self.g_i() > 25
    }

    /// Reddish pixels of a Zerg unit's natural palette (the drone body).
    pub fn is_body_red(self) -> bool {
        self.r_i() > 90 && self.r_i() - self.b_i() > 30 && self.r_i() - self.g_i() > 15
    }

    /// Saturated green of a valid placement preview overlay.
    pub fn is_preview_green(self) -> bool {
        self.g_i() > 90 && self.g_i() - self.r_i() > 30 && self.g_i() - self.b_i() > 30
    }

    /// Saturated red of a blocked/invalid placement preview overlay.
    pub fn is_preview_red(self) -> bool {
        self.r_i() > 90 && self.r_i() - self.g_i() > 40 && self.r_i() - self.b_i() > 40
    }
}

/// Why a PNG could not be turned into a [`Frame`].
#[derive(Debug)]
pub enum FrameError {
    Io(String),
    Decode(String),
    Unsupported(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(detail) => write!(f, "could not read the image: {detail}"),
            Self::Decode(detail) => write!(f, "could not decode the PNG: {detail}"),
            Self::Unsupported(detail) => write!(f, "unsupported image: {detail}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// A decoded RGBA8 screenshot of the game's client area.
#[derive(Clone, Debug)]
pub struct Frame {
    width: u32,
    height: u32,
    origin: Point,
    rgba: Vec<u8>,
}

impl Frame {
    /// Wraps `rgba` (length `width * height * 4`). Returns `None` on a size
    /// mismatch instead of panicking on later indexing.
    pub fn new(width: u32, height: u32, origin: Point, rgba: Vec<u8>) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        if rgba.len() != expected {
            return None;
        }
        Some(Self {
            width,
            height,
            origin,
            rgba,
        })
    }

    /// An opaque black frame, used by tests and as a blank-capture fallback.
    pub fn blank(width: u32, height: u32) -> Self {
        Self::new(
            width,
            height,
            Point::new(0, 0),
            vec![0; width as usize * height as usize * 4],
        )
        .expect("non-zero blank frame")
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Screen position of this frame's top-left pixel.
    pub fn origin(&self) -> Point {
        self.origin
    }

    /// Reads one pixel, or `None` outside the frame.
    pub fn pixel(&self, x: i32, y: i32) -> Option<Rgb> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return None;
        }
        let index = ((y as u32 * self.width + x as u32) * 4) as usize;
        Some(Rgb::new(
            self.rgba[index],
            self.rgba[index + 1],
            self.rgba[index + 2],
        ))
    }

    /// Same as [`Self::pixel`] but for a point in screen coordinates.
    pub fn pixel_at_screen(&self, point: Point) -> Option<Rgb> {
        self.pixel(
            point.x.checked_sub(self.origin.x)?,
            point.y.checked_sub(self.origin.y)?,
        )
    }

    /// Whether a positive screen rectangle fits completely inside this frame.
    /// Widened arithmetic also rejects overflowing/untrusted capture requests.
    pub fn contains_rect(&self, rect: Rect) -> bool {
        let x = i64::from(rect.x) - i64::from(self.origin.x);
        let y = i64::from(rect.y) - i64::from(self.origin.y);
        rect.w > 0
            && rect.h > 0
            && x >= 0
            && y >= 0
            && x + i64::from(rect.w) <= i64::from(self.width)
            && y + i64::from(rect.h) <= i64::from(self.height)
    }

    /// Copies just the requested screen rectangle, preserving its origin and
    /// alpha channel. Row copies avoid per-pixel conversion in capture fallbacks.
    pub fn crop(&self, rect: Rect) -> Option<Self> {
        if !self.contains_rect(rect) {
            return None;
        }
        let x = (i64::from(rect.x) - i64::from(self.origin.x)) as usize;
        let y = (i64::from(rect.y) - i64::from(self.origin.y)) as usize;
        let row_bytes = rect.w as usize * 4;
        let stride = self.width as usize * 4;
        let mut rgba = Vec::with_capacity(row_bytes * rect.h as usize);
        for row in y..y + rect.h as usize {
            let start = row * stride + x * 4;
            rgba.extend_from_slice(&self.rgba[start..start + row_bytes]);
        }
        Self::new(
            rect.w as u32,
            rect.h as u32,
            Point::new(rect.x, rect.y),
            rgba,
        )
    }

    /// Raw RGBA bytes, row major, top to bottom.
    pub(crate) fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// Writes one pixel. Test-only: the synthetic frames the detector and row
    /// tests build use it; the live capture path never mutates a frame.
    #[cfg(test)]
    pub(crate) fn set_pixel(&mut self, x: i32, y: i32, color: Rgb) {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return;
        }
        let index = ((y as u32 * self.width + x as u32) * 4) as usize;
        self.rgba[index] = color.r;
        self.rgba[index + 1] = color.g;
        self.rgba[index + 2] = color.b;
        self.rgba[index + 3] = 255;
    }

    /// True when the capture is (almost) completely black — which is what
    /// `PrintWindow` returns for a minimized or protected window.
    pub fn is_blank(&self) -> bool {
        self.rgba()
            .chunks_exact(4)
            .step_by(37)
            .all(|px| (i32::from(px[0]) + i32::from(px[1]) + i32::from(px[2])) / 3 < 6)
    }

    /// Decodes a PNG into a frame whose origin is `(0, 0)`.
    ///
    /// Only used by the diagnostic CLI and the tests; the live path fills a
    /// frame from GDI directly.
    pub fn from_png(path: &Path) -> Result<Self, FrameError> {
        let file = BufReader::new(
            std::fs::File::open(path)
                .map_err(|error| FrameError::Io(format!("{}: {error}", path.display())))?,
        );
        let mut decoder = png::Decoder::new(file);
        decoder.set_transformations(png::Transformations::normalize_to_color8());
        let mut reader = decoder
            .read_info()
            .map_err(|error| FrameError::Decode(error.to_string()))?;
        let info = reader.info();
        if info.width == 0 || info.height == 0 {
            return Err(FrameError::Unsupported("zero-sized image".to_owned()));
        }
        if reader.info().animation_control.is_some() {
            return Err(FrameError::Unsupported(
                "animated PNG frames are not supported".to_owned(),
            ));
        }
        let mut buffer =
            vec![
                0u8;
                reader
                    .output_buffer_size()
                    .ok_or_else(|| FrameError::Unsupported("image too large".to_owned()))?
            ];
        let output = reader
            .next_frame(&mut buffer)
            .map_err(|error| FrameError::Decode(error.to_string()))?;
        let rgba = expand_to_rgba(&buffer, output.color_type, output.line_size)?;
        Frame::new(output.width, output.height, Point::new(0, 0), rgba)
            .ok_or_else(|| FrameError::Unsupported("unexpected pixel buffer size".to_owned()))
    }
}

/// Pads `gray`/`rgb` rows to RGBA8.
fn expand_to_rgba(
    buffer: &[u8],
    color_type: png::ColorType,
    line_size: usize,
) -> Result<Vec<u8>, FrameError> {
    let samples = color_type.samples();
    if samples == 4 {
        return Ok(buffer.to_vec());
    }
    if !matches!(
        color_type,
        png::ColorType::Grayscale | png::ColorType::GrayscaleAlpha | png::ColorType::Rgb
    ) {
        return Err(FrameError::Unsupported(format!(
            "colour type {color_type:?} is not supported"
        )));
    }
    debug_assert_eq!(line_size % samples.max(1), 0);
    let mut out = Vec::with_capacity(buffer.len() / samples * 4);
    for px in buffer.chunks_exact(samples) {
        match samples {
            1 => out.extend_from_slice(&[px[0], px[0], px[0], 255]),
            2 => out.extend_from_slice(&[px[0], px[0], px[0], px[1]]),
            3 => out.extend_from_slice(&[px[0], px[1], px[2], 255]),
            _ => unreachable!("samples checked above"),
        }
    }
    Ok(out)
}

/// An axis aligned rectangle in screen coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_pixels_are_translated_by_the_frame_origin() {
        // The live capture sits wherever the client is on the desktop, so the
        // detector must never assume (0, 0).
        let mut frame = Frame::blank(64, 48);
        let placed = Frame::new(64, 48, Point::new(100, 200), frame.rgba().to_vec()).unwrap();
        assert_eq!(placed.origin(), Point::new(100, 200));
        assert_eq!(
            placed.pixel_at_screen(Point::new(100, 200)),
            Some(Rgb::new(0, 0, 0))
        );
        assert_eq!(placed.pixel_at_screen(Point::new(99, 200)), None);
        frame.set_pixel(3, 4, Rgb::new(10, 20, 30));
        let placed = Frame::new(64, 48, Point::new(100, 200), frame.rgba().to_vec()).unwrap();
        assert_eq!(
            placed.pixel_at_screen(Point::new(103, 204)),
            Some(Rgb::new(10, 20, 30))
        );
    }

    #[test]
    fn cropping_copies_rows_preserves_alpha_and_screen_coordinates() {
        let bytes: Vec<u8> = (0..48).collect();
        let frame = Frame::new(4, 3, Point::new(100, 200), bytes.clone()).unwrap();
        let crop = frame.crop(Rect::new(101, 201, 2, 2)).unwrap();
        assert_eq!(crop.origin(), Point::new(101, 201));
        assert_eq!(crop.rgba(), [&bytes[20..28], &bytes[36..44]].concat());
        assert_eq!(
            crop.pixel_at_screen(Point::new(101, 201)),
            frame.pixel(1, 1)
        );
        assert_eq!(
            crop.crop(Rect::new(102, 202, 1, 1)).unwrap().rgba(),
            &bytes[40..44]
        );
    }

    #[test]
    fn cropping_rejects_empty_outside_and_overflowing_rectangles() {
        let frame = Frame::blank(64, 48);
        for rect in [
            Rect::new(0, 0, 0, 1),
            Rect::new(-1, 0, 4, 4),
            Rect::new(60, 0, 5, 1),
            Rect::new(0, 47, 1, 2),
            Rect::new(i32::MAX, 0, i32::MAX, 1),
            Rect::new(i32::MIN, 0, i32::MAX, 1),
        ] {
            assert!(frame.crop(rect).is_none(), "{rect:?}");
        }
    }

    #[test]
    fn a_mismatched_buffer_is_refused_instead_of_panicking() {
        assert!(Frame::new(10, 10, Point::new(0, 0), vec![0; 399]).is_none());
        assert!(Frame::new(0, 10, Point::new(0, 0), Vec::new()).is_none());
    }
}
