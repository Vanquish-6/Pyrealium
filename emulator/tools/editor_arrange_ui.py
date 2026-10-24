"""Preview-first selection arrangement dialog; commits through editor history."""
import tkinter as tk
from tkinter import messagebox, ttk

from editor_arrange import arrangement_rows, prepare_arrangement


class ArrangeDialog:
    def __init__(self, editor, identities):
        self.editor = editor
        self.identities = set(identities)
        self.original = editor.original
        self.rows = [row for row in editor.placements if int(row[0], 16) in self.identities]
        self.pending = None
        self.closed = False
        self.window = tk.Toplevel(editor.root)
        self.window.title(f"Arrange {len(self.rows)} selected objects")
        self.window.transient(editor.root)
        self.window.geometry("500x560")
        self.window.minsize(480, 540)
        self.window.protocol("WM_DELETE_WINDOW", self.close)
        self.window.bind("<Escape>", lambda event: self.close())
        frame = ttk.Frame(self.window, padding=18)
        frame.pack(fill="both", expand=True)
        frame.columnconfigure(1, weight=1)
        ttk.Label(frame, text=f"Arrange {len(self.rows)} objects", style="Section.TLabel").grid(row=0, column=0, columnspan=2, sticky="w", pady=(0, 8))
        ttk.Label(frame, text="Preview movement and rotation around the selection's center.\nApply writes one undoable file change.", style="Muted.TLabel", wraplength=425).grid(row=1, column=0, columnspan=2, sticky="w", pady=(0, 12))
        self.fields = {}
        for index, (label, value, low, high, step) in enumerate((
            ("Offset X", "0", -999999, 999999, 1),
            ("Offset Y", "0", -999999, 999999, 1),
            ("Offset Z", "0", -999999, 999999, .25),
            ("Rotation (degrees)", "0", -36000, 36000, 15),
            ("Copies (0 = move)", "0", 0, 32, 1),
            ("Snap X/Y grid (0 = off)", "0", 0, 1000, .5)), 2):
            variable = tk.StringVar(value=value)
            self.fields[label] = variable
            ttk.Label(frame, text=label).grid(row=index, column=0, sticky="w", pady=5, padx=(0, 12))
            ttk.Spinbox(frame, textvariable=variable, from_=low, to=high, increment=step, width=12).grid(row=index, column=1, sticky="ew", pady=5)
            variable.trace_add("write", lambda *unused: self.schedule_preview())
        ttk.Label(frame, text="Copies use successive offsets and rotations. Original objects stay in place when Copies is above zero.", style="Muted.TLabel", wraplength=425).grid(row=8, column=0, columnspan=2, sticky="w", pady=10)
        self.status = tk.StringVar()
        ttk.Label(frame, textvariable=self.status, wraplength=425).grid(row=9, column=0, columnspan=2, sticky="w", pady=8)
        buttons = ttk.Frame(frame)
        buttons.grid(row=10, column=0, columnspan=2, sticky="ew", pady=8)
        ttk.Button(buttons, text="Cancel", command=self.close).pack(side="left")
        self.apply_button = ttk.Button(buttons, text="Apply to content file", style="Accent.TButton", command=self.apply)
        self.apply_button.pack(side="right")
        self.window.grab_set()
        self.schedule_preview()

    def parameters(self):
        try:
            return dict(translation=tuple(float(self.fields[f"Offset {axis}"].get()) for axis in "XYZ"),
                        degrees=float(self.fields["Rotation (degrees)"].get()),
                        copies=int(self.fields["Copies (0 = move)"].get()),
                        grid=float(self.fields["Snap X/Y grid (0 = off)"].get()))
        except ValueError as error:
            raise ValueError("Enter numeric offsets, rotation and grid size, and a whole copy count.") from error

    def schedule_preview(self):
        if self.pending is not None:
            self.window.after_cancel(self.pending)
        self.pending = self.window.after(120, self.preview)

    def preview(self):
        self.pending = None
        if self.closed:
            return
        try:
            options = self.parameters()
            changed = arrangement_rows(self.rows, landscape=self.editor.outdoor_grid, **options)
            reference = self.editor.active_scene_cell or int(self.rows[0][3], 16)
            self.editor.view3d.preview_arrangement(changed, self.rows, reference,
                                                 self.editor.outdoor_grid, duplicate=options["copies"] > 0)
            count = len(changed)
            loaded = {item.get("object_id") for item in self.editor.view3d.instances}
            missing = len(self.identities - loaded)
            self.status.set(f"Previewing {count} {'new copies' if options['copies'] else 'moved objects'}. Nothing saved yet."
                            + (f" {missing} selected source objects have no loaded preview geometry." if missing else ""))
            self.apply_button.configure(state="normal")
        except (ValueError, OverflowError) as error:
            self.editor.view3d.clear_arrangement()
            self.status.set(str(error))
            self.apply_button.configure(state="disabled")

    def apply(self):
        try:
            self.editor.require_loaded_path()
            if self.editor.original != self.original:
                raise ValueError("The editor's content changed. Close this dialog and select the objects again.")
            result, identities = prepare_arrangement(self.original, self.identities, self.editor.game.get(), **self.parameters())
            self.editor.view3d.clear_arrangement()
            self.editor.commit_change(result)
            self.editor.select_many(identities)
            self.editor.status.set(f"Arranged {len(identities)} objects. Undo restores the complete change.")
            self.close()
        except (OSError, ValueError, KeyError, OverflowError) as error:
            messagebox.showerror("Arrangement was not saved", str(error), parent=self.window)

    def close(self):
        if self.closed:
            return
        self.closed = True
        if self.pending is not None:
            self.window.after_cancel(self.pending)
        self.editor.view3d.clear_arrangement()
        self.editor.arrange_dialog = None
        self.window.destroy()
