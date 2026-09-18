# Stargate-screen calibration fixture (1920×1080 live capture)

`screen.png` is a real StarCraft Remastered client screenshot from the user's
running game, captured with the user's explicit permission, stored exactly as
captured (whole scene kept, so the negatives are real). Source:
`stargate-reference.png`, provided read-only to this worktree.

- SHA-256 of `screen.png`:
  `472225b8f65e3d7ceed95550b6317458e0e0df278c9f6dc789c928bc3f7db612`
- Format: 1920×1080, 8-bit RGBA, non-interlaced, client origin (0, 0).

## Visible Stargates

Six fully visible red-team Stargates (click centres) plus one top-clipped
Stargate at the top edge, which is deliberately excluded by the profile's safe
viewport (`y >= 230`) so its lower hull can never be clicked twice.

| # | Click centre |
|---|--------------|
| 1 | (712, 328)  |
| 2 | (1000, 328) |
| 3 | (1432, 328) |
| 4 | (856, 544)  |
| 5 | (1144, 544) |
| 6 | (1432, 544) |

Negatives in the scene: the clipped Stargate, a Pylon, minerals, a Probe and
the HUD.

## Derived detector assets

Both files are byte-for-byte derived from `screen.png` and embedded with
`include_bytes!` in `src/stargate_vision.rs`.

- `stargate-template-128x128.gray` — 128×128 grayscale upper-hull crop at
  `(648, 264)`; luma is `(299·r + 587·g + 114·b) / 1000`.
- `stargate-portrait-160x140.mask` — 160×140 silhouette of the selected-unit
  panel portrait at `(608, 874)`; `255` where `max(r, g, b) > 32`.

## Scope caveat

Single calibration scene, not evidence of map/skin/team generalisation, and
not live-verified. A Stargate sprite is two hulls; the template matches the
upper hull and NMS keeps one detection per building.
