# Pyrealium Content Editor

Open `emulator/OpenContentEditor.cmd`. The editor remembers your game data
folder, content file, and window size when you close it. Command-line
`--game-data`, `--catalog`, and `--output` options can override the defaults.
On first use, it reuses the launcher's game folder if both required DATs exist.

Choose the folder containing the intended `portal.dat` and `cell_1.dat` files.
Use the DAT set appropriate to the server/client profile you are authoring for.
Choose an existing world-v2/world-v3 content file, or a new `.tsv` file.

## Browse the world

The editor opens on the **World map**, using the native AC2 map image from the
DAT. Click a region to travel directly into its 3D view. Wheel zooms the map and
right-drag pans it. **World map** returns to the atlas; **Back** and **Forward**
revisit destinations. The gold marker shows the last outdoor camera position.

**Fly / explore** switches to camera travel. Click the viewport, hold WASD to
fly and strafe, Q/E to descend/ascend, and drag to look around. Shift flies faster;
Ctrl is finer. Fly speed is visible in the travel toolbar. Movement stops on key
release or focus loss. New outdoor destinations start above nearby terrain.

The left library has **Places**, **Exits**, **Objects**, and **Saved** tabs.
Places initially shows supplied destinations; searching can reveal raw cell IDs.
**Dungeons** groups the interior list by area. **Build objects** reveals the
placement inspector and object catalog. **Project / DATs** reveals file paths.
The inspector is hidden during exploration so the viewport has room.

In **Places**, search a supplied content name, hexadecimal cell, or landblock.
Filter to Outdoor or Dungeon, then double-click a result or choose **Open
selected area**. **Go to cell** opens a particular hexadecimal cell directly.
Generic interior entries include building interiors as well as dungeons.

The browser reads the actual DAT inventory, plus named anchors from the supplied
world and portal-route files. It shows at most 500 matches; narrow the search to
find later entries. Unnamed rooms and blocks use their real identifiers. Opening
an unnamed area finds a starting point on a native floor triangle rather than
assuming Z=0. This is an authoring anchor, not a claim about native walkability.

You can browse and load an area before choosing an entity. **Explore** uses
WASD/arrows to fly, Q/E for vertical travel, Shift for speed and Ctrl for fine
movement. Drag to look around. Outdoor neighborhoods load around the camera;
connected interior rooms load as the camera moves over their visible floors.
**Exits** also offers explicit travel through native doorway connections.
Exploration preserves authored placement fields. Choosing **Place / move** brings
the draft into the explored area.

## Find and place content

- Search the entity catalog by name, entity ID, or visual ID. Ctrl+F focuses
  this search. At most 500 catalog matches are displayed; narrow the search
  to find a particular entry.
- Choose an entity in **Objects**. Heading uses 0–255.
  Blank scale inherits the native asset size; explicit scale must be above
  zero and at most 100.
  The coordinate, heading, and scale fields also have step arrows.
- In **Place / move** mode, click or drag over visible world geometry to move
  the draft. Shift-click also places. In **Orbit** mode, left-drag rotates the
  camera; Alt-drag orbits from either mode. Right-drag pans and the wheel zooms.
- Arrow keys move X/Y; Page Up/Down move Z. The Move step control sets the
  distance, Shift makes movement finer, and Ctrl makes it coarser. The inspector's
  **Tools** tab also provides X/Y/Z movement buttons.
- **F / Frame object** fits the selected model's actual bounds. **Home / Fit
  scene** fits the scene. The orange outline marks the selected object through
  surrounding geometry. Cut away ceiling opens up dungeon views.
- The Position map supports click to position, right-drag
  to pan, mouse wheel to zoom, and Fit cell.
- Choose **Add placement**, or press Ctrl+S. The editor checks the entity,
  visual, and cell records in the DATs before writing the content file.

## Edit an existing placement

In **Saved**, search the placement list by name or identifier. **Current cell only**
filters it to the cell in the form. Select a saved placement to load its
position, heading, and scale. The form shows the object ID and the save button
changes to **Update placement**.

Updating preserves the object's identity. If you leave Heading unchanged,
the complete stored quaternion is preserved, including any tilt. Changing
Heading authors an upright rotation around the vertical axis. The heading
field displays the nearest whole native heading unit.

**New / duplicate** turns the current form into a new placement draft. Move
the copy or change its entity, then add it to allocate a new object ID.
**Delete selected** removes the selected object after confirmation.
Unsaved field changes are marked in the form and checked before switching
objects, reloading, or closing.

## Arrange objects and make spaced copies

Choose **Select** in the 3D workspace, then click an authored object. Ctrl-click
adds or removes objects from the selection. Native DAT scenery stays read-only;
selection edits apply to objects in the current content file. In **Saved**, use
Ctrl/Shift to select several rows instead.

Choose **Arrange selection / spaced copies**. Multiple selected objects also
change the main save button into **Arrange selected objects**. The dialog offers:

- X/Y/Z offsets for moving the selection together.
- Rotation in degrees around the selection's center, retaining each object's
  full orientation tilt and individual scale.
- A copy count. Zero moves the originals; a positive number leaves originals
  in place and creates that many copies of the selection.
- Optional X/Y grid snapping. Zero disables snapping. Z stays at the entered
  offset; grid snapping does not assert a collision-safe or floor-snapped position.

