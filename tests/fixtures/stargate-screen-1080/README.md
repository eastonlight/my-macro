# Stargate-screen calibration fixture (1920×1080 live capture)

`screen.png` is a real StarCraft Remastered client screenshot from the user's
running game, captured with the user's explicit permission, stored exactly as
captured (whole scene kept, so the negatives are real). Source:
`stargate-reference.png`, provided read-only to this worktree.

- SHA-256 of `screen.png`:
  `472225b8f65e3d7ceed95550b6317458e0e0df278c9f6dc789c928bc3f7db612`
- Format: 1920×1080, 8-bit RGBA, non-interlaced, client origin (0, 0).

## Visible Stargates

Seven red-team Stargates are detected, including the one whose upper hull is
clipped by the top edge. The lower-hull template remains fully visible there.

| # | Click centre |
|---|--------------|
| 1 | (1128, 132) |
| 2 | (768, 420)  |
| 3 | (1056, 420) |
| 4 | (1488, 420) |
| 5 | (912, 636)  |
| 6 | (1200, 636) |
| 7 | (1488, 636) |

Negatives in the scene: the Pylon, minerals, a Probe and the HUD.

## Derived detector assets

All assets are derived from `screen.png`. Active templates and the portrait mask
are embedded with `include_bytes!` in `src/stargate_vision.rs`; older crops remain
as provenance only.

- `stargate-template-128x128.gray` — original 128×128 grayscale lower-hull
  calibration crop at `(704, 356)`, retained as source evidence.
- `stargate-template-96x96.gray` — previous aggressive lower-hull core at
  `(720, 372)`, retained as calibration evidence.
- `stargate-template-64x64.gray` — very aggressive lower-hull core at
  `(736, 388)` used by the detector so more-than-half edge-clipped gates can
  remain candidates; luma is `(299·r + 587·g + 114·b) / 1000`.
- `stargate-upper-template-64x64.gray` — upper-hull core at `(672, 288)`
  used when the lower hull is hidden by the bottom HUD. Click offset `(32, 16)`
  lands higher on the hull to clear the conservative HUD skyline.
- `stargate-top-edge-40x32.gray` — bottom-left part of the 64×64 lower-hull
  template (local x=0..40, y=32..64; scene crop `(736, 420)`). Searched only
  near the top edge to recover gates whose lower-hull centre is clipped.
- `stargate-left-edge-48x64.gray` — right-hand fin at `(808, 380)`, searched
  only near the left edge. Click offset `(24, 32)` gives a point `(64, -8)`
  from the standard lower-hull centre, clear of the edge-scroll zone.
- `stargate-portrait-160x140.mask` — 160×140 silhouette of the selected-unit
  panel portrait at `(608, 874)`; `255` where `max(r, g, b) > 32`.

## Scope caveat

Single calibration scene is not evidence of map/skin/team generalisation.
All hull fragments share calibrated building anchors and use NCC >= 0.75;
selection-panel verification remains the final gate before `A`.
