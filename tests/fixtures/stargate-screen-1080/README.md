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

All files are byte-for-byte derived from `screen.png` and embedded with
`include_bytes!` in `src/stargate_vision.rs`.

- `stargate-template-128x128.gray` — original 128×128 grayscale lower-hull
  calibration crop at `(704, 356)`, retained as source evidence.
- `stargate-template-96x96.gray` — previous aggressive lower-hull core at
  `(720, 372)`, retained as calibration evidence.
- `stargate-template-64x64.gray` — very aggressive lower-hull core at
  `(736, 388)` used by the detector so more-than-half edge-clipped gates can
  remain candidates; luma is `(299·r + 587·g + 114·b) / 1000`.
- `stargate-portrait-160x140.mask` — 160×140 silhouette of the selected-unit
  panel portrait at `(608, 874)`; `255` where `max(r, g, b) > 32`.

## Scope caveat

Single calibration scene, not evidence of map/skin/team generalisation, and
not live-verified. A Stargate sprite is two hulls; the template matches the
lower hull and NMS keeps one detection per building.
