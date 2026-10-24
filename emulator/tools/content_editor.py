"""Offline graphical content placement. Saves through place_content's DAT checks."""
import argparse
from concurrent.futures import ThreadPoolExecutor
import math
from pathlib import Path
from types import SimpleNamespace
import tkinter as tk
from tkinter import filedialog, messagebox, ttk

from place_content import bounded_text, place
from preview_assets import Assets
from preview_view import Preview3D


class Editor:
    def __init__(self, root, args):
        self.root = root
        self.args = args
        self.catalog = []
        self.placements = []
        self.transform = None
        self.map_view = None
        self.executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="content-dat")
        self.assets = None
        self.future = None
        self.wanted = None
        self.scene_ready = None
        self.content_revision = 0
        self.closing = False
        root.protocol("WM_DELETE_WINDOW", self.close)
        root.title("World Content Placement")
        root.geometry("1180x790")
        root.minsize(920, 650)
        self.game = tk.StringVar(value=str(args.game_data or ""))
        self.output = tk.StringVar(value=str(args.output))
        self.search = tk.StringVar()
        self.fields = {name: tk.StringVar(value=value) for name, value in (
            ("Entity", ""), ("Cell", "02990109"), ("X", "58.060577"),
            ("Y", "240.199"), ("Z", "-44.606205"), ("Heading", "0"), ("Scale", ""))}
        self.status = tk.StringVar(value="Choose an entity and a position, then save a placement.")
        shell = ttk.Frame(root, padding=12)
        shell.pack(fill="both", expand=True)
        shell.columnconfigure(1, weight=1)
        shell.rowconfigure(3, weight=1)
        ttk.Label(shell, text="World content placement", font=("Segoe UI", 18, "bold")).grid(
            row=0, column=0, columnspan=3, sticky="w", pady=(0, 8))
        self.path_row(shell, 1, "Game data folder", self.game, self.pick_game)
        self.path_row(shell, 2, "Content file", self.output, self.pick_output)
        panes = ttk.Panedwindow(shell, orient="horizontal")
        panes.grid(row=3, column=0, columnspan=3, sticky="nsew", pady=12)
        left, right = ttk.Frame(panes), ttk.Frame(panes)
        panes.add(left, weight=1)
        panes.add(right, weight=2)
        ttk.Label(left, text="Entity catalog").pack(anchor="w")
        ttk.Entry(left, textvariable=self.search).pack(fill="x", pady=5)
        self.matches = ttk.Label(left)
        self.matches.pack(anchor="w")
        self.entities = self.tree(left, ("Entity", "Label"), 12)
        self.entities.bind("<<TreeviewSelect>>", self.choose_entity)
        ttk.Label(left, text="Saved placements (select to copy a position)").pack(anchor="w", pady=(12, 4))
        self.objects = self.tree(left, ("Object", "Entity", "Cell"), 9)
        self.objects.bind("<<TreeviewSelect>>", self.choose_position)
        ttk.Button(left, text="Reload file", command=self.reload).pack(anchor="w", pady=6)
        form = ttk.Frame(right)
        form.pack(fill="x")
        for index, (name, variable) in enumerate(self.fields.items()):
            row, column = divmod(index, 2)
            ttk.Label(form, text=name).grid(row=row, column=column * 2, sticky="w", padx=5, pady=5)
            ttk.Entry(form, textvariable=variable, width=20).grid(
                row=row, column=column * 2 + 1, sticky="ew", padx=5, pady=5)
        form.columnconfigure(1, weight=1)
        form.columnconfigure(3, weight=1)
        ttk.Label(right, text="Heading: 0–255. Blank scale inherits the asset size.").pack(anchor="w", pady=4)
        notebook = ttk.Notebook(right)
        notebook.pack(fill="both", expand=True, pady=(8, 0))
        self.view3d = Preview3D(notebook, on_pick=self.pick_3d)
        map_frame = ttk.Frame(notebook)
        notebook.add(self.view3d, text="3D world")
        notebook.add(map_frame, text="Position map")
        bar = ttk.Frame(map_frame)
        bar.pack(fill="x")
        ttk.Label(bar, text="Click: place · right-drag: pan · wheel: zoom").pack(side="left")
        ttk.Button(bar, text="Fit cell", command=self.fit_map).pack(side="right")
        self.canvas = tk.Canvas(map_frame, background="#182330", highlightthickness=0, height=300)
        self.canvas.pack(fill="both", expand=True)
        self.canvas.bind("<Configure>", lambda event: self.preview())
        self.canvas.bind("<Button-1>", self.pick_position)
        self.canvas.bind("<MouseWheel>", self.zoom_map)
        self.canvas.bind("<ButtonPress-3>", self.begin_pan)
        self.canvas.bind("<B3-Motion>", self.pan_map)
        self.render_status = tk.StringVar(value="Choose game data and an entity to load native geometry.")
        ttk.Label(right, textvariable=self.render_status, wraplength=610).pack(anchor="w", pady=6)
        self.save_button = ttk.Button(right, text="Validate DAT records and add placement", command=self.save)
        self.save_button.pack(anchor="e", pady=8)
        ttk.Label(shell, textvariable=self.status, wraplength=1100).grid(
            row=4, column=0, columnspan=3, sticky="w")
        ttk.Label(shell, text="Saves an offline content file. Use the file with the server's --content option on its next normal start.",
                  wraplength=1100).grid(row=5, column=0, columnspan=3, sticky="w", pady=(6, 0))
        self.search.trace_add("write", lambda *unused: self.filter_catalog())
        for variable in self.fields.values():
            variable.trace_add("write", lambda *unused: self.preview())
        self.game.trace_add("write", lambda *unused: self.preview())
        self.load_catalog()
        self.reload()

    @staticmethod
    def tree(parent, columns, height):
        frame = ttk.Frame(parent)
        frame.pack(fill="both", expand=True)
        tree = ttk.Treeview(frame, columns=columns, show="headings", height=height, selectmode="browse")
        scrollbar = ttk.Scrollbar(frame, orient="vertical", command=tree.yview)
        tree.configure(yscrollcommand=scrollbar.set)
        tree.pack(side="left", fill="both", expand=True)
        scrollbar.pack(side="right", fill="y")
        for column in columns:
            tree.heading(column, text=column)
            tree.column(column, width=180 if column == "Label" else 105, minwidth=65)
        return tree

    @staticmethod
    def path_row(parent, row, label, variable, command):
        ttk.Label(parent, text=label).grid(row=row, column=0, sticky="w", padx=(0, 10), pady=4)
        ttk.Entry(parent, textvariable=variable).grid(row=row, column=1, sticky="ew", pady=4)
        ttk.Button(parent, text="Browse…", command=command).grid(row=row, column=2, padx=(8, 0), pady=4)

    def pick_game(self):
        folder = filedialog.askdirectory(title="Folder containing portal.dat and cell_1.dat")
        if folder:
            self.game.set(folder)

    def pick_output(self):
        path = filedialog.asksaveasfilename(title="Choose an existing or new content file",
                                          defaultextension=".tsv", filetypes=[("Content files", "*.tsv")],
                                          confirmoverwrite=False)
        if path:
            self.output.set(path)
            self.reload()

    def load_catalog(self):
        try:
            lines = bounded_text(self.args.catalog).splitlines()
            if not lines or lines[0] != "pyrealium-definitions-v1":
                raise ValueError("Unsupported entity catalog")
            rows = [line.split("\t") for line in lines[1:] if line and not line.startswith("#")]
            if any(len(row) != 3 for row in rows):
                raise ValueError("Invalid entity catalog record")
            for entity, visual, label in rows:
                if int(entity, 16) >> 24 != 0x47 or int(visual, 16) >> 24 != 0x1f:
                    raise ValueError("Invalid entity or visual identifier")
            self.catalog = rows
            self.filter_catalog()
        except (OSError, ValueError) as error:
            messagebox.showerror("Catalog could not be loaded", str(error), parent=self.root)

    def filter_catalog(self):
        query = self.search.get().strip().casefold()
        rows = [row for row in self.catalog if query in " ".join(row).casefold()]
        self.entities.delete(*self.entities.get_children())
        for index, (entity, visual, label) in enumerate(rows[:500]):
            self.entities.insert("", "end", iid=str(index), values=(entity, label))
        self.matches.configure(text=f"{len(rows):,} matches · showing up to 500")

    def choose_entity(self, event):
        selected = self.entities.selection()
        if selected:
            self.fields["Entity"].set(self.entities.item(selected[0], "values")[0])

    def reload(self):
        try:
            path = Path(self.output.get())
            lines = bounded_text(path).splitlines() if path.exists() else ["pyrealium-world-v2"]
            if not lines or lines[0] not in ("pyrealium-world-v2", "pyrealium-world-v3"):
                raise ValueError("Choose a world-v2 or world-v3 file")
            size = 11 if lines[0].endswith("2") else 12
            rows = [line.split("\t") for line in lines[1:] if line and not line.startswith("#")]
            if len(rows) > 16384 or any(len(row) != size for row in rows):
                raise ValueError("Invalid placement file shape or object limit")
            seen = set()
            for row in rows:
                identity = int(row[0], 16)
                if identity in seen or identity >> 48 != 0x8140:
                    raise ValueError("Duplicate or invalid object identity")
                seen.add(identity)
                int(row[3], 16)
                if any(not math.isfinite(float(value)) or abs(float(value)) >= 1_000_000 for value in row[4:7]):
                    raise ValueError("Invalid position")
            self.placements = rows
            self.content_revision += 1
            self.objects.delete(*self.objects.get_children())
            for row in rows:
                self.objects.insert("", "end", iid=row[0], values=(row[0], row[1], row[3]))
            self.status.set(f"Loaded {len(rows):,} placements. Saving adds a new object; existing objects keep their identities.")
            self.preview()
        except (OSError, ValueError) as error:
            self.placements = []
            self.objects.delete(*self.objects.get_children())
            self.preview()
            self.status.set(str(error))
            messagebox.showerror("Content could not be loaded", str(error), parent=self.root)

    def choose_position(self, event):
        selected = self.objects.selection()
        if not selected:
            return
        row = next(row for row in self.placements if row[0] == selected[0])
        for name, value in zip(("Entity", "Cell", "X", "Y", "Z"), row[1:2] + row[3:7]):
            self.fields[name].set(value)
        self.fields["Scale"].set(row[11] if len(row) == 12 and row[11] != "-" else "")

    def preview(self):
        self.refresh_scene()
        self.canvas.delete("all")
        self.transform = None
        try:
            cell = int(self.fields["Cell"].get(), 16)
            x, y = (float(self.fields[name].get()) for name in ("X", "Y"))
            if not all(math.isfinite(v) and abs(v) < 1_000_000 for v in (x, y)):
                return
            points = [(float(row[4]), float(row[5]), row[1]) for row in self.placements if int(row[3], 16) == cell]
            width, height = self.canvas.winfo_width(), self.canvas.winfo_height()
            if width < 80 or height < 80:
                return
            xs, ys = [x] + [p[0] for p in points], [y] + [p[1] for p in points]
            if self.map_view is None or self.map_view[0] != cell:
                self.map_view = (cell, (min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2,
                                 max(20, max(xs)-min(xs), max(ys)-min(ys)) * 1.15)
            _, middle_x, middle_y, span = self.map_view
            scale = min(width-70, height-70) / span
            self.transform = middle_x, middle_y, scale, width, height
            def project(px, py):
                return width / 2 + (px - middle_x) * scale, height / 2 - (py - middle_y) * scale
            self.canvas.create_text(12, 12, anchor="nw", fill="#b9cad9", text=f"Cell {cell:08x} · +Y ↑ · +X →")
            spacing = 10 ** math.floor(math.log10(span / 4))
            if span / spacing > 12:
                spacing *= 5
            for axis in range(2):
                center = middle_x if axis == 0 else middle_y
                first = math.floor((center-span)/spacing)
                last = math.ceil((center+span)/spacing)
                for index in range(first, last+1):
                    value = index * spacing
                    sx, sy = project(value if axis == 0 else middle_x, value if axis == 1 else middle_y)
                    if axis == 0:
                        self.canvas.create_line(sx, 28, sx, height, fill="#263746")
                        self.canvas.create_text(sx+3, height-12, text=f"{value:g}", fill="#708a9d", anchor="w")
                    else:
                        self.canvas.create_line(0, sy, width, sy, fill="#263746")
                        self.canvas.create_text(4, sy-7, text=f"{value:g}", fill="#708a9d", anchor="w")
            for px, py, entity in points:
                sx, sy = project(px, py)
                self.canvas.create_oval(sx-4, sy-4, sx+4, sy+4, fill="#72b9e5", outline="")
                self.canvas.create_text(sx+7, sy, anchor="w", text=entity, fill="#b9cad9", font=("Segoe UI", 8))
            sx, sy = project(x, y)
            self.canvas.create_oval(sx-6, sy-6, sx+6, sy+6, fill="#efb65e", outline="white")
            self.canvas.create_text(sx, sy+17, text="New placement", fill="#efb65e")
        except (ValueError, OverflowError):
            pass

    def fit_map(self):
        self.map_view = None
        self.preview()

    def zoom_map(self, event):
        if self.map_view:
            cell, x, y, span = self.map_view
            self.map_view = cell, x, y, max(0.1, min(100000, span * (0.8 if event.delta > 0 else 1.25)))
            self.preview()

    def begin_pan(self, event):
        self.pan_origin = (event.x, event.y, self.map_view)

    def pan_map(self, event):
        if self.transform and getattr(self, "pan_origin", None) and self.pan_origin[2]:
            sx, sy, view = self.pan_origin
            cell, x, y, span = view
            scale = self.transform[2]
            self.map_view = cell, x - (event.x-sx)/scale, y + (event.y-sy)/scale, span
            self.preview()

    def pick_3d(self, point):
        for name, value in zip(("X", "Y", "Z"), point):
            self.fields[name].set(format(value, ".9g"))

    def refresh_scene(self):
        if self.closing:
            return
        try:
            folder = self.game.get().strip()
            entity = int(self.fields["Entity"].get(), 16)
            visual = {int(row[1], 16) for row in self.catalog if int(row[0], 16) == entity}
            if len(visual) != 1 or not folder:
                return
            cell = int(self.fields["Cell"].get(), 16)
            position = [float(self.fields[name].get()) for name in ("X", "Y", "Z")]
            heading = int(self.fields["Heading"].get())
            scale_text = self.fields["Scale"].get().strip()
            scale = float(scale_text) if scale_text else None
            if not all(math.isfinite(v) and abs(v) < 1_000_000 for v in position) or not 0 <= heading <= 255:
                return
            if scale is not None and (not math.isfinite(scale) or not 0 < scale <= 100):
                return
            self.view3d.update_placement(position, heading, scale)
            key = (folder, cell, next(iter(visual)), self.content_revision)
            self.wanted = (key, position, heading, scale, list(self.placements))
            if self.future is None and key != self.scene_ready:
                self.start_load()
        except (ValueError, OverflowError):
            pass

    def start_load(self):
        request = self.wanted
        self.render_status.set("Reading native room and creature geometry from DAT files…")
        self.future = self.executor.submit(self.load_scene, request)
        self.loading = request
        self.root.after(60, self.finish_load)

    def load_scene(self, request):
        key, position, heading, scale, placements = request
        folder, cell, visual, revision = key
        if self.assets is None or self.assets.folder != Path(folder):
            if self.assets:
                self.assets.close()
                self.assets = None
            self.assets = Assets(Path(folder))
        candidates = {}
        for entity, target, label in self.catalog:
            candidates.setdefault(int(entity, 16), set()).add(int(target, 16))
        catalog = {entity: next(iter(targets)) for entity, targets in candidates.items() if len(targets) == 1}
        return self.assets.scene(cell, visual, position, heading, scale, placements, catalog)

    def finish_load(self):
        if self.closing:
            return
        if not self.future.done():
            self.root.after(60, self.finish_load)
            return
        request = self.loading
        future, self.future = self.future, None
        key, position, heading, scale, placements = request
        if self.wanted[0] != key:
            # Discard stale loads; at most one bounded DAT job is queued.
            self.start_load()
            return
        self.scene_ready = key
        try:
            instances, warnings = future.result()
            self.view3d.set_scene(instances, position, warnings, heading=heading)
            self.view3d.update_placement(self.wanted[1], self.wanted[2], self.wanted[3])
            count = sum(len(row["mesh"]["indices"]) // 3 for row in instances)
            self.render_status.set(f"Loaded {len(instances)} native meshes · {count:,} triangles. "
                                   + (warnings[0] if warnings else "Actual DAT geometry; default pose."))
        except (OSError, ValueError, KeyError) as error:
            self.render_status.set(f"3D preview unavailable: {error}")

    def close(self):
        self.closing = True
        self.view3d.close()
        def release():
            if self.assets:
                self.assets.close()
        self.executor.submit(release)
        self.executor.shutdown(wait=False)
        self.root.destroy()

    def pick_position(self, event):
        if self.transform:
            x, y, scale, width, height = self.transform
            self.fields["X"].set(format(x + (event.x-width/2)/scale, ".9g"))
            self.fields["Y"].set(format(y - (event.y-height/2)/scale, ".9g"))

    def save(self):
        self.save_button.configure(state="disabled")
        try:
            if not self.game.get().strip() or not self.output.get().strip():
                raise ValueError("Choose a game data folder and a content file")
            scale = self.fields["Scale"].get().strip()
            args = SimpleNamespace(catalog=self.args.catalog, game_data=Path(self.game.get()),
                output=Path(self.output.get()), entity=int(self.fields["Entity"].get(), 16),
                cell=int(self.fields["Cell"].get(), 16), position=[float(self.fields[name].get()) for name in ("X", "Y", "Z")],
                heading=int(self.fields["Heading"].get()), scale=float(scale) if scale else None)
            self.status.set("Checking definition, visual and cell records…")
            self.root.update_idletasks()
            identity = place(args)
            self.reload()
            self.objects.selection_set(f"{identity:016x}")
            self.objects.see(f"{identity:016x}")
            self.status.set(f"Saved object {identity:016x}. The running server has not been changed.")
        except (OSError, ValueError, OverflowError) as error:
            messagebox.showerror("Placement was not saved", str(error), parent=self.root)
            self.status.set(f"Not saved: {error}")
        finally:
            self.save_button.configure(state="normal")


def main():
    base = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game-data", type=Path)
    parser.add_argument("--catalog", type=Path, default=base / "content" / "definitions.tsv")
    parser.add_argument("--output", type=Path, default=base / "content" / "local-world.tsv")
    args = parser.parse_args()
    root = tk.Tk()
    Editor(root, args)
    root.mainloop()


if __name__ == "__main__":
    main()
