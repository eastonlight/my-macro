# Left-clipped Stargate and Gateway false positives

Read-only PrintWindow capture of the user's 1920×1080 client at (0,0). No game
input was injected. The scene contains eleven Stargates and three Gateways.

Before this fix the detector returned thirteen targets: ten Stargates plus three
Gateways (scores 0.477–0.482), missing the leftmost Stargate whose normal click
centre was at x=16, inside the 24px edge-scroll exclusion.

Changes verified against this scene:

- Primary world-template NCC threshold raised from 0.46 to 0.75 to reject weak
  matches before clicking. Portrait verification before A remains mandatory.
- A 48×64 right-hand fin fragment recovers the left-clipped gate. It comes from
  the original red-team calibration scene, not this capture, and is searched only
  near the left edge. The click is on visible hull at (80,520), not at x=16.
- Upper-hull clicks move 16px upwards within the same template so bottom-row
  gates can retain their strong matches without entering the command-card HUD.
  No play-area guards were relaxed.

Expected safe click positions, one per Stargate:

```text
(1085,40), (1373,40), (448,312), (1457,312), (80,520),
(953,600), (1241,600), (1673,600), (1033,700), (1321,700), (1609,700)
```

The exact-coordinate test also excludes Gateway regions. Separate negative
checks move crops of the three Gateways to different search locations; a synthetic
edge scene checks that the extra fin detection does not duplicate a normal core.
These are offline detection tests, not proof of live selection or A commands.

screen.png SHA256:
`ad19ecaabc0e8ddef4313a16f8420196c113290f04250a7f150b6862051341e6`
