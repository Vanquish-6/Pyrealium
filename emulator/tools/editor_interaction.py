"""One mouse gesture is one validated object edit, or no edit on cancel."""
import math
from tkinter import messagebox

from editor_arrange import arrangement_rows, prepare_arrangement
from editor_document import heading_from_row


class ObjectTools:
    def __init__(self, editor):
        self.editor = editor
        self.state = None

    def selected_ids(self):
        editor = self.editor
        return editor.group_ids or ((editor.selected_id,) if editor.selected_id is not None else ())

    def restore_fields(self, values):
        editor = self.editor
        editor.changing_fields = True
        try:
            for name, value in values.items():
                editor.fields[name].set(value)
        finally:
            editor.changing_fields = False

    def cancel(self):
        if self.state is None:
            return
        state, self.state = self.state, None
        self.editor.view3d.clear_arrangement()
        self.restore_fields(state["fields"])
        self.editor.update_actions()
        self.editor.status.set("Canceled the drag. No file change.")

    def transform(self, action, options, phase):
        editor = self.editor
        if phase == "begin":
            identities = self.selected_ids()
            if not identities:
                editor.status.set("Select an object first, then drag to move, rotate or scale it.")
                return False
            if editor.future is not None or editor.scene_timer is not None:
                editor.status.set("The selection is loading. Drag once its outline appears.")
                return False
            self.state = dict(ids=identities, original=editor.original, fields=editor.field_snapshot(),
                              rows=[list(row) for row in editor.placements if int(row[0], 16) in identities],
                              options={}, error=None)
            return True
        if phase == "cancel":
            self.cancel()
            return True
        if self.state is None:
            return False
        state = self.state
        if phase == "preview":
            try:
                options = dict(options)
                if editor.snap_enabled.get():
                    spacing = float(editor.snap_size.get())
                    if not math.isfinite(spacing) or not .01 <= spacing <= 100:
                        raise ValueError("Snap spacing must be between 0.01 and 100")
                    if "translation" in options:
                        options["translation"] = tuple(round(value/spacing)*spacing for value in options["translation"])
                    if "degrees" in options:
                        options["degrees"] = round(options["degrees"]/15)*15
                    if "scale" in options:
                        options["scale"] = max(.01, round(options["scale"]*10)/10)
                rows = arrangement_rows(state["rows"], landscape=editor.outdoor_grid, **options)
                editor.view3d.preview_arrangement(rows, state["rows"], editor.active_scene_cell or int(rows[0][3], 16), editor.outdoor_grid)
                first = rows[0]
                values = dict(Cell=first[3], X=first[4], Y=first[5], Z=first[6], Heading=str(heading_from_row(first)),
                              Scale=first[11] if len(first) == 12 and first[11] != "-" else "")
                self.restore_fields(values)
                state["options"], state["error"] = options, None
                editor.status.set(f"{action.title()} preview · {len(rows)} object(s) · release to apply / Escape to cancel")
                return True
            except (ValueError, OverflowError) as error:
                state["error"] = str(error)
                editor.status.set(str(error))
                return False
        if phase != "commit":
            return False
        options = state["options"]
        unchanged = (all(abs(v) < 1e-8 for v in options.get("translation", (0, 0, 0)))
                     and abs(options.get("degrees", 0)) < 1e-8 and abs(options.get("scale", 1)-1) < 1e-8)
        if state["error"] or unchanged:
            self.cancel()
            return True
        camera = (editor.view3d.focus, editor.view3d.azimuth, editor.view3d.elevation, editor.view3d.distance)
        try:
            editor.require_loaded_path()
            if editor.original != state["original"]:
                raise ValueError("Content changed during the drag; reload and try again")
            result, identities = prepare_arrangement(state["original"], state["ids"], editor.game.get(), **options)
            editor.view3d.clear_arrangement()
            self.restore_fields(state["fields"])
            self.state = None
            editor.commit_change(result)
            editor.select_many(identities)
            editor.view3d.set_tool(action)
            editor.selection_camera = camera
            editor.status.set(f"{action.title()} applied to {len(identities)} object(s). Ctrl+Z undoes this drag.")
            return True
        except (OSError, ValueError, KeyError, OverflowError) as error:
            self.cancel()
            messagebox.showerror("Object edit was not saved", str(error), parent=editor.root)
            return False

    def stamp(self):
        editor = self.editor
        if not editor.view3d.placement_enabled:
            return
        editor.selected_id, editor.group_ids = None, ()
        before = len(editor.placements)
        keep = editor.keep_adding.get()
        editor.keep_adding.set(True)
        try:
            editor.save()
        finally:
            editor.keep_adding.set(keep)
        if len(editor.placements) > before:
            editor.status.set("Object placed. Click another surface to place again · Ctrl+Z undo · Escape finish.")

    def duplicate(self):
        editor = self.editor
        identities = self.selected_ids()
        if not identities:
            editor.status.set("Select one or more authored objects to duplicate.")
            return
        try:
            editor.require_loaded_path()
            spacing = float(editor.snap_size.get()) if editor.snap_enabled.get() else 1
            result, new_ids = prepare_arrangement(editor.original, identities, editor.game.get(), translation=(spacing, 0, 0), copies=1)
            editor.commit_change(result)
            editor.select_many(new_ids)
            editor.view3d.set_tool("move")
            editor.status.set(f"Duplicated {len(new_ids)} object(s). Drag to position the copies.")
        except (OSError, ValueError, KeyError, OverflowError) as error:
            messagebox.showerror("Objects were not duplicated", str(error), parent=editor.root)
