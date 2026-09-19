# Stargates clipped by the bottom HUD (1920×1080)

Read-only `PrintWindow` capture supplied during live debugging. The in-game HUD
covers the lower hull of the bottom three Stargates, so the lower-hull-only
detector found the four gates above them but could not produce a safe click for
the bottom row.

The dual-hull detector should return these seven clickable centres, in screen
order:

| # | centre |
|---|---|
| 1 | `(1128, 278)` |
| 2 | `(624, 566)` |
| 3 | `(912, 566)` |
| 4 | `(1344, 566)` |
| 5 | `(704, 666)` |
| 6 | `(992, 666)` |
| 7 | `(1280, 666)` |

The bottom three centres come from the visible upper hull. Selection-panel
verification is still required before the action sends `A`.

`screen.png` SHA256:
`6027147659a36626a1e98556bde1206e7368669554da6d9da886fa26c9e94c6d`
