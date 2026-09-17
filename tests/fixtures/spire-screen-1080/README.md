# Spire-screen calibration fixtures (1920×1080 live capture)

`screen.png` is a real StarCraft Remastered client screenshot captured from the
user's running game with the user's explicit permission. It is stored exactly as
captured: no pixel was edited, repainted or synthesised, and the whole scene is
kept (not only the Spires) so the negatives — Spore/Sunken colonies, drones,
terrain, HUD and minimap — are real too. No desktop content outside the game
client is included.

- Source: user-authorized live capture, StarCraft Remastered, 1920×1080 client.
- SHA-256 of `screen.png`:
  `89298f84be160e962caa56d1b4b448940b60dc3ef561e065e342a63dd345cddc`
- Format: 1920×1080, 8-bit RGBA, non-interlaced.

## Visible Spire crowns

Four Spire crowns are visible (approximate click centres):

| # | Approximate centre | Note |
|---|--------------------|------|
| 1 | (790, 150) | top centre |
| 2 | (790, 365) | below #1, health bar crosses the crown |
| 3 | (1365, 365) | right |
| 4 | (790, 655) | lower; base partly behind the HUD growth, crown in the safe play field |

The other Zerg structures in the scene (Spore/Sunken colonies, the cluster near
the bottom HUD) are the negative distractors.

## Derived detector assets

Both files are byte-for-byte derived from `screen.png` and are embedded into the
binary with `include_bytes!` (see `src/spire_vision.rs`). The crate ships no
hand-drawn template.

- `crown-template-96x96.gray` — 96×96 grayscale crop at `(742, 110)`, i.e. the
  crown of #1 (the training building). Grayscale is
  `(299·r + 587·g + 114·b) / 1000`.
- `spire-portrait-104x140.mask` — 104×140 silhouette of the single-unit
  selection-panel portrait at `(636, 874)`; a byte is `255` where
  `max(r, g, b) > 32` and `0` otherwise. Used only to verify a click selected a
  Spire.

`src/spire_vision.rs` tests regenerate both assets from `screen.png` at run time
and compare, so the committed bytes cannot silently drift from the screenshot.

## Scope caveat

This is a **single calibration scene**, not evidence of generalisation. The
crown template was trained on building #1 and is tested separately on #2, #3 and
#4; all four are found in this one scene, but no second scene, map, camera
position or player colour has been captured. The detector is deliberately
limited to the 1920×1080 Remastered profile and fails closed on anything else.
The portrait verification is calibrated against this scene's team colour, so a
different player colour must be re-checked before trusting it.