For example, one selected object with Offset X=3 and Copies=4 previews four new
objects at X offsets 3, 6, 9 and 12. Rotation is also applied successively when
creating copies. Moving the originals preserves their IDs; every new copy gets
a new ID.

Changes appear as a preview. **Cancel** restores the previous scene without
writing the file. **Apply to content file** validates the affected asset/cell
records and commits the whole arrangement as one Undo step. Group deletion is
also one undoable operation.

An operation supports at most 256 resulting objects and 32 copies. The runtime's
world and per-cell limits still apply. Interior group arrangements require
objects from one room. Outdoor arrangements use the DAT's native landscape
dimensions to update cell IDs correctly across landblock boundaries.

For repeated surface placement, the inspector's Tools tab has **Keep adding
copies**: move the draft, Ctrl+S, then move and save the next copy.

## Recover and coordinate changes

- Undo and Redo restore up to 20 file edits during this editor session.
  These actions write the file immediately, just like Add, Update, and Delete.
- Each write keeps the previous file at `<content-file>.bak`. This is a rolling
  backup, replaced by the next write, including Undo/Redo. Copy it elsewhere
  before editing further if you need to retain it permanently.
- If another program changes the file, saving stops with a reload message.
  Reloading starts a new undo history. A shared `.edit.lock` coordinates saves
  with the command-line placement tool.
- A failed validation leaves the content file untouched. Scale overrides
  upgrade world-v2 to world-v3 while preserving the other objects.

Edits are offline. They take effect through the server's normal start/content
loading process. Previewing or saving does not establish in-client gameplay
acceptance.

## Validation

Run the persistence and Tk workflow checks from `emulator`:

```powershell
python -m unittest discover -s tools -p test_editor_document.py -v
python -m unittest discover -s tools -p test_content_editor.py -v
python -m unittest discover -s tools -p test_editor_locations.py -v
python -m unittest discover -s tools -p test_editor_arrange.py -v
python -m unittest discover -s tools -p test_editor_travel.py -v
python -m unittest discover -s tools -p test_preview_assets.py -v
python -m unittest discover -s tools -p test_preview_controls.py -v
```

These tests use isolated files and mocked DAT reads. The Tk workflow suite
requires a graphical desktop with Python's Tk support.

## Asset diagnostics

**Details** shows the current scene's remaining limitations and missing records.
Use **Reload 3D assets** in the inspector's Tools tab to retry after correcting
the DAT folder. Failed loads clear the old area's preview instead of leaving a
different area available for picking.

The loader honors explicit null texture overrides and proven fully transparent
helper materials. Empty physics-only meshes do not produce false rendering
errors. Static props can resolve their visuals directly from native entity
definitions when absent from the supplied catalog. Some appearance graphs,
effects, water, animated material behavior, and original shader passes remain
outside the preview's supported subset.

For an isolated editor session, `--no-settings` prevents remembered preferences
from being changed on exit. Source DATs are read-only. Object placements use the content TSV; geometry and
entrances use saved world projects and independent export folders.


## Terrain, custom rooms, and dungeon entrances

Open **Terrain / rooms → Build**. Terrain offers Raise, Lower, Flatten, Smooth,
and Paint. Radius presets and `[` / `]` change the footprint. Flatten's ground
picker and Shift-click sample a real terrain height without sculpting. A drag is
one undo step; Escape cancels it. Flatten/Smooth use blend percentages.

The Dungeon page offers a native room library and **Create a custom room**.
Custom rooms accept width, depth, height, doorway width/height, and openings on
north/east/south/west walls. They contain render and collision geometry, native
physics/cell BSP, and actual doorway holes. They can start an unused dungeon or
snap onto a selected native doorway. Dimensions must match that doorway.
Library pieces can finish open branches. Orange doorway outlines are open;
green ones are connected. Check dungeon connections before export.

**Entrance & return** creates two visible useable portals. Mark the outdoor
portal and a safe return arrival, then mark the dungeon portal and a safe entry
arrival. Each mark uses ground clicks; arrivals must be away from the portal.
Facing can be set separately for each point. Entrance pairs are stored in the
project, previewed in the world, and can be removed with undo available.

Save stores a `.world.json` project (v2), including cell and portal archive edits
and entrance pairs. Existing v1 terrain/dungeon projects still open. Custom room
creation and its mesh form one reversible transaction across both archives.

**Export world** creates a new folder. It contains changed `cell_1.dat` and/or
`portal.dat` copies, `world.tsv`, adjacent `world.routes.tsv`, and the project.
Original archives are never replaced. Unchanged DATs are still required from
the same installation; use the exported archives in a separate game-data copy.
The updated server loads the sidecar for its selected content file with portal
travel enabled; `serve-training` enables it. Keep routes beside their content.
The retained bundled portal routes remain available and cannot be overridden.

Tests check generated room collision volumes and doorway holes independently,
real native template fitting, project undo/reload, and both archive exports.
These checks do not establish native-client rendering, movement, or portal
traversal acceptance. No running client/server is restarted by the editor.


The editor draws cyan entrance markers even when the original portal's visual
is an effect with no supported preview mesh. Use **Preview entry travel** or
**Preview return travel** in the entrance list to check the editor destinations.
These buttons move the editor camera; live player travel uses the server route.

A current server build containing external route support is available at
`emulator/target/worldbuilder-build/release/pyrealium.exe`. It has not replaced
the server running in an existing game session. Build validation does not imply
a live server restart or native-client acceptance.
