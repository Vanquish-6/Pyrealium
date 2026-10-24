"""WorldBuilder terrain/dungeon tools backed by persistent native DAT records."""
import json
import math
import time
from pathlib import Path
from tkinter import filedialog, messagebox, ttk
import tkinter as tk

from preview_assets import Assets
from world_project import WorldProject, OverlayDat, digest


class GeometryTools:
    def __init__(self, editor):
        self.editor = editor
        self.project = WorldProject(editor.game.get() or Path.cwd())
        self.path = None
        self.future = None
        self.kind = None
        self.terrain_engine = None
        self.stroke_before = None
        self.stroke_after = None
        self.stroke_ended = False
        self.next_point = None
        self.stroke_poll = False
        self.last_sample_time = -math.inf
        self.last_sample_point = None
        self.pick_height = False
        self.palette = []
        self.room_rows = []
        self.pending_room = None
        self.project_room = tk.StringVar()
        self.brush_mode = tk.StringVar(value="Raise")
        self.radius = tk.StringVar(value="20")
        self.strength = tk.StringVar(value="10")
        self.blend = tk.StringVar(value="50")
        self.terrain_status = tk.StringVar(value="Choose a brush, then start sculpting")
        self.flatten_height = tk.StringVar(value="0")
        self.terrain_type = tk.StringVar()
        self.project_status = tk.StringVar(value="Native world project: no geometry edits")
        self.room_source = tk.StringVar(value="0299010d")
        self.room_target = tk.StringVar(value="f1000100")
        self.room_door = tk.StringVar(value="0")
        self.template_door = tk.StringVar(value="0")
        from editor_building import BuildingTools
        self.building=BuildingTools(self)

    def ensure_source(self, folder):
        source = Path(folder).resolve()
        if self.project.source != source:
            if self.project.dirty:
                raise ValueError("Save the geometry project before changing its source DAT folder")
            self.project = WorldProject(source)
            self.path, self.terrain_engine = None, None

    def reader(self, folder):
        """Called only by the editor's single DAT worker."""
        editor = self.editor
        if editor.assets is None or editor.assets.folder != Path(folder):
            if editor.assets:
                editor.assets.close()
            editor.assets = Assets(Path(folder))
            self.terrain_engine = None
        if not isinstance(editor.assets.cells, OverlayDat) or editor.assets.cells.project is not self.project:
            base = editor.assets.cells.base if isinstance(editor.assets.cells, OverlayDat) else editor.assets.cells
            editor.assets.cells = OverlayDat(base, self.project)
            self.terrain_engine = None
        if not isinstance(editor.assets.portal,OverlayDat) or editor.assets.portal.project is not self.project:
            base=editor.assets.portal.base if isinstance(editor.assets.portal,OverlayDat) else editor.assets.portal
            editor.assets.portal=OverlayDat(base,self.project,"portal.dat")
        signature=tuple((did,digest(value)) for did,value in sorted(self.project.portal_patches.items()))
        if getattr(self,"portal_signature",None)!=signature:
            editor.assets.cache.clear();editor.assets.materials.clear()
            self.portal_signature=signature
        return editor.assets

    def build_panel(self, parent):
        editor = self.editor
        top = ttk.Frame(parent)
        top.pack(fill="x", pady=4)
        ttk.Button(top, text="Open project", command=self.open).pack(side="left")
        ttk.Button(top, text="Save", command=self.save).pack(side="left", padx=4)
        ttk.Button(top, text="Export world", command=self.export).pack(side="right")
        ttk.Label(parent, textvariable=self.project_status, wraplength=300).pack(anchor="w", pady=5)
        tabs = ttk.Notebook(parent)
        self.tabs=tabs
        tabs.pack(fill="both", expand=True)
        terrain, rooms = ttk.Frame(tabs, padding=8), ttk.Frame(tabs, padding=8)
        tabs.add(terrain, text="Terrain")
        tabs.add(rooms, text="Dungeon")
        self.rooms_page=rooms
        ttk.Label(terrain, text="Shape the ground", style="Section.TLabel").pack(anchor="w", pady=6)
        ttk.Label(terrain,textvariable=self.terrain_status,wraplength=285).pack(anchor="w",pady=4)
        modes=ttk.Frame(terrain);modes.pack(fill="x",pady=4)
        for index,mode in enumerate(("Raise","Lower","Flatten","Smooth","Paint")):
            ttk.Radiobutton(modes,text=mode,variable=self.brush_mode,value=mode,
                            command=self.terrain_mode_changed).grid(row=index//3,column=index%3,sticky="w",padx=4,pady=4)
        row=ttk.Frame(terrain);row.pack(fill="x",pady=4)
        ttk.Label(row,text="Radius (world units)").pack(side="left")
        self.radius_widget=ttk.Spinbox(row,textvariable=self.radius,from_=.5,to=320,increment=2,width=10)
        self.radius_widget.pack(side="right")
        presets=ttk.Frame(terrain);presets.pack(fill="x",pady=3)
        for label,radius in (("Small",10),("Medium",20),("Large",40)):
            ttk.Button(presets,text=label,command=lambda value=radius:self.set_radius(value)).pack(side="left",expand=True,fill="x",padx=2)
        row=ttk.Frame(terrain);row.pack(fill="x",pady=4)
        self.strength_label=ttk.Label(row,text="Height per pass");self.strength_label.pack(side="left")
        self.strength_widget=ttk.Spinbox(row,textvariable=self.strength,from_=.5,to=50,increment=.5,width=10)
        self.strength_widget.pack(side="right")
        row=ttk.Frame(terrain);row.pack(fill="x",pady=4)
        ttk.Label(row,text="Flatten height").pack(side="left")
        self.flatten_widget=ttk.Spinbox(row,textvariable=self.flatten_height,from_=-10000,to=10000,increment=1,width=10)
        self.flatten_widget.pack(side="right")
        self.pick_height_button=ttk.Button(terrain,text="Pick height from the ground",command=self.arm_height_pick)
        self.pick_height_button.pack(fill="x",pady=4)
        ttk.Label(terrain,text="Ground material",style="Muted.TLabel").pack(anchor="w",pady=(6,0))
        self.palette_widget = ttk.Combobox(terrain,textvariable=self.terrain_type,state="readonly")
        self.palette_widget.pack(fill="x",pady=8)
        ttk.Button(terrain,text="Start sculpting / painting",style="Accent.TButton",command=self.activate_terrain).pack(fill="x",pady=6)
        ttk.Label(terrain,text="Drag on the ground · [ / ] resize · Shift-click samples height · Escape cancels · Ctrl+Z undoes the whole drag.\n\nThe soft brush edge blends into nearby terrain. Native height steps may need a stronger pass to show a change.",wraplength=285,style="Muted.TLabel").pack(anchor="w",pady=8)
        self.terrain_mode_changed()
        editor.root.after_idle(self.attach_terrain_view)
        self.building.build_panel(rooms)

    def changed(self):
        editor=self.editor
        self.project_status.set(f"{len(self.project.patches)+len(self.project.portal_patches)} records · {len(self.project.entrances)} entrance pairs"+(" · unsaved project" if self.project.dirty else " · saved"))
        rooms=[f"{did:08x}" for did in sorted(self.project.patches) if 0x100<=did&65535<=0xfffb]
        self.project_rooms.configure(values=rooms)
        if rooms and self.project_room.get() not in rooms:self.project_room.set(rooms[0])
        editor.links_cell=None
        editor.refresh_scene()
        editor.update_actions()

    def commit_changes(self, changes, label):
        if not changes:
            self.editor.status.set("The brush did not change a native sample at this strength.")
            return
        before,after=self.project.apply(changes)
        self.editor.history.append(("geometry",before,after))
        self.editor.history=self.editor.history[-20:]
        self.editor.redo_history.clear()
        self.changed()
        self.editor.status.set(f"{label} · {len(changes)} native records · Ctrl+Z undo")

    def start_job(self, kind, function, *args):
        if self.future is not None:
            self.editor.status.set("Finish the current native-data operation first.")
            return False
        self.kind=kind
        self.future=self.editor.executor.submit(function,*args)
        self.editor.root.after(50,self.finish_job)
        return True

    def finish_job(self):
        if self.editor.closing or self.future is None:
            return
        if not self.future.done():
            self.editor.root.after(50,self.finish_job);return
        future,self.future=self.future,None
        try:
            result=future.result()
            if self.kind=="palette":
                self.palette=result
                values=[f"{row['id']}: {row['label']}" for row in result]
                self.palette_widget.configure(values=values)
                if values:self.terrain_type.set(values[0])
            elif self.kind=="stroke":
                if self.stroke_ended=="cancel":
                    self.finish_stroke()
                elif self.stroke_before is None:
                    self.next_point=None
                elif self.next_point is not None:
                    self.apply_stroke_result(result)
                    point,self.next_point=self.next_point,None
                    self.queue_stroke(point)
                else:
                    self.apply_stroke_result(result)
                    if self.stroke_ended:self.finish_stroke()
            elif self.kind=="export":
                self.editor.status.set(f"Exported and verified a native DAT copy: {result['destination']}")
            else:
                self.finish_dungeon_job(self.kind,result)
        except Exception as error:
            if self.stroke_before is not None:
                self.project.restore(self.stroke_before)
                self.stroke_before,self.stroke_after=None,None
                self.changed()
            self.next_point=None
            self.stroke_ended=False
            self.editor.status.set(f"Native world edit failed: {error}")
            messagebox.showerror("World edit could not finish",str(error),parent=self.editor.root)

    def terrain(self, folder):
        from world_terrain import TerrainEditor
        assets=self.reader(folder)
        if self.terrain_engine is None:
            self.terrain_engine=TerrainEditor(assets.cells,assets.portal)
        return self.terrain_engine

    def activate_terrain(self):
        editor=self.editor
        if editor.active_scene_cell is None or not 1 <= (editor.active_scene_cell&65535)<=64:
            editor.status.set("Travel to outdoor terrain before activating a brush.");return
        self.ensure_source(editor.game.get())
        editor.view3d.set_tool("terrain")
        editor.workspace.select(1)
        editor.show_inspector(False)
        if hasattr(self,"tabs"):self.tabs.select(0)
        self.terrain_feedback()
        if hasattr(self,"radius_widget"):
            self.radius_widget.configure(to=self.radius_limit())
        if not self.palette:
            folder=editor.game.get()
            self.start_job("palette",lambda:self.terrain(folder).palette())

    def radius_limit(self):
        return max(.5,self.editor.outdoor_grid.get("block_side",160)*2) if self.editor.outdoor_grid else 320

    def attach_terrain_view(self):
        """The viewport is constructed after the Build panel."""
        view=self.editor.view3d
        view.tool.trace_add("write",lambda *args:self.terrain_feedback())
        view.canvas.bind("<KeyPress-bracketleft>",lambda event:self.resize_brush(.8),add="+")
        view.canvas.bind("<KeyPress-bracketright>",lambda event:self.resize_brush(1.25),add="+")
        self.terrain_feedback()

    def set_radius(self, value):
        self.radius.set(format(min(self.radius_limit(),max(.5,float(value))),".6g"))
        preview=self.editor.view3d.brush_preview
        if preview:
            preview["radius"]=float(self.radius.get())
            self.editor.view3d.redraw()
        self.terrain_feedback()

    def resize_brush(self, factor):
        if self.editor.view3d.tool.get()!="terrain":return
        try:self.set_radius(float(self.radius.get())*factor)
        except (ValueError,TypeError):self.set_radius(20)
        return "break"

    def terrain_feedback(self):
        if not hasattr(self,"terrain_status"):return
        view=getattr(self.editor,"view3d",None)
        if view is None or view.tool.get()!="terrain":
            self.terrain_status.set("Brush paused · Start sculpting to edit the ground")
        elif self.pick_height:
            self.terrain_status.set("Pick height · Click the ground; this click does not edit it")
        else:
            self.terrain_status.set(f"{self.brush_mode.get()} active · Radius {self.radius.get()} · Drag on the ground")

    def terrain_mode_changed(self):
        mode=self.brush_mode.get()
        self.pick_height=False
        blend=mode in ("Flatten","Smooth")
        self.strength_label.configure(text="Blend per pass (%)" if blend else "Height per pass")
        self.strength_widget.configure(textvariable=self.blend if blend else self.strength,
                                       from_=1 if blend else .5,to=100 if blend else 50,
                                       increment=5 if blend else .5,state="disabled" if mode=="Paint" else "normal")
        self.flatten_widget.configure(state="normal" if mode=="Flatten" else "disabled")
        self.pick_height_button.configure(state="normal" if mode=="Flatten" else "disabled")
        self.palette_widget.configure(state="readonly" if mode=="Paint" else "disabled")
        self.terrain_feedback()

    def arm_height_pick(self):
        if self.stroke_before is not None or self.future is not None:
            self.editor.status.set("Finish the current brush pass before picking a height.");return
        self.activate_terrain()
        if self.editor.view3d.tool.get()=="terrain":
            self.pick_height=True
            self.terrain_feedback()

    def stroke(self, point, phase):
        if phase in ("resize-smaller","resize-larger"):
            self.resize_brush(.8 if phase=="resize-smaller" else 1.25);return True
        if phase=="entrance":
            return self.building.capture_surface(point)
        if phase=="sample" or phase=="begin" and getattr(self,"pick_height",False):
            if self.stroke_before is not None or self.future is not None:return False
            if point is None or len(point)!=3 or any(not math.isfinite(v) for v in point):return False
            self.flatten_height.set(format(point[2],".6g"))
            self.brush_mode.set("Flatten")
            if hasattr(self,"strength_widget"):self.terrain_mode_changed()
            self.pick_height=False
            self.terrain_feedback()
            self.editor.status.set(f"Flatten height picked: {point[2]:.3f} · Drag to level the ground")
            return False
        if phase == "hover":
            try:
                radius=float(self.radius.get())
                if not math.isfinite(radius) or radius <= 0 or len(point)!=3 or any(not math.isfinite(v) for v in point):
                    raise ValueError("Invalid brush preview")
                self.editor.view3d.brush_preview = dict(center=list(point), radius=radius)
                self.editor.view3d.redraw()
            except (ValueError, TypeError):
                self.editor.view3d.brush_preview=None
            return True
        if phase=="begin":
            if self.future is not None or self.stroke_before is not None:return False
            self.stroke_before,self.stroke_after={},{}
            self.stroke_ended=False
            self.next_point=None
            self.last_sample_time=-math.inf
            self.last_sample_point=None
        if self.stroke_before is None:return False
        if phase in ("begin","preview"):
            if self.stroke_ended:return False
            if phase=="preview" and self.last_sample_point is not None and self.brush_mode.get()==getattr(self,"last_sample_mode",None):
                try:
                    spacing=max(.5,float(self.radius.get())*.15)
                    if math.hypot(point[0]-self.last_sample_point[0],point[1]-self.last_sample_point[1])<spacing:
                        self.next_point=None
                        return True
                except (ValueError,TypeError,IndexError):pass
            if self.future is not None:self.next_point=point
            else:self.queue_stroke(point)
        elif phase=="commit":
            if self.stroke_ended=="cancel":return False
            self.stroke_ended=True
            if self.future is None and self.next_point is None:self.finish_stroke()
        elif phase=="cancel":
            self.next_point=None
            self.stroke_ended=False
            if self.future is not None:
                self.stroke_ended="cancel"
            else:
                self.project.restore(self.stroke_before)
                self.stroke_before,self.stroke_after=None,None
                self.changed()
        return self.stroke_before is not None or phase in ("commit","cancel")

    def queue_stroke(self, point):
        try:
            remaining=.08-(time.monotonic()-self.last_sample_time)
            if remaining>0:
                self.next_point=point
                if not self.stroke_poll:
                    self.stroke_poll=True
                    self.editor.root.after(max(1,math.ceil(remaining*1000)),self.flush_stroke)
                return
            self._queue_stroke(point)
        except (ValueError, TypeError) as error:
            self.project.restore(self.stroke_before or {})
            self.stroke_before,self.stroke_after=None,None
            self.next_point=None
            self.stroke_ended=False
            self.changed()
            self.editor.status.set(f"Brush settings need attention: {error}")

    def flush_stroke(self):
        self.stroke_poll=False
        if self.stroke_before is None or self.stroke_ended=="cancel" or self.future is not None:return
        point,self.next_point=self.next_point,None
        if point is not None:self.queue_stroke(point)
        elif self.stroke_ended:self.finish_stroke()

    def _queue_stroke(self, point):
        editor=self.editor
        reference=editor.active_scene_cell
        from preview_terrain import placement_from_point
        cell,canonical=placement_from_point(reference,point,editor.outdoor_grid)
        options=dict(radius=float(self.radius.get()),strength=float(self.strength.get()),mode=self.brush_mode.get().lower(),
                     target_height=float(self.flatten_height.get()) if self.brush_mode.get()=="Flatten" else None,
                     terrain_type=int(self.terrain_type.get().split(":",1)[0]) if self.brush_mode.get()=="Paint" else None)
        if options["mode"] in ("flatten","smooth") and hasattr(self,"blend"):
            blend=float(self.blend.get())
            if not math.isfinite(blend) or not 0<blend<=100:raise ValueError("Blend must be between 1 and 100 percent")
            options["strength"]=blend/100
        elif options["mode"]=="paint":options["strength"]=1
        if options["mode"] not in ("raise","lower","flatten","smooth","paint"):
            raise ValueError("Choose a terrain brush")
        if not math.isfinite(options["radius"]) or not 0 < options["radius"] <= editor.outdoor_grid["block_side"]*2:
            raise ValueError("Radius must be positive and at most two landblocks")
        if not math.isfinite(options["strength"]) or not 0 < options["strength"] <= 1000:
            raise ValueError("Strength must be positive and at most 1000")
        if options["target_height"] is not None and (not math.isfinite(options["target_height"]) or abs(options["target_height"]) >= 1_000_000):
            raise ValueError("Flatten height must be finite and within world bounds")
        folder=editor.game.get()
        if self.start_job("stroke",lambda:self.terrain(folder).stroke(cell,canonical,**options)):
            self.last_sample_time=time.monotonic()
            self.last_sample_point=list(point)
            self.last_sample_mode=self.brush_mode.get()

    def apply_stroke_result(self, changes):
        if self.stroke_before is None or not changes:return
        for identity,(before,after) in changes.items():
            self.stroke_before.setdefault(identity,before)
            self.stroke_after[identity]=after
        self.project.apply(changes)
        self.changed()

    def finish_stroke(self):
        if self.stroke_before is None:return
        if self.stroke_ended=="cancel":
            self.project.restore(self.stroke_before)
        else:
            after={identity:value for identity,value in self.stroke_after.items() if value!=self.stroke_before[identity]}
            before={identity:self.stroke_before[identity] for identity in after}
            if after:
                self.editor.history.append(("geometry",before,after))
                self.editor.history=self.editor.history[-20:]
                self.editor.redo_history.clear()
                self.editor.status.set(f"Terrain stroke applied · {len(after)} native blocks · Ctrl+Z undo")
            else:
                self.editor.status.set("The brush did not change a native sample at this strength.")
        self.stroke_before,self.stroke_after=None,None
        self.next_point=None
        self.stroke_ended=False
        self.changed()

    def save(self):
        if self.future is not None:
            self.editor.status.set("Wait for the native operation before saving the project.");return
        path=self.path or filedialog.asksaveasfilename(title="Save world geometry project",defaultextension=".world.json",initialfile=Path(self.editor.output.get()).stem+".world.json",filetypes=[("World project","*.json")])
        if path:
            try:
                self.project.save(path);self.path=Path(path);self.changed()
            except (OSError,ValueError) as error:messagebox.showerror("Project was not saved",str(error),parent=self.editor.root)

    def open(self):
        if self.future is not None or self.stroke_before is not None:
            self.editor.status.set("Finish the current world edit before opening another project.");return
        if self.project.dirty and not messagebox.askyesno("Unsaved world project","Discard unsaved geometry changes and open another project?",parent=self.editor.root):return
        path=filedialog.askopenfilename(title="Open world geometry project",filetypes=[("World project","*.json")])
        if path:
            try:
                self.project=WorldProject.load(path,source=self.editor.game.get());self.path=Path(path)
                self.terrain_engine=None;self.editor.history.clear();self.editor.redo_history.clear();self.changed()
            except (OSError,ValueError,KeyError) as error:messagebox.showerror("Project could not open",str(error),parent=self.editor.root)

    def export(self):
        if not (self.project.patches or self.project.portal_patches or self.project.entrances):
            self.editor.status.set("Make world edits before exporting.");return
        if self.future is not None:return
        from copy import deepcopy
        from datetime import datetime
        parent=filedialog.askdirectory(title="Choose where to create a new world export folder")
        if not parent:return
        destination=Path(parent)/("world-export-"+datetime.now().strftime("%Y%m%d-%H%M%S"))
        project=deepcopy(self.project)
        document=self.editor.original or "pyrealium-world-v2\n"
        def work():
            from world_bundle import export_bundle
            return export_bundle(project,destination,document)
        self.editor.status.set("Validating rooms and entrances, then exporting independent DAT copies…")
        self.start_job("export",work)

    def use_current_room(self):
        cell=self.editor.active_scene_cell
        if cell is not None and 0x100 <= (cell&65535)<=0xfffb:self.room_source.set(f"{cell:08x}")

    def inspect_room(self):
        self.dungeon_job("inspect")

    def seed_room(self):
        self.dungeon_job("seed")

    def attach_room(self):
        self.dungeon_job("attach")

    def resolve_location(self, folder, cell):
        from editor_locations import resolve_location
        assets=self.reader(folder)
        return resolve_location(folder,cell,cells=assets.cells,portal=assets.portal,mesh_loader=assets.mesh)

    def location_links(self, folder, cell):
        from editor_locations import location_links
        assets=self.reader(folder)
        return location_links(folder,cell,cells=assets.cells)

    def open_project_room(self):
        if self.project_room.get():
            cell=int(self.project_room.get(),16)
            self.pending_room=cell
            self.editor.navigate_to(cell)

    def browse_templates(self):
        if self.future is not None:return
        window=tk.Toplevel(self.editor.root)
        window.title("Native dungeon room library")
        window.geometry("1020x600")
        ttk.Label(window,text="Select a room to preview its actual native geometry. End caps finish branches; passages and junctions extend them.",wraplength=940).pack(padx=12,pady=10)
        body=ttk.Panedwindow(window,orient="horizontal");body.pack(fill="both",expand=True)
        left,right=ttk.Frame(body),ttk.Frame(body);body.add(left,weight=1);body.add(right,weight=1)
        self.templates_tree=self.editor.tree(left,("Template","Shape / doors"),12)
        self.templates_tree.column("Shape / doors",width=270)
        self.templates_tree.bind("<<TreeviewSelect>>",self.select_template)
        from preview_view import Preview3D
        self.template_view=Preview3D(right)
        self.template_view.pack(fill="both",expand=True)
        def close():
            self.template_view.close();window.destroy()
        window.protocol("WM_DELETE_WINDOW",close)
        ttk.Button(window,text="Use selected template",command=close).pack(pady=10)
        self.dungeon_job("catalog")

    def pick_door(self, index):
        self.room_door.set(str(index))
        if self.doors.exists(str(index)):
            self.doors.selection_set(str(index))
            self.doors.see(str(index))
        self.highlight_doors()

    def highlight_doors(self):
        for door in self.editor.view3d.dungeon_doors:
            door["selected"]=str(door["index"])==self.room_door.get()
        self.editor.view3d.redraw()

    def select_door(self, event=None):
        selected=self.doors.selection()
        if selected:self.room_door.set(selected[0])
        self.highlight_doors()

    def select_template(self, event=None):
        selected=self.templates_tree.selection()
        if selected:
            self.room_source.set(selected[0])
            self.template_door.set("0")
            self.dungeon_job("template_preview")

    def dungeon_job(self, kind):
        if self.future is not None:return
        try:
            folder=self.editor.game.get()
            self.ensure_source(folder)
            template=int(self.room_source.get().strip(),16)
            target=int(self.room_target.get().strip(),16)
            current=self.editor.active_scene_cell
            door=int(self.room_door.get())
            template_door=int(self.template_door.get())
            if kind in ("attach","inspect","validate") and (current is None or (current&65535)<0x100):
                raise ValueError("Open a dungeon room before using this tool")
        except ValueError as error:
            self.editor.status.set(str(error));return
        def work():
            from world_dungeon import DungeonBuilder
            assets=self.reader(folder)
            builder=DungeonBuilder(assets.cells,assets.portal)
            if kind=="catalog":return builder.template_catalog(template>>16,limit=512)
            if kind=="template_preview":
                room,geometry=builder.room_template(template)
                mesh=assets.mesh(room["mesh"])
                return dict(mesh=mesh,position=[0,0,0],quaternion=[1,0,0,0],scale=[1,1,1],role="world",cell=template),[door["index"] for door in room["portals"]]
            if kind=="inspect":return builder.doorways(current)
            if kind=="validate":return builder.validate(current>>16)
            if kind=="seed":
                free=self.building.free_block(builder,target>>16)
                changes=builder.seed(template,free)
            else:changes=builder.attach(current,door,template,template_door)
            from world_dungeon import parse_cell
            cell=builder.last_cell
            room=parse_cell(changes[cell][1])
            return changes,cell,room["position"]
        self.editor.status.set("Loading native room construction data…")
        self.start_job(kind,work)

    def finish_dungeon_job(self, kind, result):
        if kind.startswith("build:"):
            self.building.finish_job(kind,result);return
        if kind=="catalog":
            if not self.templates_tree.winfo_exists():return
            self.templates_tree.delete(*self.templates_tree.get_children())
            for row in result:
                self.templates_tree.insert("","end",iid=f"{row['cell']:08x}",values=(f"{row['cell']:08x}",row["label"]))
            self.room_status.set(f"{len(result)} native templates. One-door rooms can finish an open branch. Select a template, then choose its doorway number.")
        elif kind=="template_preview":
            if not self.template_view.winfo_exists():return
            instance,doors=result
            self.template_door.set(str(doors[0]) if doors else "0")
            self.template_view.set_scene([instance],[0,0,0])
            self.template_view.set_tool("dungeon")
            self.template_view.fit()
        elif kind=="inspect":
            self.tabs.select(self.rooms_page)
            self.editor.view3d.dungeon_doors=result
            self.editor.view3d.set_tool("dungeon")
            self.highlight_doors()
            self.doors.delete(*self.doors.get_children())
            for door in result:
                self.doors.insert("","end",iid=str(door["index"]),values=(door["index"],"OPEN" if door["open"] else f"→ {door['other_cell']:08x}"))
            self.room_status.set("Select an open doorway, choose a template, then Fit and connect room.")
        elif kind=="validate":
            self.room_status.set("Connections valid: all doorway polygons and reciprocal links match." if not result else "\n".join(result[:12]))
        else:
            changes,cell,position=result
            self.commit_changes(changes,"Created native room" if kind=="seed" else "Fitted and connected native room")
            self.room_target.set(f"{cell+1:08x}")
            self.pending_room=cell
            self.editor.navigate_to(cell,position=position)
            self.room_status.set(f"Room {cell:08x} created. Inspect its doorways to extend the dungeon. Check connections before exporting.")
