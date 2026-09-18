# Actual Colony preview near a coloured building

These are lossless 480×480 RGB crops at **screen origin (96, 300)** from previously saved, authorized game captures. They were cropped offline; no new game input or capture was performed for this regression.

- `before.png`: terrain and selected Drone before `B`, `C`.
- `green-near-building.png`: green Colony preview at cursor **(336, 540)**. Visible footprint: x=252..395, y=466..609; integer detected center **(323, 537)**.
- Source client: 1920×1080 at (0,0).

Source SHA256 values (full originals remain outside the repository):

- `single-before.png`: `1f8a4be1a0215281855f8bc444ea5620788eafbaca429fcf3f59ab4d53374334`
- `preview.png`: `213fbe091a25bcd2a402051db12aa724702c3f9ca72c5fb1ad184a8eb2645438`

## Reproduced failure

The previous whole-window colour bounding box included green pixels from a nearby existing building. Its green bounds expanded to x=186..395, y=369..609 (210×241), outside the 144px footprint tolerance, so it returned `Placement::Absent` despite the visible green preview.

The strict F4 builder now separates 8-connected colour components before applying the existing square dimensions, fill, ring and cursor-distance gates. Fresh colour in every quadrant and partial-red rejection remain mandatory, as do the two-read live checks and other safety gates. Row/Spire preview detection remains unchanged.

Tests establish:

1. The old detector rejects this exact preview; the local detector and strict freshness gate accept its correct center.
2. Reusing the same frame, unchanged terrain, or an already reserved center never authorizes a click.
3. Adding a red patch inside this real preview is still refused.

These captures prove a pre-click false negative, not successful arrival, construction-start detection, or live end-to-end placement.
