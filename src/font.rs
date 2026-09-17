//! Korean font discovery helpers.
//!
//! The GUI prefers Korean labels, but only when a font that actually contains
//! Hangul glyphs could be loaded; otherwise it falls back to English instead of
//! showing empty boxes.

/// Font files we try, in order, for Hangul coverage.
///
/// Only single-face `.ttf` files: `.ttc` collections (gulim.ttc, batang.ttc)
/// would need a face index and are skipped on purpose.
pub const KOREAN_FONT_FILES: [&str; 4] = [
    "malgun.ttf",      // Malgun Gothic, shipped with every Windows since Vista
    "malgunsl.ttf",    // Malgun Gothic Semilight
    "NanumGothic.ttf", // common third-party install
    "NotoSansKR-Regular.ttf",
];

/// Minimal sanity check for a single-face sfnt font.
///
/// Guards the font parser against non-font files (an HTML error page, a `.ttc`
/// collection, a half written download). Accepts TrueType (`0x00010000`), CFF
/// (`OTTO`) and the legacy Apple (`true`) signatures; `ttcf` collections are
/// rejected because a face index is needed to load them.
pub fn looks_like_sfnt(bytes: &[u8]) -> bool {
    matches!(
        bytes.first_chunk::<4>(),
        Some(b"\x00\x01\x00\x00" | b"OTTO" | b"true")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_single_face_font_signatures() {
        assert!(looks_like_sfnt(b"\x00\x01\x00\x00rest"));
        assert!(looks_like_sfnt(b"OTTOrest"));
        assert!(looks_like_sfnt(b"truerest"));
    }

    #[test]
    fn rejects_collections_and_non_fonts() {
        assert!(!looks_like_sfnt(b"ttcfrest"));
        assert!(!looks_like_sfnt(b"<!DOCTYPE html>"));
        assert!(!looks_like_sfnt(b""));
        assert!(!looks_like_sfnt(b"\x00\x01\x00"));
    }

    #[test]
    fn candidates_are_ttf_only() {
        for file in KOREAN_FONT_FILES {
            assert!(file.ends_with(".ttf"), "{file} is not a .ttf");
        }
    }
}
