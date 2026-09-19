# spire-dense-1080 — dense Spire scene (real capture)

Real StarCraft: Remastered client capture, **1920×1080**, screen origin `(0, 0)`.
The game window was **not** in the foreground: the parent captured the client with
`PrintWindow` in the background, so no window activation, mouse movement or input
was involved. The user explicitly allowed using the current game state for testing.

- `screen.png` — the raw capture, unmodified.
- `annotated-detections.png` — diagnostic overlay produced by the vision test with
  `OH_MY_MACRO_WRITE_ANNOTATIONS=1`. It is a *report artifact*, not test input.

## Why this fixture exists

The single-scene calibration fixture (`../spire-screen-1080`) only holds four
Spires, which hid a real defect: with many Spires the candidate shortlist was
sorted and truncated globally, so several correctly-matched crowns were dropped
before scoring, and the search viewport's `y = 96` floor discarded crowns that sit
higher on screen. This scene reproduces both.

## Crowns in this scene

| Column (x) | Crown centres (y) | Status |
| --- | --- | --- |
| 574 | 156, 299, 443, 587 | detected |
| 718 | 84, 227, 371, 516 | detected |
| 862 | 84, 228, 372, 515 | detected |
| 1006 | 227, 372, 515 | detected |

Detected: **17** clickable Spires: 15 fully visible crowns plus two top-clipped
bodies (see `src/spire_vision.rs::DENSE_EXPECTED` and `DENSE_TOP_CLIPPED`).

Edge cases and deliberate exclusions:

- `(574, 74)` and `(1006, 74)` — their crown centres are above the client, but
  the calibrated top-band fragment clicks an opaque part of each visible body.
- `(862, ~803)` — crown whose visible part is occluded by the unit wireframe console;
  the centre is inside the HUD contour and remains excluded.
- `x ≈ 1150, y ≈ 250` — Greater Spire, a different building; the template must not
  match it.
## Calibration scope (honest limits)

This is **one scene at one camera position** with one team colour. It proves the
detector can separate many same-looking buildings on this terrain and that it does
not fire on this scene's distractors; it does **not** prove accuracy on other maps,
zooms, team colours, skins, animations or partial occlusion. The code fails closed
(no click, no `A`) whenever a target cannot be verified against the selection panel.

## Playfield boundary evidence

The parent re-measured the bottom HUD contour on this capture to check
`src/play_area.rs` (zoom crops, screen pixels):

- `x < 120`: decorative horn topmost pixel ≈ 645 (guard 640, only reachable by the
  Spire action, not by row builds whose usable `x` starts at 152).
- `120..540`: minimap console horizon ≈ 690–717 (guard 670).
- `540..660`: ridge ≈ 710–750 while the ramp interpolates 670→770, so the ramp
  stays above the real contour.
- `660..1260`: central console ridge ≈ 782–800 (guard 770).
- `1340..1840`: command-card ridge ≈ 715–740 (guard 710).

Margins are therefore positive everywhere; the tightest is the top-left decoration,
which row builds cannot reach.
