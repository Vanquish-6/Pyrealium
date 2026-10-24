"""Offline graphical content placement. Saves through place_content's DAT checks."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import faulthandler
import logging
import json
import math
import os
from pathlib import Path
import sys
import tempfile
import traceback
import xml.etree.ElementTree as ET
from types import SimpleNamespace
import tkinter as tk
from tkinter import filedialog, messagebox, ttk

from place_content import bounded_text
from editor_document import (read_document, rows_from_text, prepare_change,
                             commit_document, heading_from_row)
from preview_assets import Assets
from preview_view import Preview3D
from editor_travel import WorldAtlas, load_atlas
from editor_interaction import ObjectTools
from editor_palette import ModelSwatch
from editor_geometry import GeometryTools


def launcher_game_folder():
    """Reuse the launcher's chosen DAT folder on the editor's first run."""
    path = Path(os.environ.get("LOCALAPPDATA", tempfile.gettempdir())) / "Pyrealium" / "launcher.xml"
    try:
        folder = ET.fromstring(bounded_text(path)).findtext("GameDirectory", "")
        if folder and all((Path(folder) / name).is_file() for name in ("portal.dat", "cell_1.dat")):
            return folder
    except (OSError, ValueError, ET.ParseError):
        pass
    return ""


class Editor:
    def __init__(self, root, args):
        self.root = root
        self.args = args
        self.settings_path = Path(os.environ.get("LOCALAPPDATA", tempfile.gettempdir())) / "Pyrealium" / "content-editor" / "settings.json"
        try:
            self.settings = json.loads(bounded_text(self.settings_path))
            if not isinstance(self.settings, dict):
                self.settings = {}
        except (OSError, ValueError):
            self.settings = {}
        self.loaded_path = None
        self.original = None
        self.selected_id = None
        self.group_ids = ()
        self.arrange_dialog = None
        self.selection_camera = None
        self.baseline = None
        self.history = []
        self.redo_history = []
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
        self.changing_fields = False
        self.outdoor_grid = None
        self.active_scene_cell = None
        self.atlas_future = None
        self.atlas_folder = None
        self.navigation_history = []
        self.navigation_index = -1
        self.history_target = None
        self.has_travelled = False
        self.arrival_cell = None
        self.locations = []
        self.locations_future = None
        self.location_request = None
        self.navigation_future = None
        self.navigation_wanted = None
        self.locations_folder = None
        self.scene_timer = None
        self.location_timer = None
        self.retry_revision = 0
        self.explore_cell = None
        self.explore_position = None
        self.stream_camera = None
        self.stream_origin_focus = None
        self.camera_timer = None
        self.links_future = None
        self.links_cell = None
        self.nearby_locations = []
        root.protocol("WM_DELETE_WINDOW", self.close)
        root.title("Pyrealium WorldBuilder")
        try:
            root.geometry(self.settings.get("geometry", "1360x900"))
        except tk.TclError:
            root.geometry("1360x900")
        root.minsize(1140, 750)
        self.game = tk.StringVar(value=str(args.game_data or self.settings.get("game_data") or launcher_game_folder()))
        self.output = tk.StringVar(value=str(args.output or self.settings.get("output", Path(__file__).resolve().parent.parent / "content" / "local-world.tsv")))
        self.search = tk.StringVar()
        self.placement_search = tk.StringVar()
        self.cell_only = tk.BooleanVar()
        self.mode = tk.StringVar(value="New placement")
        self.fields = {name: tk.StringVar(value=value) for name, value in (
            ("Entity", ""), ("Cell", "02990109"), ("X", "58.060577"),
            ("Y", "240.199"), ("Z", "-44.606205"), ("Heading", "0"), ("Scale", ""))}
        self.status = tk.StringVar(value="Choose an entity and a position, then save a placement.")
        self.location_search = tk.StringVar()
        self.location_kind = tk.StringVar(value="All areas")
        self.location_count = tk.StringVar(value="Choose a DAT folder to browse the world.")
        self.location_id = tk.StringVar(value="02990109")
        self.selection_name = tk.StringVar(value="Choose an entity to place")
        self.move_step = tk.StringVar(value="1")
        self.bookmarks = [value for value in self.settings.get("bookmarks", []) if isinstance(value, dict)][:20]
        self.bookmark_name = tk.StringVar(value="My view")
        self.bookmark_camera = None
        self.keep_adding = tk.BooleanVar(value=False)
        self.snap_enabled = tk.BooleanVar(value=True)
        self.snap_size = tk.StringVar(value="1")
        self.object_tools = ObjectTools(self)
        self.catalog_view = tk.StringVar(value="All objects")
        self.recent_entities = [value for value in self.settings.get("recent_entities", []) if isinstance(value, str)][:24]
        self.favorite_entities = set(value for value in self.settings.get("favorite_entities", []) if isinstance(value, str))
        self.geometry = GeometryTools(self)
        self.build_ui()
        self.view3d.set_tool("explore")
        self.last_tool = "explore"
        self.view3d.tool.trace_add("write", lambda *unused: self.tool_changed())
        root.bind("<Control-s>", lambda event: self.save())
        root.bind("<Control-f>", lambda event: self.focus_catalog())
        root.bind("<Control-z>", lambda event: self.viewport_command("undo"))
        root.bind("<Control-y>", lambda event: self.viewport_command("redo"))
        root.bind("<Control-d>", lambda event: self.viewport_command("duplicate"))
        self.location_search.trace_add("write", lambda *unused: self.filter_locations())
        self.placement_search.trace_add("write", lambda *unused: self.filter_placements())
        self.search.trace_add("write", lambda *unused: self.filter_catalog())
        for variable in self.fields.values():
            variable.trace_add("write", lambda *unused: self.fields_changed())
        self.game.trace_add("write", lambda *unused: self.game_changed())
        self.load_catalog()
        self.reload()
        self.status.set("Click the world map to travel. Build objects opens the placement tools.")
        self.root.after_idle(self.load_world_atlas)
        self.root.after(300, self.load_locations)

    def build_ui(self):
        root = self.root
        style = ttk.Style(root)
        style.theme_use("clam")
        bg, panel, field, text, muted = "#151c26", "#202a38", "#111925", "#e5edf7", "#9fb1c7"
        root.configure(background=bg)
        style.configure(".", background=bg, foreground=text, font=("Segoe UI", 10))
        style.configure("TFrame", background=bg)
        style.configure("Panel.TFrame", background=panel)
        style.configure("TLabel", background=bg, foreground=text)
        style.configure("Muted.TLabel", foreground=muted)
        style.configure("Title.TLabel", font=("Segoe UI", 19, "bold"))
        style.configure("Section.TLabel", font=("Segoe UI", 11, "bold"))
        style.configure("TButton", background="#2d3d51", foreground=text, borderwidth=0, padding=(9, 6))
        style.map("TButton", background=[("active", "#405d7a"), ("disabled", "#202a38")], foreground=[("disabled", "#66788d")])
        style.configure("Accent.TButton", background="#246ea8", foreground="white", padding=(12, 9))
        style.map("Accent.TButton", background=[("active", "#318bcc"), ("disabled", "#334052")])
        style.configure("TEntry", fieldbackground=field, foreground=text, insertcolor="white", padding=5)
        style.configure("TSpinbox", fieldbackground=field, foreground=text, arrowcolor=text, padding=4)
        style.configure("TCombobox", fieldbackground=field, background=panel, foreground=text, arrowcolor=text, padding=4)
        style.map("TCombobox", fieldbackground=[("readonly", field)], foreground=[("readonly", text)])
        style.configure("Treeview", background=field, fieldbackground=field, foreground=text, rowheight=27, borderwidth=0)
        style.configure("Treeview.Heading", background="#2c3b4e", foreground=text, padding=5)
        style.map("Treeview", background=[("selected", "#245e87")], foreground=[("selected", "white")])
        style.configure("TNotebook", background=bg, borderwidth=0)
        style.configure("TNotebook.Tab", background=panel, foreground=muted, padding=(8, 6))
        style.map("TNotebook.Tab", background=[("selected", "#31475f")], foreground=[("selected", "white")])
        style.configure("TCheckbutton", background=bg, foreground=text)
        style.map("TCheckbutton", background=[("active", bg)])
        shell = ttk.Frame(root, padding=12)
        shell.pack(fill="both", expand=True)
        header = ttk.Frame(shell)
        self.header = header
        header.pack(fill="x", pady=(0, 10))
        ttk.Label(header, text="Pyrealium", style="Title.TLabel").pack(side="left")
        ttk.Label(header, text="WORLDBUILDER", style="Muted.TLabel").pack(side="left", padx=15)
        ttk.Button(header, text="Undo", command=self.undo).pack(side="right", padx=3)
        self.undo_button = header.winfo_children()[-1]
        self.redo_button = ttk.Button(header, text="Redo", command=self.redo)
        self.redo_button.pack(side="right", padx=3)
        ttk.Button(header, text="New / duplicate", command=self.new_placement).pack(side="right", padx=8)
        project = ttk.Frame(shell)
        self.project_frame = project
        if not self.game.get().strip():
            project.pack(fill="x", pady=(0, 10))
        ttk.Button(header, text="Project / DATs", command=self.toggle_project).pack(side="left", padx=8)
        self.travel_bar = ttk.Frame(shell)
        self.travel_bar.pack(fill="x", pady=(0, 10))
        project.columnconfigure(1, weight=1)
        self.path_row(project, 0, "DAT folder", self.game, self.pick_game)
        self.path_row(project, 1, "Content file", self.output, self.pick_output)
        self.panes = ttk.Panedwindow(shell, orient="horizontal")
        self.panes.pack(fill="both", expand=True)
        left, center, inspector = ttk.Frame(self.panes, width=300), ttk.Frame(self.panes), ttk.Frame(self.panes, width=260)
        self.panes.add(left, weight=0)
        self.panes.add(center, weight=1)
        self.panes.add(inspector, weight=0)
        self.inspector = inspector
        self.inspector_visible = True
        inspector.pack_propagate(False)
        left.pack_propagate(False)
        self.library = ttk.Notebook(left)
        self.library.pack(fill="both", expand=True, padx=(0, 8))
        self.page_map = ttk.Frame(self.library, padding=3)
        self.page_objects = entities = ttk.Frame(self.library, padding=5)
        self.page_scene = saved = ttk.Frame(self.library, padding=5)
        self.page_travel = ttk.Frame(self.library, padding=3)
        self.page_geometry = ttk.Frame(self.library, padding=4)
        for frame, label in ((self.page_map, "Map"), (entities, "Objects"), (saved, "Scene"), (self.page_travel, "Travel"), (self.page_geometry, "Build")):
            self.library.add(frame, text=label)
        travel_tabs = ttk.Notebook(self.page_travel)
        travel_tabs.pack(fill="both", expand=True)
        areas, nearby = ttk.Frame(travel_tabs, padding=3), ttk.Frame(travel_tabs, padding=3)
        travel_tabs.add(areas, text="Places")
        travel_tabs.add(nearby, text="Rooms / exits")
        self.travel_tabs = travel_tabs
        self.geometry.build_panel(self.page_geometry)
        self.minimap = WorldAtlas(self.page_map, self.navigate_to, compact=True)
        self.minimap.pack(fill="both", expand=True)
        bookmark_bar = ttk.Frame(self.page_map)
        bookmark_bar.pack(fill="x", pady=5)
        ttk.Entry(bookmark_bar, textvariable=self.bookmark_name, width=15).pack(side="left", fill="x", expand=True)
        ttk.Button(bookmark_bar, text="Bookmark", command=self.add_bookmark).pack(side="right")
        self.bookmark_tree = self.tree(self.page_map, ("View",), 5)
        self.bookmark_tree.master.pack_configure(expand=False)
        self.bookmark_tree.bind("<Double-1>", lambda event: self.open_bookmark())
        ttk.Button(self.page_map, text="Open bookmarked view", command=self.open_bookmark).pack(fill="x", pady=5)
        self.refresh_bookmarks()
        ttk.Label(areas, text="Explore an area", style="Section.TLabel").pack(anchor="w", pady=5)
        ttk.Entry(areas, textvariable=self.location_search).pack(fill="x", pady=4)
        ttk.Label(areas, text="Search a name, cell or landblock", style="Muted.TLabel").pack(anchor="w")
        kind = ttk.Combobox(areas, textvariable=self.location_kind, values=("All areas", "Outdoor", "Dungeon"), state="readonly")
        kind.pack(fill="x", pady=6)
        kind.bind("<<ComboboxSelected>>", lambda event: self.filter_locations())
        self.location_tree = self.tree(areas, ("Area", "Cell"), 14)
        self.location_tree.bind("<Double-1>", lambda event: self.open_location())
        self.location_tree.bind("<Return>", lambda event: self.open_location())
        footer = ttk.Frame(areas)
        footer.pack(side="bottom", fill="x", before=self.location_tree.master)
        ttk.Label(footer, textvariable=self.location_count, style="Muted.TLabel", wraplength=270).pack(anchor="w", pady=6)
        ttk.Button(footer, text="Open selected area", style="Accent.TButton", command=self.open_location).pack(fill="x", pady=4)
        direct = ttk.Frame(footer)
        direct.pack(fill="x", pady=(10, 0))
        ttk.Entry(direct, textvariable=self.location_id, width=12).pack(side="left", fill="x", expand=True)
        ttk.Button(direct, text="Go to cell", command=self.go_to_cell).pack(side="right", padx=(5, 0))
        ttk.Button(footer, text="Refresh world list", command=lambda: self.load_locations(force=True)).pack(fill="x", pady=5)
        ttk.Label(nearby, text="Connected areas", style="Section.TLabel").pack(anchor="w", pady=5)
        ttk.Label(nearby, text="Doorways follow native room portals. Outdoor links follow neighboring landblocks.", style="Muted.TLabel", wraplength=270).pack(anchor="w", pady=6)
        self.nearby_tree = self.tree(nearby, ("Area", "Cell"), 14)
        self.nearby_tree.bind("<Double-1>", lambda event: self.open_nearby())
        self.nearby_tree.bind("<Return>", lambda event: self.open_nearby())
        ttk.Button(nearby, text="Travel to selected area", command=self.open_nearby, style="Accent.TButton").pack(side="bottom", fill="x", pady=5, before=self.nearby_tree.master)
        ttk.Label(entities, text="Choose content to place", style="Section.TLabel").pack(anchor="w", pady=5)
        self.search_entry = ttk.Entry(entities, textvariable=self.search)
        self.search_entry.pack(fill="x", pady=4)
        self.matches = ttk.Label(entities, style="Muted.TLabel")
        self.matches.pack(anchor="w", pady=3)
        palette_bar = ttk.Frame(entities)
        palette_bar.pack(fill="x", pady=4)
        palette_mode = ttk.Combobox(palette_bar, textvariable=self.catalog_view, values=("All objects", "Recent", "Favourites"), state="readonly", width=15)
        palette_mode.pack(side="left", fill="x", expand=True)
        palette_mode.bind("<<ComboboxSelected>>", lambda event: self.filter_catalog())
        ttk.Button(palette_bar, text="Star / unstar", command=self.toggle_favorite).pack(side="right", padx=(4,0))
        self.entities = self.tree(entities, ("Entity", "Label"), 15)
        self.entities.configure(displaycolumns=("Label", "Entity"))
        self.entities.bind("<<TreeviewSelect>>", self.choose_entity)
        self.asset_swatch = ModelSwatch(entities)
        self.asset_swatch.pack(side="bottom", fill="x", pady=5, before=self.entities.master)
        ttk.Label(entities, text="Pick an object, hover over the world, then click to place. Escape finishes placing.", style="Muted.TLabel", wraplength=270).pack(anchor="w", pady=7)
        ttk.Entry(saved, textvariable=self.placement_search).pack(fill="x", pady=5)
        ttk.Checkbutton(saved, text="Current area only", variable=self.cell_only, command=self.filter_placements).pack(anchor="w")
        self.placement_count = ttk.Label(saved, style="Muted.TLabel")
        self.placement_count.pack(anchor="w", pady=4)
        self.objects = self.tree(saved, ("Name", "Object", "Cell"), 15)
        self.objects.configure(selectmode="extended")
        self.objects.bind("<<TreeviewSelect>>", self.choose_position)
        self.objects.bind("<Button-3>", self.scene_context_menu)
        ttk.Button(saved, text="Arrange selection / spaced copies", command=self.open_arrange).pack(side="bottom", fill="x", pady=5, before=self.objects.master)
        ttk.Button(saved, text="Reload content file", command=self.reload).pack(fill="x", pady=6)
        self.workspace = ttk.Notebook(center)
        self.workspace.pack(fill="both", expand=True, padx=6)
        self.view3d = Preview3D(self.workspace, on_pick=self.pick_3d, on_camera_move=self.camera_moved, on_select=self.select_in_view, on_command=self.viewport_command, on_transform=self.object_tools.transform, on_stamp=self.object_tools.stamp, on_surface=self.geometry.stroke, on_door=self.geometry.pick_door)
        self.view3d.canvas.configure(height=280)
        self.move_step = self.view3d.move_step
        map_frame = ttk.Frame(self.workspace)
        self.atlas = WorldAtlas(self.workspace, self.navigate_to)
        self.workspace.add(self.atlas, text="World map")
        self.workspace.add(self.view3d, text="Explore in 3D")
        self.workspace.add(map_frame, text="Position map")
        bar = ttk.Frame(map_frame)
        bar.pack(fill="x", pady=5)
        ttk.Label(bar, text="Click to move · right-drag to pan · wheel to zoom", style="Muted.TLabel").pack(side="left")
        ttk.Button(bar, text="Fit cell", command=self.fit_map).pack(side="right")
        self.canvas = tk.Canvas(map_frame, background=field, highlightthickness=0, height=280)
        self.canvas.pack(fill="both", expand=True)
        self.canvas.bind("<Configure>", lambda event: self.preview())
        self.canvas.bind("<Button-1>", self.pick_position)
        self.canvas.bind("<MouseWheel>", self.zoom_map)
        self.canvas.bind("<ButtonPress-3>", self.begin_pan)
        self.canvas.bind("<B3-Motion>", self.pan_map)
        actions = ttk.Frame(inspector)
        actions.pack(side="bottom", fill="x", padx=8, pady=6)
        self.save_button = ttk.Button(actions, text="Add placement  /  Ctrl+S", style="Accent.TButton", command=self.save)
        self.save_button.pack(fill="x", pady=4)
        self.delete_button = ttk.Button(actions, text="Delete selected", command=self.delete_selected, state="disabled")
        self.delete_button.pack(fill="x", pady=4)
        ttk.Label(inspector, text="PLACEMENT", style="Section.TLabel").pack(anchor="w", padx=10, pady=(10, 7))
        ttk.Label(inspector, textvariable=self.selection_name, wraplength=240).pack(anchor="w", padx=10)
        ttk.Label(inspector, textvariable=self.mode, style="Muted.TLabel", wraplength=240).pack(anchor="w", padx=10, pady=(4, 10))
        inspector_tabs = ttk.Notebook(inspector)
        inspector_tabs.pack(fill="both", expand=True, padx=6, pady=6)
        transform_tab, tools_tab = ttk.Frame(inspector_tabs), ttk.Frame(inspector_tabs)
        inspector_tabs.add(transform_tab, text="Transform")
        inspector_tabs.add(tools_tab, text="Tools")
        form = ttk.Frame(transform_tab, padding=(3, 5))
        form.pack(fill="x")
        form.columnconfigure(1, weight=1)
        self.transform_entries = {}
        for index, (name, variable) in enumerate(self.fields.items()):
            ttk.Label(form, text=name).grid(row=index, column=0, sticky="w", padx=3, pady=4)
            if name in ("X", "Y", "Z", "Heading", "Scale"):
                low, high, step = ((0, 255, 1) if name == "Heading" else (.1, 100, .1) if name == "Scale" else (-999999, 999999, 1))
                entry = ttk.Spinbox(form, textvariable=variable, from_=low, to=high, increment=step, width=15)
            else:
                entry = ttk.Entry(form, textvariable=variable, width=15)
            self.transform_entries[name] = entry
            entry.grid(row=index, column=1, sticky="ew", padx=3, pady=4)
        ttk.Label(transform_tab, text="Heading 0–255 · blank scale = native size", style="Muted.TLabel", wraplength=230).pack(anchor="w", padx=10, pady=8)
        nudge = ttk.Frame(tools_tab, padding=8)
        nudge.pack(fill="x")
        ttk.Label(nudge, text="Move by").grid(row=0, column=0, sticky="w")
        ttk.Spinbox(nudge, textvariable=self.move_step, values=("0.1", "0.25", "0.5", "1", "5", "10"), width=7).grid(row=0, column=1, columnspan=2, sticky="ew", pady=3)
        for row, axis in enumerate(("X", "Y", "Z"), 1):
            ttk.Label(nudge, text=axis).grid(row=row, column=0, sticky="w", padx=3)
            ttk.Button(nudge, text="−", width=4, command=lambda a=axis: self.nudge(a, -1)).grid(row=row, column=1, padx=3, pady=2)
            ttk.Button(nudge, text="+", width=4, command=lambda a=axis: self.nudge(a, 1)).grid(row=row, column=2, padx=3, pady=2)
        ttk.Button(tools_tab, text="Focus selected  ·  F", command=self.view3d.focus_placement).pack(fill="x", padx=10, pady=5)
        ttk.Button(tools_tab, text="Reload 3D assets", command=self.retry_scene).pack(fill="x", padx=10, pady=3)
        ttk.Checkbutton(tools_tab, text="Keep adding copies", variable=self.keep_adding).pack(anchor="w", padx=10, pady=(15, 4))
        ttk.Label(tools_tab, text="Move the draft, then Ctrl+S to stamp another object. Each saved copy gets a new ID.", style="Muted.TLabel", wraplength=220).pack(anchor="w", padx=10, pady=4)
        ttk.Button(tools_tab, text="Revert unsaved transform", command=self.revert_draft).pack(fill="x", padx=10, pady=10)
        self.render_status = tk.StringVar(value="Choose an area in World, then an entity to place.")
        ttk.Label(shell, textvariable=self.render_status, style="Muted.TLabel", wraplength=1250).pack(anchor="w", pady=(8, 3))
        ttk.Label(shell, textvariable=self.status, wraplength=1250).pack(anchor="w", pady=3)
        ttk.Button(self.travel_bar, text="World map", command=self.show_world_map, style="Accent.TButton").pack(side="left", padx=(0, 5))
        ttk.Button(self.travel_bar, text="Fly / explore", command=self.start_exploring).pack(side="left", padx=3)
        ttk.Button(self.travel_bar, text="Back", command=lambda: self.travel_history(-1)).pack(side="left", padx=(15, 3))
        ttk.Button(self.travel_bar, text="Forward", command=lambda: self.travel_history(1)).pack(side="left", padx=3)
        ttk.Button(self.travel_bar, text="Dungeons", command=self.show_dungeons).pack(side="left", padx=8)
        ttk.Button(self.travel_bar, text="Terrain / rooms", command=self.show_geometry).pack(side="left", padx=3)
        ttk.Button(self.travel_bar, text="Build objects", command=self.show_building).pack(side="right", padx=3)
        ttk.Checkbutton(self.travel_bar, text="Snap", variable=self.snap_enabled).pack(side="left", padx=(10,2))
        ttk.Spinbox(self.travel_bar, textvariable=self.snap_size, values=("0.1","0.25","0.5","1","2","5"), width=4).pack(side="left", padx=(0,4))
        ttk.Label(self.travel_bar, text="Fly speed").pack(side="left", padx=(12, 3))
        ttk.Spinbox(self.travel_bar, textvariable=self.view3d.travel_speed, from_=1, to=500, increment=10, width=5).pack(side="left")
        self.show_inspector(False)
        self.library.select(self.page_map)
        self.root.after(100, self.size_panes)

    def scene_context_menu(self, event):
        item = self.objects.identify_row(event.y)
        if not item:
            return
        if item not in self.objects.selection():
            self.objects.selection_set(item)
            self.choose_position(None)
        menu = tk.Menu(self.root, tearoff=False)
        menu.add_command(label="Frame selection  F", command=self.view3d.focus_placement)
        for tool in ("move", "rotate", "scale"):
            menu.add_command(label=tool.title(), command=lambda value=tool: self.view3d.set_tool(value))
        menu.add_separator()
        menu.add_command(label="Duplicate  Ctrl+D", command=self.object_tools.duplicate)
        menu.add_command(label="Delete", command=self.delete_selected)
        menu.tk_popup(event.x_root, event.y_root)

    def refresh_bookmarks(self):
        self.bookmark_tree.delete(*self.bookmark_tree.get_children())
        for index, row in enumerate(self.bookmarks):
            self.bookmark_tree.insert("", "end", iid=str(index), values=(str(row.get("name", "View")),))

    def add_bookmark(self):
        if self.active_scene_cell is None:
            self.status.set("Travel to an area before bookmarking its camera view.")
            return
        cell, focus = self.active_scene_cell, list(self.view3d.focus)
        if self.outdoor_grid:
            from preview_terrain import placement_from_point
            cell, focus = placement_from_point(cell, focus, self.outdoor_grid)
        name = self.bookmark_name.get().strip()[:80] or f"View {len(self.bookmarks)+1}"
        self.bookmarks = [row for row in self.bookmarks if row.get("name") != name]
        self.bookmarks.append(dict(name=name, cell=cell, focus=focus, azimuth=self.view3d.azimuth,
                                   elevation=self.view3d.elevation, distance=self.view3d.distance))
        self.bookmarks = self.bookmarks[-20:]
        self.refresh_bookmarks()
        self.status.set(f"Bookmarked {name}. The view is remembered when the editor closes.")

    def open_bookmark(self):
        selected = self.bookmark_tree.selection()
        if not selected:
            return
        try:
            row = self.bookmarks[int(selected[0])]
            cell, focus = int(row["cell"]), tuple(float(v) for v in row["focus"])
            values = (float(row["azimuth"]), float(row["elevation"]), float(row["distance"]))
            if len(focus) != 3 or not all(math.isfinite(v) for v in focus+values) or not .5 <= values[2] <= 5000:
                raise ValueError("Invalid stored camera view")
            self.bookmark_camera = (cell, focus, *values)
            self.navigate_to(cell, list(focus))
        except (ValueError, KeyError, TypeError) as error:
            self.status.set(f"Bookmark could not open: {error}")

    def frame_arrival(self, instances, point):
        if any(source.get("terrain") for source in instances):
            # A fixed orbit offset can sit inside the uphill side of the terrain.
            # Start above all nearby ground, then let the user fly from there.
            high = float(point[2])
            for source in instances:
                if not source.get("terrain"):
                    continue
                origin = source.get("position", (0, 0, 0))
                for vertex in source["mesh"]["vertices"]:
                    x, y, z = (vertex[i]+origin[i] for i in range(3))
                    if (x-point[0])**2+(y-point[1])**2 <= 100**2:
                        high = max(high, z)
            focus = (point[0], point[1], point[2]+3)
            rise = max(65, high-focus[2]+25)
            self.view3d.focus = focus
            self.view3d.azimuth = -math.pi/2
            self.view3d.elevation = math.atan2(rise, 45)
            self.view3d.distance = math.hypot(rise, 45)
        elif self.view3d.tool.get() == "dungeon":
            self.view3d.fit()
            self.view3d.elevation=math.radians(48)
        else:
            self.view3d.focus_placement()
        self.view3d.redraw()

    def toggle_project(self):
        if self.project_frame.winfo_manager():
            self.project_frame.pack_forget()
        else:
            self.project_frame.pack(fill="x", pady=(0, 10), after=self.header)

    def show_inspector(self, visible):
        if self.inspector_visible == visible:
            return
        self.inspector_visible = visible
        if visible:
            self.panes.add(self.inspector, weight=0)
        else:
            self.panes.forget(self.inspector)
        self.root.after_idle(self.size_panes)

    def viewport_command(self, command):
        if command=="cancel" and self.geometry.building.capture is not None:
            self.geometry.building.capture=None
            self.view3d.set_tool("explore")
            self.status.set("Entrance placement cancelled.")
            return
        if command in ("undo", "redo", "delete", "duplicate", "cancel"):
            self.view3d.cancel_transform()
            self.object_tools.cancel()
        actions = {"save": self.save, "search": self.focus_catalog, "undo": self.undo,
                   "redo": self.redo, "delete": self.delete_selected, "duplicate": self.object_tools.duplicate}
        if command in actions:
            actions[command]()
        elif command == "cancel" and self.view3d.tool.get() == "place":
            self.selected_id, self.group_ids = None, ()
            self.set_fields(dict(Entity=""))
            self.baseline = self.field_snapshot()
            self.view3d.set_tool("select")
            self.status.set("Placement finished. Select an object to edit it, or choose another from the palette.")

    def show_world_map(self):
        self.view3d.stop_travel()
        self.workspace.select(0)
        self.show_inspector(False)
        self.atlas.draw()
        self.library.select(self.page_map)

    def show_geometry(self):
        self.library.select(self.page_geometry)
        self.show_inspector(False)
        self.workspace.select(1)

    def show_building(self):
        self.library.select(self.page_objects)
        self.show_inspector(True)
        self.workspace.select(1)
        self.search_entry.focus_set()

    def show_dungeons(self):
        self.location_kind.set("Dungeon")
        self.location_search.set("")
        self.filter_locations()
        self.library.select(self.page_travel)
        self.status.set("Choose an interior area from Places, then Open selected area. Exits follows connected rooms.")

    def start_exploring(self):
        if not self.has_travelled:
            location = next((row for row in self.locations if row["kind"] == "Outdoor" and row.get("position") is not None), None)
            if location:
                self.navigate_to(location["cell"], location["position"])
                return
            self.show_world_map()
            self.status.set("Choose a destination on the world map while the saved places list loads.")
            return
        self.view3d.set_tool("explore")
        self.workspace.select(1)
        self.view3d.canvas.focus_set()
        self.status.set("Hold W/A/S/D to fly. Q/E change height. Drag to look. World map travels to another region.")

    def travel_history(self, direction):
        target = self.navigation_index + direction
        if not 0 <= target < len(self.navigation_history):
            self.status.set("No earlier destination." if direction < 0 else "No later destination.")
            return
        self.history_target = target
        self.navigate_to(*self.navigation_history[target], from_history=True)

    def load_world_atlas(self):
        folder = self.game.get().strip()
        if self.closing or not folder or self.atlas_future is not None or self.atlas_folder == folder:
            return
        self.atlas_folder = folder
        self.atlas.note.set("Reading the native AC2 world map…")
        self.atlas_future = self.executor.submit(load_atlas, folder)
        self.root.after(60, self.finish_world_atlas)

    def finish_world_atlas(self):
        if self.closing or self.atlas_future is None:
            return
        if not self.atlas_future.done():
            self.root.after(60, self.finish_world_atlas)
            return
        future, self.atlas_future = self.atlas_future, None
        try:
            data = future.result()
            if self.atlas_folder != self.game.get().strip():
                self.atlas_folder = None
                self.load_world_atlas()
                return
            self.atlas.set_atlas(data)
            self.minimap.set_atlas(data)
        except Exception as error:
            logging.exception("World atlas unavailable")
            self.atlas.note.set(f"World map unavailable: {error}. Use Project / DATs to select the game folder.")
            self.atlas.draw()

    def size_panes(self):
        if not self.closing:
            width = self.panes.winfo_width()
            if width > 800:
                self.panes.sashpos(0, 330)
                if self.inspector_visible:
                    self.panes.sashpos(1, width - 265)

    def focus_catalog(self):
        self.library.select(self.page_objects)
        self.search_entry.focus_set()

    def nudge(self, axis, direction):
        try:
            step = float(self.move_step.get())
            if not math.isfinite(step) or not 0 < step <= 1000:
                raise ValueError("Move step must be greater than zero and at most 1000.")
            value = float(self.fields[axis].get()) + direction * step
            if not math.isfinite(value) or abs(value) >= 1_000_000:
                raise ValueError("Position is outside the supported range.")
            self.fields[axis].set(format(value, ".9g"))
        except ValueError as error:
            self.status.set(str(error))

    def revert_draft(self):
        if self.baseline is not None:
            self.set_fields(self.baseline)
            self.status.set("Reverted the unsaved transform. The content file was unchanged.")

    def retry_scene(self):
        self.retry_revision += 1
        self.scene_ready = None
        self.refresh_scene()

    def game_changed(self):
        self.preview()
        if self.location_timer is not None:
            self.root.after_cancel(self.location_timer)
        self.location_timer = self.root.after(700, self.refresh_location_folder)

    def refresh_location_folder(self):
        self.location_timer = None
        self.load_world_atlas()
        self.load_locations()

    @staticmethod
    def tree(parent, columns, height):
        frame = ttk.Frame(parent)
        frame.pack(fill="both", expand=True)
        frame.columnconfigure(0, weight=1)
        frame.rowconfigure(0, weight=1)
        tree = ttk.Treeview(frame, columns=columns, show="headings", height=height, selectmode="browse")
        vertical = ttk.Scrollbar(frame, orient="vertical", command=tree.yview)
        horizontal = ttk.Scrollbar(frame, orient="horizontal", command=tree.xview)
        tree.configure(yscrollcommand=vertical.set, xscrollcommand=horizontal.set)
        tree.grid(row=0, column=0, sticky="nsew")
        vertical.grid(row=0, column=1, sticky="ns")
        horizontal.grid(row=1, column=0, sticky="ew")
        for column in columns:
            tree.heading(column, text=column)
            tree.column(column, width=160 if column in ("Label", "Name", "Area") else (145 if column == "Object" else 85), minwidth=50)
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
            self.load_locations(force=True)

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
        if self.catalog_view.get() == "Favourites":
            rows = [row for row in rows if row[0] in self.favorite_entities]
        elif self.catalog_view.get() == "Recent":
            recent = {value: index for index, value in enumerate(self.recent_entities)}
            rows = sorted((row for row in rows if row[0] in recent), key=lambda row: recent[row[0]])
        self.entities.delete(*self.entities.get_children())
        for index, (entity, visual, label) in enumerate(rows[:500]):
            self.entities.insert("", "end", iid=str(index), values=(entity, label))
        self.matches.configure(text=f"{len(rows):,} matches · showing up to 500")

    def toggle_favorite(self):
        entity = self.fields["Entity"].get().strip().lower()
        if not entity:
            return
        if entity in self.favorite_entities:
            self.favorite_entities.remove(entity)
        else:
            self.favorite_entities.add(entity)
        self.filter_catalog()
        self.status.set("Object palette favourites updated.")

    def choose_entity(self, event):
        selected = self.entities.selection()
        if selected:
            if not self.has_travelled:
                self.show_world_map()
                self.status.set("Choose a destination on the map first, then pick an object to build there.")
                return
            entity = self.entities.item(selected[0], "values")[0]
            self.selected_id, self.group_ids = None, ()
            self.objects.selection_remove(*self.objects.selection())
            self.view3d.set_tool("place")
            self.fields["Entity"].set(entity)
            self.baseline = self.field_snapshot()
            self.recent_entities = [entity]+[value for value in self.recent_entities if value != entity][:23]
            self.update_actions()
            self.workspace.select(1)
            self.status.set("Hover over a world surface for the placement preview; click to place. Ctrl+Z undoes. Escape finishes.")

    def field_snapshot(self):
        return {name: variable.get() for name, variable in self.fields.items()}

    def discard_draft(self):
        return (not self.fields["Entity"].get().strip() or self.baseline is None or self.field_snapshot() == self.baseline or
                messagebox.askyesno("Unsaved placement", "Discard the unsaved field changes?", parent=self.root))

    def fields_changed(self):
        if self.changing_fields:
            return
        if self.view3d.tool.get() in ("terrain", "dungeon", "entrance") or self.library.select()==str(self.page_geometry):
            self.save_button.configure(text="Save world project",state="normal")
            self.preview()
            return
        for entry in self.transform_entries.values():
            entry.configure(state="disabled" if self.group_ids else "normal")
        if self.group_ids:
            self.view3d.placement_enabled = False
            self.selection_name.set(f"{len(self.group_ids)} selected objects")
            self.mode.set("Group selection / arrange together")
            self.save_button.configure(state="normal", text="Arrange selected objects")
            self.delete_button.configure(state="normal")
            self.preview()
            return
        dirty = bool(self.fields["Entity"].get().strip()) and self.baseline is not None and self.field_snapshot() != self.baseline
        entity = self.fields["Entity"].get().strip().lower()
        self.selection_name.set(next((row[2] for row in self.catalog if row[0].lower() == entity), "Choose an entity to place"))
        valid_entity = any(row[0].lower() == entity for row in self.catalog)
        self.view3d.placement_enabled = valid_entity and self.selected_id is None
        self.save_button.configure(state="normal" if valid_entity else "disabled")
        self.mode.set((f"Editing object {self.selected_id:016x}" if self.selected_id is not None else "New placement")
                      + (" · unsaved changes" if dirty else ""))
        if self.cell_only.get():
            self.filter_placements()
        self.preview()

    def filter_placements(self):
        if not hasattr(self, "objects"):
            return
        query = self.placement_search.get().strip().casefold()
        labels = {entity.lower(): label for entity, visual, label in self.catalog}
        selected = self.group_ids or ((self.selected_id,) if self.selected_id is not None else ())
        self.objects.delete(*self.objects.get_children())
        count = 0
        for row in self.placements:
            label = labels.get(row[1].lower(), row[1])
            if query not in " ".join([label] + row[:4]).casefold():
                continue
            if self.cell_only.get():
                try:
                    cell = int(row[3], 16)
                    reference = self.active_scene_cell or int(self.fields["Cell"].get(), 16)
                    if self.active_scene_cell is not None and 1 <= (reference & 65535) <= 64:
                        if abs((cell >> 24)-(reference >> 24)) > 1 or abs(((cell >> 16)&255)-((reference >> 16)&255)) > 1 or (cell & 65535) > 64:
                            continue
                    elif cell != reference and cell not in {item.get("cell") for item in self.view3d.instances}:
                        continue
                except ValueError:
                    continue
            self.objects.insert("", "end", iid=row[0], values=(label, row[0], row[3]))
            count += 1
        visible_selection = [row[0] for row in self.placements if int(row[0], 16) in selected and self.objects.exists(row[0])]
        if visible_selection:
            self.objects.selection_set(visible_selection)
        self.placement_count.configure(text=f"{count:,} shown / {len(self.placements):,} saved")

    def reload(self, force=False):
        if not force and not self.discard_draft():
            if self.loaded_path:
                self.output.set(str(self.loaded_path))
            return
        try:
            path = Path(self.output.get()).resolve()
            original = read_document(path)
            rows = rows_from_text(original)
            if self.loaded_path != path or not force:
                self.history.clear()
                self.redo_history.clear()
            self.loaded_path, self.original = path, original
            self.root.title(f"Pyrealium WorldBuilder — {path.name}")
            self.placements = rows
            self.selected_id = None
            self.group_ids = ()
            self.baseline = self.field_snapshot()
            self.content_revision += 1
            self.filter_placements()
            self.update_actions()
            self.status.set(f"Loaded {len(rows):,} placements. Select one to edit, or choose an entity to add.")
            self.preview()
        except (OSError, ValueError) as error:
            if self.loaded_path:
                self.output.set(str(self.loaded_path))
            self.status.set(str(error))
            messagebox.showerror("Content could not be loaded", str(error), parent=self.root)

    def update_actions(self):
        self.save_button.configure(text="Update placement  ·  Ctrl+S" if self.selected_id is not None else "Add placement  ·  Ctrl+S")
        self.delete_button.configure(state="normal" if self.selected_id is not None else "disabled")
        self.undo_button.configure(state="normal" if self.history else "disabled")
        self.redo_button.configure(state="normal" if self.redo_history else "disabled")
        self.fields_changed()

    def new_placement(self):
        if self.group_ids:
            self.open_arrange(duplicate=True)
            return
        # Keep the form as a duplicate draft; adding always allocates a fresh IID.
        self.selected_id = None
        self.objects.selection_remove(*self.objects.selection())
        self.baseline = self.field_snapshot()
        self.update_actions()
        self.status.set("New placement draft. Choose an entity or move this copy, then Add placement.")

    def choose_position(self, event):
        selected = self.objects.selection()
        if not selected:
            return
        identities = tuple(sorted(int(value, 16) for value in selected))
        if len(identities) > 1:
            if identities == self.group_ids:
                return
            if not self.discard_draft():
                return
            self.group_ids = identities
            self.selected_id = None
            self.view3d.set_tool("select")
            row = next(row for row in self.placements if int(row[0], 16) == identities[0])
            values = dict(zip(("Entity", "Cell", "X", "Y", "Z"), row[1:2] + row[3:7]))
            values["Heading"] = str(heading_from_row(row))
            values["Scale"] = row[11] if len(row) == 12 and row[11] != "-" else ""
            self.set_fields(values)
            self.baseline = self.field_snapshot()
            self.update_actions()
            self.view3d.set_selection(identities)
            return
        identity = identities[0]
        if identity == self.selected_id and not self.group_ids:
            return
        if not self.discard_draft():
            if self.selected_id is not None:
                previous = next((r[0] for r in self.placements if int(r[0], 16) == self.selected_id), None)
                if previous and self.objects.exists(previous):
                    self.objects.selection_set(previous)
            return
        row = next(row for row in self.placements if int(row[0], 16) == identity)
        self.group_ids = ()
        self.view3d.set_tool("place")
        self.selected_id = identity
        values = dict(zip(("Entity", "Cell", "X", "Y", "Z"), row[1:2] + row[3:7]))
        values["Heading"] = str(heading_from_row(row))
        values["Scale"] = row[11] if len(row) == 12 and row[11] != "-" else ""
        self.set_fields(values)
        self.baseline = self.field_snapshot()
        self.update_actions()

    def set_fields(self, values):
        self.changing_fields = True
        try:
            for name,value in values.items():
                self.fields[name].set(value)
        finally:
            self.changing_fields = False
        self.fields_changed()

    def preview(self):
        if self.changing_fields:
            return
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
            self.canvas.create_text(sx, sy+17, text="Editing selected" if self.selected_id is not None else "New placement", fill="#efb65e")
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
        cell = self.view3d.picked_cell or self.active_scene_cell
        if self.outdoor_grid and self.active_scene_cell is not None:
            from preview_terrain import placement_from_point
            try:
                cell, point = placement_from_point(self.active_scene_cell, point, self.outdoor_grid)
            except ValueError as error:
                self.status.set(str(error))
                return
        values = {name:format(value,".9g") for name,value in zip(("X","Y","Z"),point)}
        if cell is not None:
            values["Cell"] = f"{cell:08x}"
        self.set_fields(values)
        self.status.set(f"Draft moved to cell {values.get('Cell', self.fields['Cell'].get())} · "
                        f"X {values['X']}  Y {values['Y']}  Z {values['Z']}. Save placement to apply.")

    def position_in_scene(self, cell, position, reference):
        if self.outdoor_grid and 1 <= (cell & 65535) <= 64 and 1 <= (reference & 65535) <= 64:
            position = list(position)
            side = self.outdoor_grid["block_side"]
            position[0] += ((cell >> 24) - (reference >> 24)) * side
            position[1] += (((cell >> 16) & 255) - ((reference >> 16) & 255)) * side
        return position

    def refresh_scene(self):
        if self.closing:
            return
        if self.view3d.tool.get() == "explore" and not self.has_travelled and self.explore_cell is None:
            self.wanted = None
            return
        try:
            folder = self.game.get().strip()
            if folder:
                self.geometry.ensure_source(folder)
            if not folder:
                self.render_status.set("Choose a DAT folder, then open an area from World.")
                self.wanted = None
                return
            entity_text = self.fields["Entity"].get().strip()
            entity = int(entity_text, 16) if entity_text else 0
            visuals = {int(row[1], 16) for row in self.catalog if int(row[0], 16) == entity}
            visual = next(iter(visuals)) if len(visuals) == 1 else 0
            cell = int(self.fields["Cell"].get(), 16)
            if not 0 < cell <= 0xffffffff or not (1 <= (cell & 65535) <= 64 or 256 <= (cell & 65535) <= 65531):
                raise ValueError("Cell must identify an outdoor cell or dungeon room.")
            position = [float(self.fields[name].get()) for name in ("X", "Y", "Z")]
            heading = int(self.fields["Heading"].get())
            scale_text = self.fields["Scale"].get().strip()
            scale = float(scale_text) if scale_text else None
            if not all(math.isfinite(v) and abs(v) < 1_000_000 for v in position) or not 0 <= heading <= 255:
                raise ValueError("Enter a valid position and heading to update the preview.")
            if scale is not None and (not math.isfinite(scale) or not 0 < scale <= 100):
                raise ValueError("Scale must be above zero and at most 100.")
            exploring = self.view3d.tool.get() in ("explore", "terrain", "dungeon")
            if self.group_ids:
                visual = 0
            if exploring:
                cell = self.explore_cell or cell
                position = list(self.explore_position) if self.explore_position is not None else position
                visual, heading, scale = 0, 0, None
            reference = cell
            # Outdoor coordinates are block-local. Keep the loaded neighborhood's
            # frame while dragging through its cells; translate the draft into it.
            if (not exploring and self.active_scene_cell is not None and self.outdoor_grid and self.scene_ready
                    and self.scene_ready[0] == folder and 1 <= (cell & 65535) <= 64
                    and abs((cell >> 24) - (self.active_scene_cell >> 24)) <= 1
                    and abs(((cell >> 16) & 255) - ((self.active_scene_cell >> 16) & 255)) <= 1):
                reference = self.active_scene_cell
            position = self.position_in_scene(cell, position, reference)
            if self.active_scene_cell == reference:
                self.view3d.update_placement(position, heading, scale)
            selected_id = None if exploring else self.selected_id
            key = (folder, reference, visual, (self.content_revision, selected_id, self.group_ids, self.geometry.project.revision, self.retry_revision))
            placements = [row for row in self.placements if int(row[0], 16) != selected_id]
            self.wanted = (key, position, heading, scale, placements)
            if self.future is None and key != self.scene_ready and self.scene_timer is None:
                self.scene_timer = self.root.after(180, self.start_load)
        except (ValueError, OverflowError) as error:
            self.wanted = None
            self.render_status.set(f"Preview paused: {error}")

    def start_load(self):
        self.scene_timer = None
        if self.closing or self.wanted is None or self.future is not None:
            return
        request = self.wanted
        if request[0] == self.scene_ready:
            return
        logging.info("Loading DAT scene: folder=%s cell=%08x visual=%08x", *request[0][:3])
        self.render_status.set("Loading area geometry and materials…")
        self.future = self.executor.submit(self.load_scene, request)
        self.loading = request
        self.root.after(60, self.finish_load)

    def load_scene(self, request):
        key, position, heading, scale, placements = request
        folder, cell, visual, revision = key
        self.geometry.reader(folder)
        if revision[-1] != getattr(self, "assets_retry_revision", None):
            # Retry is an explicit request to re-read failed material/mesh records.
            self.assets.cache.clear()
            self.assets.materials.clear()
            self.assets_retry_revision = revision[-1]
        candidates = {}
        for entity, target, label in self.catalog:
            candidates.setdefault(int(entity, 16), set()).add(int(target, 16))
        catalog = {entity: next(iter(targets)) for entity, targets in candidates.items() if len(targets) == 1}
        from world_bundle import entrance_objects
        ids={row[0] for row in placements}
        placements=list(placements)+[row for row in entrance_objects(self.geometry.project.entrances) if row[0] not in ids]
        return self.assets.scene(cell, visual, position, heading, scale, placements, catalog)

    def finish_load(self):
        if self.closing or self.future is None:
            return
        if not self.future.done():
            self.root.after(60, self.finish_load)
            return
        request = self.loading
        future, self.future = self.future, None
        key, position, heading, scale, placements = request
        try:
            instances, warnings = future.result()
        except Exception as error:
            logging.exception("Scene load failed")
            if self.wanted and self.wanted[0] == key:
                self.scene_ready = key
                self.view3d.set_scene([], position, [str(error)], heading=heading)
                self.render_status.set(f"Area could not load: {error}. Check DAT folder or use Reload 3D assets.")
            if self.wanted and self.wanted[0] != key:
                self.start_load()
            return
        if not self.wanted or self.wanted[0] != key:
            if self.wanted:
                self.start_load()
            return
        prior = self.scene_ready
        preserve = bool(prior and prior[0] == key[0] and prior[1] >> 16 == key[1] >> 16 and prior[2] == key[2])
        streamed_view = None
        if self.stream_camera and self.stream_camera[0] == key[1] and self.view3d.tool.get() == "explore":
            focus = self.stream_camera[1]
            if self.stream_origin_focus is not None:
                focus = tuple(focus[i] + self.view3d.focus[i] - self.stream_origin_focus[i] for i in range(3))
            streamed_view = (focus, self.view3d.azimuth, self.view3d.elevation, self.view3d.distance)
        self.scene_ready = key
        self.active_scene_cell = key[1]
        self.outdoor_grid = next((row["outdoor_grid"] for row in instances if "outdoor_grid" in row), None)
        self.view3d.set_scene(instances, position, warnings, heading=heading, preserve_camera=preserve)
        self.view3d.portal_markers=[]
        for pair in self.geometry.project.entrances:
            for route in (pair["entrance"],pair["return"]):
                source=route["source"];cell=source["cell"];point=list(source["position"])
                if self.outdoor_grid and 1<=cell&65535<=64:
                    if abs((cell>>24)-(key[1]>>24))>1 or abs(((cell>>16)&255)-((key[1]>>16)&255))>1:continue
                    point[0]+=((cell>>24)-(key[1]>>24))*self.outdoor_grid["block_side"]
                    point[1]+=(((cell>>16)&255)-((key[1]>>16)&255))*self.outdoor_grid["block_side"]
                elif cell>>16!=key[1]>>16 or cell&65535<256:continue
                self.view3d.portal_markers.append(dict(position=point,name=pair["name"],destination=route["destination"]))
        self.view3d.redraw()
        self.view3d.update_placement(self.wanted[1], self.wanted[2], self.wanted[3])
        if self.selected_id is not None:
            for item in self.view3d.instances:
                if item["role"] == "preview":
                    item["object_id"] = self.selected_id
        self.view3d.set_selection(self.group_ids or ([self.selected_id] if self.selected_id is not None else []))
        self.asset_swatch.set_model(self.view3d.instances, self.selection_name.get())
        if streamed_view is not None:
            focus, azimuth, elevation, distance = streamed_view
            self.view3d.focus, self.view3d.azimuth = focus, azimuth
            self.view3d.elevation, self.view3d.distance = elevation, distance
            self.stream_camera = None
            self.stream_origin_focus = None
            self.view3d.redraw()
        if self.selection_camera is not None and self.view3d.tool.get() in ("select", "move", "rotate", "scale"):
            self.view3d.focus, self.view3d.azimuth, self.view3d.elevation, self.view3d.distance = self.selection_camera
            self.selection_camera = None
            self.view3d.redraw()
        if self.arrival_cell == key[1] and self.view3d.tool.get() in ("explore", "dungeon"):
            self.frame_arrival(instances, self.wanted[1])
            self.arrival_cell = None
            self.view3d.canvas.focus_set()
        if self.bookmark_camera and self.bookmark_camera[0] == key[1]:
            unused, focus, azimuth, elevation, distance = self.bookmark_camera
            self.view3d.focus, self.view3d.azimuth, self.view3d.elevation, self.view3d.distance = focus, azimuth, elevation, distance
            self.bookmark_camera = None
            self.view3d.redraw()
        self.filter_placements()
        self.request_links(key[1])
        if self.geometry.pending_room == key[1]:
            self.geometry.pending_room=None
            self.geometry.inspect_room()
        count = sum(len(row["mesh"]["indices"]) // 3 for row in instances)
        logging.info("Scene loaded: meshes=%d triangles=%d notes=%d", len(instances), count, len(warnings))
        self.render_status.set(f"Cell {key[1]:08x} · {len(instances)} meshes · {count:,} triangles"
                               + (f" · {len(warnings)} asset notes in Details" if warnings else " · all requested assets loaded"))

    def load_locations(self, force=False):
        if self.closing:
            return
        folder = self.game.get().strip()
        if not folder or (not force and self.locations_folder == folder):
            return
        if self.locations_future is not None:
            self.root.after(150, lambda: self.load_locations(force))
            return
        from editor_locations import build_locations
        self.locations_folder = folder
        self.location_count.set("Reading the world's area index…")
        self.locations_future = self.executor.submit(build_locations, folder, Path(self.args.catalog).parent, list(self.catalog))
        self.root.after(80, self.finish_locations)

    def finish_locations(self):
        if self.closing or self.locations_future is None:
            return
        if not self.locations_future.done():
            self.root.after(80, self.finish_locations)
            return
        future, self.locations_future = self.locations_future, None
        if self.locations_folder != self.game.get().strip():
            self.load_locations(force=True)
            return
        try:
            self.locations = future.result()
            self.filter_locations()
        except Exception as error:
            logging.exception("Area index failed")
            self.locations = []
            self.filter_locations()
            self.location_count.set(f"World list unavailable: {error}")

    def filter_locations(self):
        if not hasattr(self, "location_tree"):
            return
        self.location_tree.delete(*self.location_tree.get_children())
        query, kind = self.location_search.get().strip().casefold(), self.location_kind.get()
        matches = [(index, row) for index, row in enumerate(self.locations)
                   if (kind == "All areas" or row["kind"] == kind)
                   and query in f"{row['name']} {row['cell']:08x} {row['kind']}".casefold()]
        if not query and kind != "Dungeon":
            matches = [(index, row) for index, row in matches if row.get("position") is not None]
        if not query and kind == "Dungeon":
            groups = {}
            for index, row in matches:
                groups.setdefault(row["cell"] >> 16, (index, row))
            matches = list(groups.values())
        for index, row in matches[:500]:
            self.location_tree.insert("", "end", iid=str(index), values=(row["name"], f"{row['cell']:08x}"))
        self.location_count.set(f"{len(matches):,} areas · showing {min(500, len(matches)):,}. Double-click to open.")

    def open_location(self):
        selected = self.location_tree.selection()
        if selected:
            row = self.locations[int(selected[0])]
            self.navigate_to(row["cell"], row.get("position"))

    def go_to_cell(self):
        try:
            cell = int(self.location_id.get().strip(), 16)
            if not 0 < cell <= 0xffffffff:
                raise ValueError()
            self.navigate_to(cell)
        except ValueError:
            self.status.set("Enter a hexadecimal cell ID, for example 02990109.")

    def navigate_to(self, cell, position=None, from_history=False):
        if self.closing:
            return
        if not from_history:
            self.history_target = None
        folder = self.game.get().strip()
        if not folder:
            self.status.set("Choose the DAT folder before opening an area.")
            return
        self.navigation_wanted = (folder, cell, position)
        self.status.set(f"Opening area {cell:08x}…")
        if self.navigation_future is None:
            self.start_navigation()

    def start_navigation(self):
        from editor_locations import resolve_location
        folder, cell, position = self.navigation_wanted
        self.navigation_loading = self.navigation_wanted
        self.navigation_future = self.executor.submit(self.geometry.resolve_location, folder, cell) if position is None else self.executor.submit(lambda: position)
        self.root.after(60, self.finish_navigation)

    def finish_navigation(self):
        if self.closing or self.navigation_future is None:
            return
        if not self.navigation_future.done():
            self.root.after(60, self.finish_navigation)
            return
        future, self.navigation_future = self.navigation_future, None
        if self.navigation_loading != self.navigation_wanted:
            self.start_navigation()
            return
        try:
            point = future.result()
            folder, cell, unused = self.navigation_loading
            if folder != self.game.get().strip():
                self.status.set("DAT folder changed; open the area again from the refreshed world list.")
                return
            self.view3d.set_tool("explore")
            self.explore_cell, self.explore_position = cell, list(point)
            self.active_scene_cell = None
            self.outdoor_grid = None
            self.scene_ready = None
            self.stream_camera = None
            self.location_id.set(f"{cell:08x}")
            self.arrival_cell = cell
            self.view3d.set_scene([], point, ["Loading the selected region…"])
            self.refresh_scene()
            if self.history_target is not None:
                self.navigation_index = self.history_target
                self.history_target = None
            else:
                self.navigation_history = self.navigation_history[:self.navigation_index+1]
                self.navigation_history.append((cell, list(point)))
                self.navigation_history = self.navigation_history[-30:]
                self.navigation_index = len(self.navigation_history)-1
            self.has_travelled = True
            self.atlas.marker = (cell, list(point)) if 1 <= (cell & 65535) <= 64 else None
            self.atlas.note.set("Click a region to travel there. The gold marker shows the current outdoor position.")
            self.atlas.draw()
            self.workspace.select(1)
            self.library.select(self.page_map)
            if self.geometry.pending_room == cell:
                self.library.select(self.page_geometry)
                self.view3d.set_tool("dungeon")
            self.status.set(f"Exploring {cell:08x}. WASD travel / Q,E height. Choose an entity to build here.")
        except Exception as error:
            logging.exception("Could not open area")
            self.history_target = None
            self.status.set(f"Area could not open: {error}")

    def tool_changed(self):
        tool = self.view3d.tool.get()
        self.show_inspector(tool in ("place", "select", "move", "rotate", "scale"))
        if tool != "select":
            self.selection_camera = None
        prior, self.last_tool = getattr(self, "last_tool", tool), tool
        if tool == prior:
            return
        if tool == "explore":
            if self.active_scene_cell is not None:
                self.explore_cell = self.active_scene_cell
                self.explore_position = list(self.view3d.focus)
        elif prior == "explore" and tool == "place" and self.active_scene_cell is not None:
            # Entering the placement tool explicitly moves its draft to the area
            # being explored; merely travelling never edits placement fields.
            self.pick_3d(list(self.view3d.focus))
            self.explore_cell, self.explore_position = None, None
        if tool != "explore":
            self.stream_camera, self.stream_origin_focus = None, None
        self.refresh_scene()

    def camera_moved(self, focus):
        if self.closing or self.view3d.tool.get() != "explore":
            return
        if self.active_scene_cell is not None and 1 <= (self.active_scene_cell & 65535) <= 64:
            self.atlas.marker = (self.active_scene_cell, list(self.view3d.get_navigation_point()))
            self.minimap.marker = self.atlas.marker
            if self.library.select() == str(self.page_map):
                self.minimap.draw()
        if self.camera_timer is None:
            self.camera_timer = self.root.after(200, self.stream_camera_area)

    def stream_camera_area(self):
        self.camera_timer = None
        if self.closing or self.view3d.tool.get() != "explore" or self.active_scene_cell is None:
            return
        if self.future is not None:
            self.camera_timer = self.root.after(200, self.stream_camera_area)
            return
        from preview_terrain import placement_from_point
        try:
            eye = self.view3d.get_navigation_point()
            if 0x100 <= (self.active_scene_cell & 65535) <= 0xfffb:
                from editor_locations import room_below
                cell = room_below(eye, self.view3d.instances)
                linked = {row["cell"] for row in self.nearby_locations}
                if cell is None or cell == self.active_scene_cell or cell not in linked:
                    return
                focus = tuple(self.view3d.focus)
                self.stream_camera = (cell, focus, self.view3d.azimuth, self.view3d.elevation, self.view3d.distance)
                self.stream_origin_focus = focus
                self.explore_cell, self.explore_position = cell, list(focus)
                self.refresh_scene()
                self.status.set(f"Exploring room {cell:08x}; loading connected rooms…")
                return
            if not self.outdoor_grid:
                return
            cell, point = placement_from_point(self.active_scene_cell, eye, self.outdoor_grid)
            if cell >> 16 == self.active_scene_cell >> 16:
                return
            shift = [eye[i] - point[i] for i in range(3)]
            focus = tuple(self.view3d.focus[i] - shift[i] for i in range(3))
            self.stream_camera = (cell, focus, self.view3d.azimuth, self.view3d.elevation, self.view3d.distance)
            self.stream_origin_focus = tuple(self.view3d.focus)
            self.explore_cell, self.explore_position = cell, list(focus)
            self.refresh_scene()
            self.status.set(f"Exploring landblock {cell >> 16:04x}; loading the next neighborhood…")
        except ValueError as error:
            self.status.set(str(error))

    def request_links(self, cell):
        if self.links_cell == cell or self.links_future is not None or self.closing:
            return
        from editor_locations import location_links
        self.links_cell = cell
        self.links_future = self.executor.submit(self.geometry.location_links, self.game.get().strip(), cell)
        self.root.after(80, self.finish_links)

    def finish_links(self):
        if self.closing or self.links_future is None:
            return
        if not self.links_future.done():
            self.root.after(80, self.finish_links)
            return
        future, self.links_future = self.links_future, None
        if self.links_cell != self.active_scene_cell:
            self.request_links(self.active_scene_cell) if self.active_scene_cell is not None else None
            return
        try:
            self.nearby_locations = future.result()
            self.nearby_tree.delete(*self.nearby_tree.get_children())
            for index, row in enumerate(self.nearby_locations):
                self.nearby_tree.insert("", "end", iid=str(index), values=(row["name"], f"{row['cell']:08x}"))
        except Exception:
            logging.exception("Connected area inventory failed")

    def open_nearby(self):
        selected = self.nearby_tree.selection()
        if selected:
            row = self.nearby_locations[int(selected[0])]
            self.navigate_to(row["cell"], row.get("position"))

    def save_preferences(self):
        if getattr(self.args, "no_settings", False):
            return
        try:
            self.settings_path.parent.mkdir(parents=True, exist_ok=True)
            self.settings_path.write_text(json.dumps({"game_data": self.game.get(), "output": self.output.get(),
                                                       "geometry": self.root.geometry(),
                                                       "recent_entities": self.recent_entities,
                                                       "favorite_entities": sorted(self.favorite_entities), "bookmarks": self.bookmarks}, indent=2), encoding="utf-8")
        except OSError:
            logging.exception("Could not save editor preferences")

    def close(self):
        if self.closing:
            return
        if not self.discard_draft():
            return
        if self.geometry.project.dirty and not messagebox.askyesno("Unsaved world geometry", "Close without saving the terrain/dungeon project?", parent=self.root):
            return
        if self.arrange_dialog is not None:
            self.arrange_dialog.close()
        self.object_tools.cancel()
        self.save_preferences()
        self.closing = True
        if self.scene_timer is not None:
            self.root.after_cancel(self.scene_timer)
            self.scene_timer = None
        if self.location_timer is not None:
            self.root.after_cancel(self.location_timer)
            self.location_timer = None
        if self.camera_timer is not None:
            self.root.after_cancel(self.camera_timer)
            self.camera_timer = None
        self.view3d.close()
        self.status.set("Closing after the current DAT read finishes…")
        def release():
            if self.assets:
                self.assets.close()
                self.assets = None
        self.release_future = self.executor.submit(release)
        self.root.after(40,self.finish_close)

    def finish_close(self):
        # Keep Tk's owner alive until worker-held bound methods are released.
        # Destroying immediately can finalize Tk variables on the DAT thread.
        if not self.release_future.done():
            self.root.after(40,self.finish_close)
            return
        try:
            self.release_future.result()
        except Exception:
            logging.exception("DAT cleanup failed during editor shutdown")
        # Cleanup is already complete; joining now does not wait for file I/O.
        self.executor.shutdown(wait=True)
        self.root.destroy()

    def pick_position(self, event):
        if self.transform:
            x, y, scale, width, height = self.transform
            self.fields["X"].set(format(x + (event.x-width/2)/scale, ".9g"))
            self.fields["Y"].set(format(y - (event.y-height/2)/scale, ".9g"))

    def placement_args(self):
        values = {}
        for name, variable in self.fields.items():
            text = variable.get().strip()
            try:
                values[name] = (int(text, 16) if name in ("Entity", "Cell") else
                                int(text) if name == "Heading" else
                                None if name == "Scale" and not text else float(text))
            except ValueError as error:
                expected = ("a hexadecimal identifier" if name in ("Entity", "Cell") else
                            "a whole number from 0 to 255" if name == "Heading" else "a number")
                raise ValueError(f"{name}: enter {expected}.") from error
        return SimpleNamespace(catalog=self.args.catalog, game_data=Path(self.game.get()),
            output=Path(self.output.get()), entity=values["Entity"], cell=values["Cell"],
            position=[values[name] for name in ("X", "Y", "Z")],
            heading=values["Heading"], scale=values["Scale"])

    def save(self):
        if self.closing:
            return
        if self.view3d.tool.get() in ("terrain", "dungeon", "entrance") or self.library.select()==str(self.page_geometry):
            self.geometry.save()
            return
        if self.group_ids:
            self.open_arrange()
            return
        self.save_button.configure(state="disabled")
        try:
            if not self.game.get().strip() or not self.output.get().strip():
                raise ValueError("Choose a game data folder and a content file")
            args = self.placement_args()
            self.status.set("Checking definition, visual and cell records…")
            self.root.update_idletasks()
            self.require_loaded_path()
            preserve = self.selected_id is not None and self.baseline is not None and self.fields["Heading"].get() == self.baseline["Heading"]
            result, identity = prepare_change(args, self.original, self.selected_id, preserve_orientation=preserve)
            self.commit_change(result)
            if self.keep_adding.get():
                self.new_placement()
                self.status.set(f"Saved object {identity:016x}. Move the draft and Ctrl+S to add the next copy.")
            else:
                self.select_saved(identity)
                self.status.set(f"Saved object {identity:016x}. The running server has not been changed.")
        except (OSError, ValueError, OverflowError) as error:
            messagebox.showerror("Placement was not saved", str(error), parent=self.root)
            self.status.set(f"Not saved: {error}")
        finally:
            self.save_button.configure(state="normal")

    def require_loaded_path(self):
        if self.loaded_path is None or Path(self.output.get()).resolve() != self.loaded_path:
            raise ValueError("Reload the chosen content file before saving")

    def select_saved(self, identity):
        row = next((r for r in self.placements if int(r[0], 16) == identity), None)
        if row and self.objects.exists(row[0]):
            self.objects.selection_set(row[0])
            self.objects.see(row[0])
            self.choose_position(None)

    def commit_change(self, result):
        before = self.original
        commit_document(self.loaded_path, before, result)
        self.history.append((before, result))
        self.history = self.history[-20:]
        self.redo_history.clear()
        self.reload(force=True)

    def delete_selected(self):
        identities = self.group_ids or ((self.selected_id,) if self.selected_id is not None else ())
        if not identities or self.closing:
            return
        label = f"{len(identities)} selected objects" if len(identities) > 1 else f"object {identities[0]:016x}"
        if not messagebox.askyesno("Delete placements", f"Delete {label} from this file? You can undo this change.", parent=self.root):
            return
        try:
            self.require_loaded_path()
            from editor_arrange import remove_selection
            result = remove_selection(self.original, identities)
            self.commit_change(result)
            self.status.set(f"Deleted {label}. Undo restores the complete selection.")
        except (OSError, ValueError) as error:
            messagebox.showerror("Placements were not deleted", str(error), parent=self.root)

    def select_many(self, identities):
        self.placement_search.set("")
        self.cell_only.set(False)
        self.group_ids = ()
        self.selected_id = None
        self.filter_placements()
        values = [row[0] for row in self.placements if int(row[0], 16) in identities]
        if values:
            self.objects.selection_set(values)
            self.objects.see(values[0])
            self.choose_position(None)

    def select_in_view(self, identity, additive=False):
        if self.arrange_dialog is not None:
            return
        selected = set(self.group_ids or ([self.selected_id] if self.selected_id is not None else [])) if additive else set()
        if additive and identity in selected:
            selected.remove(identity)
        else:
            selected.add(identity)
        if not selected:
            self.new_placement()
            return
        if not self.discard_draft():
            return
        camera = (self.view3d.focus, self.view3d.azimuth, self.view3d.elevation, self.view3d.distance)
        self.select_many(selected)
        # Keep the selection tool active for subsequent Ctrl-clicks.
        self.view3d.set_tool("select")
        self.selection_camera = camera
        self.library.select(self.page_scene)

    def open_arrange(self, duplicate=False):
        if self.arrange_dialog is not None:
            self.arrange_dialog.window.lift()
            return
        identities = self.group_ids or ((self.selected_id,) if self.selected_id is not None else ())
        if not identities:
            self.status.set("Select saved objects in the 3D view or Ctrl/Shift-select rows in Saved.")
            return
        if not self.discard_draft():
            return
        if self.baseline is not None and self.field_snapshot() != self.baseline:
            self.set_fields(self.baseline)
        if self.future is not None or self.scene_timer is not None:
            self.status.set("Wait for the selected area's 3D assets to finish loading, then Arrange selection.")
            return
        from editor_arrange_ui import ArrangeDialog
        self.arrange_dialog = ArrangeDialog(self, identities)
        if duplicate:
            self.arrange_dialog.fields["Copies (0 = move)"].set("1")

    def undo(self):
        if self.geometry.stroke_before is not None:
            self.geometry.stroke(None,"cancel")
            return
        self.restore_history(self.history, self.redo_history, False)

    def redo(self):
        self.restore_history(self.redo_history, self.history, True)

    def restore_history(self, source, destination, forward):
        if not source or self.closing or not self.discard_draft():
            return
        try:
            self.require_loaded_path()
            entry = source[-1]
            if len(entry) == 3 and entry[0] == "geometry":
                self.geometry.project.restore(entry[2] if forward else entry[1])
                destination.append(source.pop())
                self.geometry.changed()
                self.status.set("Redid native geometry edit." if forward else "Undid native geometry edit.")
                return
            before, after = entry
            result = after if forward else before
            commit_document(self.loaded_path, self.original, result if result is not None else "pyrealium-world-v2\n")
            destination.append(source.pop())
            self.reload(force=True)
            self.status.set("Redid last file change." if forward else "Undid last file change.")
        except (OSError, ValueError) as error:
            messagebox.showerror("History could not be restored", str(error), parent=self.root)


def main():
    log_folder = Path(os.environ.get("LOCALAPPDATA",tempfile.gettempdir())) / "Pyrealium" / "content-editor" / "logs"
    log_folder.mkdir(parents=True,exist_ok=True)
    log_path = log_folder / (datetime.now(timezone.utc).strftime("editor-%Y%m%d-%H%M%S-") + str(os.getpid()) + ".log")
    # pythonw has no console streams. Keep the log open until all owned worker
    # threads finish, including during interpreter shutdown or a native abort.
    log = log_path.open("a",encoding="utf-8",buffering=1,errors="backslashreplace")
    sys.stdout = sys.stderr = log
    logging.basicConfig(stream=log,level=logging.INFO,format="%(asctime)s %(levelname)s %(message)s")
    faulthandler.enable(file=log,all_threads=True)
    logging.info("Content editor starting: Python %s",sys.version)
    base = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--no-settings", action="store_true", help="do not change remembered editor preferences on exit")
    parser.add_argument("--game-data", type=Path)
    parser.add_argument("--catalog", type=Path, default=base / "content" / "definitions.tsv")
    parser.add_argument("--output", type=Path, default=None)
    args = parser.parse_args()
    root = tk.Tk()
    editor = Editor(root, args)
    def callback_error(kind, value, trace):
        traceback.print_exception(kind,value,trace,file=log)
        log.flush()
        editor.status.set(f"An editor error was recorded in {log_path}")
    root.report_callback_exception = callback_error
    root._content_editor_log = log
    root.mainloop()
    logging.info("Content editor window closed")


if __name__ == "__main__":
    main()
