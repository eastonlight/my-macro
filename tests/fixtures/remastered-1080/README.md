# Remastered 1920×1080 live-capture fixtures

Captured locally from the user's running StarCraft Remastered game with permission.
Only game UI/placement crops are stored; no desktop content is included.

## Selection-panel crops (550×200, offset (600, 880))

- `drones-2.png` through `drones-5.png`: true selections of 2–5 drones,
  **all wireframes fully visible**.
- `drones-5-tooltip.png`: a real 5-drone selection where an in-game hover
  tooltip ("클릭: 유닛 선택 / Shift+클릭: 유닛 선택 해제 / Ctrl+클릭: 유닛 유형 선택")
  covers the fifth wireframe. It is a **negative** fixture: the detector must
  refuse it instead of counting the four visible portraits.
- `drone-single.png`: single drone selected, shown as the Korean single-unit
  information panel (not a wireframe grid).
- `not-drone-colony.png`: single morphing Creep Colony information panel,
  NOT a drone. Also a negative fixture.

Selected-unit icons fill **column-major**, top then bottom: centres
approximately `(653 + 81 * column, 930 + 83 * row)`, columns 0–5, rows 0–1.
For five drones, columns contain 2, 2, 1 portraits.

## Placement crops (430×300, offset (160, 200))

- `preview-first.png`: valid green colony placement preview, cursor `(320,340)`.
- `preview-second.png`: valid green colony placement preview, cursor `(464,340)`.
- `no-preview.png`: two morphing colonies after placement, no placement preview.

Footprint rectangles observed: first approximately `(239,286)-(383,430)`,
second `(383,286)-(527,430)`. Footprint / horizontal center spacing: **144 px**.
The green preview square measures exactly **144×144**. Cursor placement is
snapped to a tile grid; the cursor need not equal the footprint centre.

## Live control protocol verified

1. Ctrl+9 saves a selected drone group. 9 recalls it without moving the camera
   when it is not a rapid double-tap.
2. Clicking a selected-unit portrait selects that one drone.
3. B, C, then a world click assigns a Creep Colony build order to that drone.
4. Recall 9. If the issued drone has not morphed yet, it remains in the group.
   Shift-clicking its portrait removes it from the selection. Ctrl+9 then
   saves the remaining drones, preventing reuse of a traveling drone.
5. A group of two becomes a **single-unit information panel** after removing
   one portrait; it is not a one-icon group panel.
6. The final remaining drone can be assigned directly, without a portrait click.

Two real colonies were observed morphing side by side after this sequence.

## Scope caveats

- Only the 1920×1080 client with the standard HUD/skin is represented. The
  detector rejects every other profile instead of guessing a scale.
- `drones-5.png` was regenerated from the clean full capture
  `restored-group.png` (five fully visible drones); the earlier occluded
  capture is kept as `drones-5-tooltip.png`.
- No real 6–12 drone capture exists; those layouts are covered by synthetic
  tests that paint wireframes with the same embedded mask.
- These fixtures were collected **before** the Rust F6 implementation existed.
  They calibrate the detector; they are not evidence that the full F6 row
  protocol has been verified live.
