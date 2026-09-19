# Spire clipped-edge calibration fixture (1920×1080 live capture)

`screen.png` is a real StarCraft Remastered client screenshot captured from the
user's running game with the user's explicit permission. It is stored exactly as
captured: no pixel was edited, repainted or synthesised, and the whole scene is
kept so the negatives (creep, terrain, HUD, resource bar) are real too. No
desktop content outside the game client is included.

- Source: user-authorized live capture, StarCraft Remastered, 1920×1080 client.
- SHA-256 of `screen.png`:
  `bc4cc160e19501f89d1c063f013cdc9037d5cb9060c15ce21b62ecc4f618c71a`
- Format: 1920×1080, 8-bit RGBA, non-interlaced.

## What the scene pins down

`detect_spires` reports the 18 actionable clicks below (exact list:
`the_clipped_fixture_yields_every_visible_spire`).

| click `y` | click `x` | reached by |
|-----------|-----------|------------|
| 40 | 826, 970, 1114, 1258, 1402 | preferred top-band body fragment |
| 265 | 34 | left-edge crown fragment |
| 265/266 | 826, 970, 1258, 1402 | primary crown template |
| 409/410 | 682, 826, 970, 1114, 1258 | primary crown template |
| 625/626 | 538, 1114, 1258 | primary crown template |

- The five top-row Spires overlap the row underneath. Controlled live probes
  showed that clicking their detected crown coordinates around `y = 122` selects
  the next Spire below, while clicking the opaque upper body at `y = 40` selects
  and verifies the intended top-row Spire. The body fragment therefore has
  priority and suppresses the overlapping crown detections.
- The top-band pass also recovers the Spire at `x = 970`, whose crown template
  does not meet the primary pass threshold.
- The left-edge Spire's crown centre is `(34, 265)`: its crown's left 14 columns
  are outside the client, so the primary 96×96 template can never be placed in
  frame (best in-frame NCC `0.31`). The crown's right 80 columns are fully visible
  and score `0.85`.
- With these fragments, terrain, resource-bar artwork and the fully visible
  crowns of this scene stay at or below `0.24`.

## Derived detector assets

Both files are byte-for-byte derived from committed screenshots, are embedded
with `include_bytes!` and are recomputed by tests in `src/spire_vision.rs` at run
time, so the committed bytes cannot silently drift.

- `spire-left-crown-80x96.gray` — columns `[16..96)` of
  `tests/fixtures/spire-screen-1080/crown-template-96x96.gray`, i.e. exactly the
  part of a calibrated crown that survives a left-edge clip. Used by
  `LEFT_EDGE_PROFILE` with a `(48 - 16, 48)` click offset, so the click stays on
  the crown centre instead of drifting to the right.
- `spire-top-band-64x64.gray` — grayscale crop of this fixture's own visible
  Spire body at `(794, 144)` (`TOP_BAND_CROP`). Matching it in the top band
  produces the safe upper-body click `(x, 40)`, mapped to the overlapping crown
  anchor `(x, 122)` for deduplication. Live probes confirmed the body click selects
  the intended top-row Spire.

## Scope caveat

Single-scene calibration, not evidence of generalisation: both fragments are
calibrated on this scene's team colour and skin. Candidate clicks are still
verified against the Spire selection portrait before `A`, so a fragment that
matches in the wrong place fails closed instead of upgrading something by
accident.
