"""Task-oriented dungeon construction and paired entrance controls."""
import math
import tkinter as tk
from tkinter import ttk, messagebox

class BuildingTools:
    def __init__(self, geometry):
        self.g=geometry;self.editor=geometry.editor
        self.anchors={};self.capture=None
        self.entrance_status=tk.StringVar(value="Mark two ground points outdoors, then two inside: portal and safe arrival.")
        self.name=tk.StringVar(value="My dungeon")
        self.facing=tk.StringVar(value="0")

    def build_panel(self, parent):
        g=self.g;e=self.editor
        ttk.Label(parent,text="Build a dungeon",style="Section.TLabel").pack(anchor="w",pady=6)
        row=ttk.Frame(parent);row.pack(fill="x",pady=3)
        g.project_rooms=ttk.Combobox(row,textvariable=g.project_room,state="readonly",width=12)
        g.project_rooms.pack(side="left",fill="x",expand=True)
        ttk.Button(row,text="Open room",command=g.open_project_room).pack(side="right")
        ttk.Button(parent,text="Choose a room from the library…",command=g.browse_templates).pack(fill="x",pady=3)
        ttk.Button(parent,text="Create a custom room…",style="Accent.TButton",command=self.custom_dialog).pack(fill="x",pady=3)
        ttk.Button(parent,text="Start dungeon with selected template",command=g.seed_room).pack(fill="x",pady=3)
        ttk.Label(parent,text="Extend the current room",style="Section.TLabel").pack(anchor="w",pady=(10,3))
        ttk.Button(parent,text="Show / select doorways in 3D",command=g.inspect_room).pack(fill="x",pady=3)
        g.doors=e.tree(parent,("Door","Connection"),3)
        g.doors.bind("<<TreeviewSelect>>",g.select_door)
        ttk.Button(parent,text="Attach selected template at this doorway",command=g.attach_room).pack(fill="x",pady=3)
        row=ttk.Frame(parent);row.pack(fill="x",pady=3)
        ttk.Button(row,text="Check dungeon",command=lambda:g.dungeon_job("validate")).pack(side="left",fill="x",expand=True)
        ttk.Button(row,text="Entrance & return…",command=self.entrance_dialog).pack(side="right",fill="x",expand=True)
        g.room_status=tk.StringVar(value="Choose a template or draw a custom room. Orange openings need another room; green doorways are connected.")
        ttk.Label(parent,textvariable=g.room_status,wraplength=280,style="Muted.TLabel").pack(anchor="w",pady=6)
        ttk.Button(parent,text="Advanced room IDs / template doorway…",command=self.advanced).pack(fill="x",pady=3)

    def advanced(self):
        g=self.g;window=tk.Toplevel(self.editor.root);window.title("Advanced room settings")
        for label,var in (("Template cell",g.room_source),("New dungeon cell",g.room_target),("Current doorway",g.room_door),("Template doorway",g.template_door)):
            row=ttk.Frame(window,padding=6);row.pack(fill="x")
            ttk.Label(row,text=label,width=22).pack(side="left");ttk.Entry(row,textvariable=var).pack(side="right")
        ttk.Button(window,text="Use current room as template",command=g.use_current_room).pack(fill="x",padx=6,pady=6)

    @staticmethod
    def free_block(builder, preferred):
        used={cell>>16 for cell in builder.templates()}
        for block in range(preferred,0xfffd):
            if block not in used:return block
        raise ValueError("No unused dungeon block remains in this range")

    def custom_dialog(self):
        if self.g.future is not None:return
        g=self.g;window=tk.Toplevel(self.editor.root);window.title("Build a custom room")
        window.geometry("480x620")
        ttk.Label(window,text="Room dimensions and openings",style="Section.TLabel").pack(anchor="w",padx=16,pady=12)
        values={name:tk.StringVar(value=str(value)) for name,value in (("width",40),("depth",40),("height",24),("door_width",20),("door_height",20))}
        presets=ttk.Frame(window);presets.pack(fill="x",padx=16,pady=4)
        def preset(width,depth,height):
            for key,value in (("width",width),("depth",depth),("height",height)):values[key].set(str(value))
        for label,size in (("Room",(40,40,24)),("Corridor",(24,80,24)),("Chamber",(80,80,30))):
            ttk.Button(presets,text=label,command=lambda size=size:preset(*size)).pack(side="left",expand=True,fill="x",padx=2)
        for name,label in (("width","Width"),("depth","Depth"),("height","Height"),("door_width","Door width"),("door_height","Door height")):
            row=ttk.Frame(window,padding=6);row.pack(fill="x",padx=10)
            ttk.Label(row,text=label,width=18).pack(side="left")
            ttk.Spinbox(row,textvariable=values[name],from_=1,to=500,width=12).pack(side="right")
        ttk.Label(window,text="Openings (unselected sides become solid walls)",wraplength=440).pack(anchor="w",padx=16,pady=8)
        doors={side:tk.BooleanVar(value=side in ("north","south")) for side in ("north","east","south","west")}
        row=ttk.Frame(window);row.pack(fill="x",padx=10)
        for side,var in doors.items():ttk.Checkbutton(row,text=side.title(),variable=var).pack(side="left",padx=5)
        current=self.editor.active_scene_cell
        can_attach=current is not None and 0x100<=current&65535<=0xfffb and any(door["open"] for door in self.editor.view3d.dungeon_doors)
        mode=tk.StringVar(value="attach" if can_attach else "new")
        ttk.Radiobutton(window,text="Start a new dungeon",variable=mode,value="new").pack(anchor="w",padx=16,pady=7)
        ttk.Radiobutton(window,text="Attach to selected doorway",variable=mode,value="attach",state="normal" if can_attach else "disabled").pack(anchor="w",padx=16,pady=7)
        source=tk.StringVar(value="south")
        row=ttk.Frame(window,padding=8);row.pack(fill="x")
        ttk.Label(row,text="Opening used for attachment").pack(side="left")
        ttk.Combobox(row,textvariable=source,values=list(doors),state="readonly",width=10).pack(side="right")
        ttk.Label(window,text="Uses the selected template's native material. Custom geometry includes floors, ceiling, walls, collision, and real door openings. Finish open branches before export.",wraplength=440,style="Muted.TLabel").pack(padx=16,pady=10)
        def create():
            try:
                options={key:float(var.get()) for key,var in values.items()}
                if any(not math.isfinite(value) for value in options.values()):raise ValueError("Use finite room dimensions")
                options.update(doors=tuple(side for side,var in doors.items() if var.get()),texture_template_cell=int(g.room_source.get(),16),target_block=int(g.room_target.get(),16)>>16)
                if mode.get()=="attach":options.update(attach_cell=current,attach_portal=int(g.room_door.get()),source_opening=source.get())
            except ValueError as error:messagebox.showerror("Check room dimensions",str(error),parent=window);return
            folder=self.editor.game.get()
            def work():
                from world_room import build_room
                assets=g.reader(folder)
                if "attach_cell" not in options:
                    from world_dungeon import DungeonBuilder
                    options["target_block"]=self.free_block(DungeonBuilder(assets.cells,assets.portal),options["target_block"])
                return build_room(assets.cells,assets.portal,**options)
            if g.start_job("build:custom",work):window.destroy()
        ttk.Button(window,text="Build room",style="Accent.TButton",command=create).pack(fill="x",padx=16,pady=12)

    def entrance_dialog(self):
        window=tk.Toplevel(self.editor.root);window.title("Dungeon entrance and return")
        window.geometry("490x730")
        ttk.Label(window,text="Connect your dungeon to the world",style="Section.TLabel").pack(anchor="w",padx=16,pady=12)
        ttk.Entry(window,textvariable=self.name).pack(fill="x",padx=16,pady=6)
        ttk.Label(window,text="1. Travel outdoors. Mark the entrance portal, then its safe return arrival.\n\n2. Open your dungeon room. Mark the return portal, then its safe entry arrival.\n\nEach button takes two ground clicks in the 3D view. Keep arrivals away from their portals.",wraplength=450).pack(padx=16,pady=10)
        for side,label in (("outside","Mark outdoor entrance + arrival"),("inside","Mark dungeon return + arrival")):
            ttk.Button(window,text=label,command=lambda side=side:self.arm_capture(side,window)).pack(fill="x",padx=16,pady=5)
        row=ttk.Frame(window);row.pack(fill="x",padx=16,pady=5)
        ttk.Label(row,text="Facing for next point (degrees)").pack(side="left")
        ttk.Spinbox(row,textvariable=self.facing,from_=0,to=359,width=8).pack(side="right")
        ttk.Label(window,textvariable=self.entrance_status,wraplength=450).pack(padx=16,pady=10)
        ttk.Button(window,text="Create entrance and return pair",style="Accent.TButton",command=self.create_entrance).pack(fill="x",padx=16,pady=10)
        ttk.Label(window,text="Saved entrance pairs",style="Section.TLabel").pack(anchor="w",padx=16,pady=6)
        tree=self.editor.tree(window,("Name","Outside → dungeon"),3)
        for index,pair in enumerate(self.g.project.entrances):
            tree.insert("","end",iid=str(index),values=(pair["name"],f"{pair['entrance']['source']['cell']:08x} → {pair['entrance']['destination']['cell']:08x}"))
        def remove():
            if not tree.selection():return
            entries=list(self.g.project.entrances);entries.pop(int(tree.selection()[0]))
            before,after=self.g.project.set_entrances(entries);self.record(before,after,"Removed entrance pair")
            window.destroy();self.entrance_dialog()
        ttk.Button(window,text="Remove selected pair (undo available)",command=remove).pack(fill="x",padx=16,pady=6)
        row=ttk.Frame(window);row.pack(fill="x",padx=16,pady=5)
        def preview(key):
            if not tree.selection():return
            route=self.g.project.entrances[int(tree.selection()[0])][key]
            destination=route["destination"]
            self.g.pending_room=destination["cell"] if destination["cell"]&65535>=256 else None
            self.editor.navigate_to(destination["cell"],position=destination["position"])
            window.destroy()
        ttk.Button(row,text="Preview entry travel",command=lambda:preview("entrance")).pack(side="left",fill="x",expand=True)
        ttk.Button(row,text="Preview return travel",command=lambda:preview("return")).pack(side="right",fill="x",expand=True)

    def arm_capture(self, side, window):
        cell=self.editor.active_scene_cell
        if cell is None:return
        outdoor=1<=cell&65535<=64
        if outdoor != (side=="outside"):
            self.editor.status.set("Travel outdoors first." if side=="outside" else "Open a dungeon room first.");return
        self.capture=(side,"portal");self.editor.view3d.set_tool("entrance")
        self.editor.workspace.select(1);window.destroy()
        self.editor.status.set("Click the ground for the portal, then click its safe arrival point.")

    def capture_surface(self, hit):
        if not self.capture:return False
        side,part=self.capture
        cell=hit.get("cell") or self.editor.active_scene_cell
        if (1<=cell&65535<=64)!=(side=="outside"):
            self.editor.status.set("Choose ground in the same outdoor / dungeon area.");return False
        from world_entrance import endpoint
        if side=="outside":
            from preview_terrain import placement_from_point
            cell,point=placement_from_point(self.editor.active_scene_cell,hit["point"],self.editor.outdoor_grid)
        else:point=hit["point"]
        try:
            angle=float(self.facing.get())
            if not math.isfinite(angle) or not 0<=angle<360:raise ValueError()
        except ValueError:
            self.editor.status.set("Arrival facing must be from 0 to 359 degrees.");return False
        if part=="arrival":
            source=self.anchors[side+"_portal"]
            if cell>>16==source["cell"]>>16 and sum((a-b)**2 for a,b in zip(point,source["position"]))<8**2:
                self.editor.status.set("Pick an arrival at least 8 world units from the portal.");return False
        self.anchors[side+"_"+part]=endpoint(cell,point,round(angle*255/360))
        if part=="portal":
            self.capture=(side,"arrival")
            self.editor.status.set("Now click a safe arrival point away from this portal.")
        else:
            self.capture=None;self.editor.view3d.set_tool("explore" if side=="outside" else "dungeon")
            self.entrance_status.set(f"{side.title()} portal and arrival marked. "+("Ready to create the pair." if len(self.anchors)==4 else "Mark the other side next."))
            self.editor.status.set(self.entrance_status.get());self.entrance_dialog()
        return True

    def create_entrance(self):
        try:
            if len(self.anchors)!=4:raise ValueError("Mark both portals and both safe arrivals first")
            from world_entrance import make_pair
            ids=[int(row[0],16) for row in self.editor.placements]
            for pair in self.g.project.entrances:ids.extend((pair["entrance"]["id"],pair["return"]["id"]))
            pair=make_pair(ids,self.anchors["outside_portal"],self.anchors["inside_portal"],self.anchors["inside_arrival"],self.anchors["outside_arrival"],name=self.name.get())
            before,after=self.g.project.set_entrances(self.g.project.entrances+[pair])
            self.record(before,after,"Created entrance and return portals")
            self.anchors.clear();self.entrance_status.set("Entrance pair saved in the project. Export includes both portal objects and travel routes.")
        except (ValueError,KeyError) as error:messagebox.showerror("Entrance pair needs attention",str(error),parent=self.editor.root)

    def record(self,before,after,label):
        self.editor.history.append(("geometry",before,after));self.editor.history=self.editor.history[-20:]
        self.editor.redo_history.clear();self.g.changed();self.editor.status.set(label+" · Ctrl+Z undo")

    def finish_job(self,kind,result):
        if kind=="build:custom":
            archives={key:result[key] for key in ("cell_1.dat","portal.dat")}
            before,after=self.g.project.apply_bundle(archives)
            self.record(before,after,"Built custom native room")
            cell=result["cell"];self.g.pending_room=cell
            self.g.room_target.set(f"{cell+1:08x}")
            self.editor.navigate_to(cell)
