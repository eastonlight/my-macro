# Eleven Stargates, including two clipped at the top

Read-only PrintWindow capture of the user's 1920×1080 game client at (0,0).
No game input was injected. The original dual-hull detector returned nine
positions: the two top gates lacked a complete 64×64 lower-hull fragment.

A 40×32 bottom-left fragment of the existing lower template recovers them without
moving the cursor into the screen-edge scrolling zone or resource-bar artwork.
This fragment is searched only near the top edge and uses NCC >= 0.75. Its
calibrated offset is (-12,+16) relative to the original lower-hull click centre.
All fragment types are deduplicated in a common building coordinate system.

Expected click positions (one per visually checked building):

```
(1121,40), (1409,40), (484,312), (1493,312), (52,528),
(989,600), (1277,600), (1709,600), (1069,700), (1357,700), (1645,700)
```

The test asserts the exact list and checks all points against existing play-area
safety bounds. This proves detection on the saved scene, not live selection or
successful A commands. Portrait verification remains mandatory.

Reproduce without any live input:

```
cargo run --release --example stargate-scan -- tests/fixtures/stargate-eleven-1080/screen.png
```

screen.png SHA256:
`b4f19b163e9e3dc087e2816b441dfaa3c2814b18d24296bde7224114be12cb6a`
