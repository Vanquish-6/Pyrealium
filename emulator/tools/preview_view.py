"""Native DAT geometry, UV textures and animation playback in a Tk viewport.

Windows OpenGL renders actual texture pixels; a painter fallback remains for
unavailable contexts. Native clip skinning is separate from game behavior logic.
"""
import math
import logging
import time
import tkinter as tk
from tkinter import ttk


def add(a, b):
    return tuple(x + y for x, y in zip(a, b))


def sub(a, b):
    return tuple(x - y for x, y in zip(a, b))


def mul(a, amount):
    return tuple(x * amount for x in a)


def dot(a, b):
    return sum(x * y for x, y in zip(a, b))


def cross(a, b):
    return (a[1]*b[2]-a[2]*b[1], a[2]*b[0]-a[0]*b[2], a[0]*b[1]-a[1]*b[0])


def unit(vector):
    length = math.sqrt(dot(vector, vector))
    return mul(vector, 1 / length) if length > 1e-12 else (0.0, 0.0, 1.0)


def rotate(point, quaternion):
    w, x, y, z = quaternion
    v = (x, y, z)
    twice_cross = mul(cross(v, point), 2)
    return add(point, add(mul(twice_cross, w), cross(v, twice_cross)))


def qmultiply(a, b):
    aw, ax, ay, az = a
    bw, bx, by, bz = b
    return (aw*bw-ax*bx-ay*by-az*bz, aw*bx+ax*bw+ay*bz-az*by,
            aw*by-ax*bz+ay*bw+az*bx, aw*bz+ax*by-ay*bx+az*bw)


class Preview3D(ttk.Frame):
    TRIANGLE_LIMIT = 100000
    COLORS = {"world": (113, 132, 145), "saved": (76, 159, 213), "preview": (229, 172, 75)}

    def __init__(self, parent, on_pick=None, on_nudge=None, on_camera_move=None, on_select=None, on_command=None, on_transform=None, on_stamp=None, on_surface=None, on_door=None):
        super().__init__(parent)
        self.on_pick = on_pick
        self.on_nudge = on_nudge
        self.on_camera_move = on_camera_move
        self.on_select = on_select
        self.on_command = on_command
        self.on_transform = on_transform
        self.on_stamp = on_stamp
        self.on_surface = on_surface
        self.terrain_dragging = False
        self.brush_preview = None
        self.dungeon_doors = []
        self.portal_markers = []
        self.on_door = on_door
        self.placement_enabled = False
        self.transform_drag = None
        self.hover_last = 0
        self.held_keys = {}
        self.travel_pending = None
        self.travel_speed = tk.StringVar(value="20")
        self.arrangement_base = None
        self.instances = []
        self.scene_warnings = []
        self.geometry_warnings = []
        self.center = (0.0, 0.0, 0.0)
        self.placement = self.center
        self.initial_heading = 0
        self.focus = (0.0, 0.0, 1.5)
        self.azimuth = math.radians(-55)
        self.elevation = math.radians(28)
        self.distance = 24.0
        self.pending = None
        self.drag = None
        self.placement_drag = False
        self.closed = False
        self.gpu = None
        self.gpu_error = None
        self.animation_pending = None
        self.paint_poll_pending = None
        self.animation_time = 0.0
        self.animation_last = time.perf_counter()
        self.clip_ids = {}
        self.picked_cell = None
        tools = ttk.Frame(self)
        tools.pack(fill="x", pady=(0, 5))
        self.tool = tk.StringVar(value="place")
        self.tool_buttons = {}
        for name, label in (("explore", "Fly"), ("select", "Select  Q"), ("place", "Place  P"),
                            ("move", "Move  W"), ("rotate", "Rotate  E"), ("scale", "Scale  R")):
            button = ttk.Button(tools, text=label, command=lambda value=name: self.set_tool(value))
            button.pack(side="left", padx=2)
            self.tool_buttons[name] = button
        self.move_step = tk.StringVar(value="1")
        controls = ttk.Frame(self)
        controls.pack(fill="x", pady=(0, 4))
        ttk.Button(controls, text="Frame object (F)", command=self.focus_placement).pack(side="left")
        ttk.Button(controls, text="Fit scene", command=self.fit).pack(side="left", padx=5)
        ttk.Button(controls, text="Reset view", command=self.reset).pack(side="left")
        ttk.Button(controls, text="Details", command=self.details).pack(side="left", padx=5)
        self.cutaway = tk.BooleanVar(value=True)
        ttk.Checkbutton(controls, text="Cut away ceiling", variable=self.cutaway,
                        command=self.redraw).pack(side="right")
        animation_bar = ttk.Frame(self)
        self.animation_bar = animation_bar
        ttk.Button(controls, text="Animation", command=self.toggle_animation_controls).pack(side="left", padx=3)
        self.playing = tk.BooleanVar(value=True)
        self.play_button = ttk.Checkbutton(animation_bar, text="Play animation", variable=self.playing,
                                          command=self.animation_changed)
        self.play_button.pack(side="left")
        self.clip_name = tk.StringVar()
        self.clip_selector = ttk.Combobox(animation_bar, textvariable=self.clip_name, width=28, state="disabled")
        self.clip_selector.pack(side="left", padx=6, fill="x", expand=True)
        self.clip_selector.bind("<<ComboboxSelected>>", self.animation_changed)
        ttk.Label(animation_bar, text="Speed").pack(side="left")
        self.animation_speed = tk.StringVar(value="1.0")
        ttk.Spinbox(animation_bar, from_=0.1, to=3, increment=0.1, width=4,
                    textvariable=self.animation_speed).pack(side="left", padx=4)
        self.canvas = tk.Canvas(self, background="#17212c", highlightthickness=0, height=330)
        self.canvas.pack(fill="both", expand=True)
        self.status = tk.StringVar(value="Choose a game data folder and load the native geometry.")
        self.tool_help = tk.StringVar()
        ttk.Label(self, textvariable=self.tool_help, wraplength=850).pack(anchor="w", pady=(5, 0))
        self.tool_changed()
        ttk.Label(self, textvariable=self.status, wraplength=650).pack(anchor="w", pady=(3, 0))
        self.canvas.bind("<Configure>", lambda event: self.redraw())
        self.canvas.bind("<Motion>", self.hover_placement)
        self.canvas.bind("<ButtonPress-1>", self.orbit_start)
        self.canvas.bind("<B1-Motion>", self.orbit_drag)
        self.canvas.bind("<ButtonRelease-1>", lambda event: self.end_drag())
        self.canvas.bind("<ButtonPress-3>", self.pan_start)
        self.canvas.bind("<B3-Motion>", self.pan_drag)
        self.canvas.bind("<ButtonRelease-3>", lambda event: self.end_drag())
        self.canvas.bind("<MouseWheel>", self.zoom)
        self.canvas.bind("<Button-4>", lambda event: self.zoom_steps(1))
        self.canvas.bind("<Button-5>", lambda event: self.zoom_steps(-1))
        self.canvas.bind("<KeyPress-f>", lambda event: self.focus_placement())
        self.canvas.bind("<KeyPress-F>", lambda event: self.focus_placement())
        self.canvas.bind("<KeyPress-Home>", lambda event: self.fit())
        self.canvas.bind("<KeyPress-p>", lambda event: self.set_tool("place"))
        self.canvas.bind("<KeyPress-o>", lambda event: self.set_tool("orbit"))
        for key, movement in (("a", (-1, 0, 0)), ("d", (1, 0, 0)),
                              ("w", (0, 1, 0)), ("s", (0, -1, 0)),
                              ("q", (0, 0, -1)), ("e", (0, 0, 1))):
            self.canvas.bind(f"<KeyPress-{key}>", lambda event: self.key_input(event.keysym, event.state, True))
            self.canvas.bind(f"<KeyRelease-{key}>", lambda event: self.key_input(event.keysym, event.state, False))
            self.canvas.bind(f"<KeyPress-{key.upper()}>", lambda event: self.key_input(event.keysym, event.state, True))
            self.canvas.bind(f"<KeyRelease-{key.upper()}>", lambda event: self.key_input(event.keysym, event.state, False))
        for key, movement in (("Left", (-1, 0, 0)), ("Right", (1, 0, 0)),
                              ("Up", (0, 1, 0)), ("Down", (0, -1, 0)),
                              ("Prior", (0, 0, 1)), ("Next", (0, 0, -1))):
            self.canvas.bind(f"<KeyPress-{key}>", lambda event, movement=movement: self.nudge(event, movement))
        self.canvas.bind("<Enter>", lambda event: self.canvas.focus_set())
        self.canvas.bind("<Destroy>", lambda event: self.close())
        self.paint_poll_pending = self.after(33, self.poll_native_paint)

    def poll_native_paint(self):
        """Bridge native paint requests only from a normal Tk callback."""
        self.paint_poll_pending = None
        if self.closed:
            return
        if self.gpu and self.gpu.consume_repaint():
            self.redraw()
        if self.gpu:
            for sequence, x, y, state, delta, key in self.gpu.consume_input():
                if sequence == "<FocusOut>":
                    self.stop_travel()
                    continue
                if key is not None:
                    self.key_input(key, state, sequence == "<KeyPress>")
                    continue
                options = {"state": state}
                if key is not None:
                    options["keysym"] = key
                else:
                    options.update(x=x, y=y)
                if sequence == "<MouseWheel>":
                    options["delta"] = delta
                self.canvas.event_generate(sequence, when="tail", **options)
        self.paint_poll_pending = self.after(33, self.poll_native_paint)

    def set_scene(self, instances, center, warnings=(), heading=0, preserve_camera=False):
        """Replace the scene. center is the initial selected placement anchor."""
        self.arrangement_base = None
        instances = list(instances)
        self.center = tuple(float(value) for value in center)
        self.placement = self.center
        self.initial_heading = heading
        self.scene_warnings = list(warnings)
        if not preserve_camera:
            self.cutaway.set(not any(source.get("terrain") for source in instances))
        self.geometry_warnings = []
        self.instances = []
        total_triangles = sum(len(instance["mesh"].get("indices", ())) // 3 for instance in instances)
        remaining = self.TRIANGLE_LIMIT
        # The selected creature remains present if a large room exceeds the bound.
        ordered = sorted(instances, key=lambda instance: {"preview": 0, "saved": 1}.get(instance.get("role"), 2))
        for source in ordered:
            mesh = source["mesh"]
            indices = mesh.get("indices", ())
            count = min(len(indices) // 3, remaining)
            if count == 0:
                continue
            local = mesh.get("vertices", ())
            selected_indices = tuple(indices[:count * 3])
            if any(not isinstance(index, int) or index < 0 or index >= len(local) for index in selected_indices):
                self.geometry_warnings.append("A mesh with invalid triangle indices was omitted")
                continue
            if any(len(vertex) != 3 or any(not math.isfinite(value) for value in vertex) for vertex in local):
                self.geometry_warnings.append("A mesh with invalid vertices was omitted")
                continue
            item = {
                "local": local, "indices": selected_indices,
                "position": tuple(source.get("position", (0, 0, 0))),
                "quaternion": tuple(source.get("quaternion", (1, 0, 0, 0))),
                "scale": tuple(source.get("scale", (1, 1, 1))),
                "placement_scale": source.get("placement_scale", 1) or 1,
                "role": source.get("role", "world"),
                "triangle_colors": mesh.get("triangle_colors", ()),
                "did": mesh.get("did"),
                "local_normals": mesh.get("normals", ()),
                "uvs": mesh.get("uvs", ()),
                "groups": mesh.get("groups", ()),
                "material_images": mesh.get("material_images", {}),
                "material_colors": mesh.get("material_colors", {}),
                "material_diffuse": mesh.get("material_diffuse", {}),
                "material_states": mesh.get("material_states", {}),
                # Skin/room formats may contain zero-filled color fields that
                # their shaders ignore. Terrain near colors are explicitly used.
                "colors": mesh.get("colors", ()) if source.get("terrain") else (),
                "cell": source.get("cell"),
                "object_id": source.get("object_id"),
                "selected": False,
                "terrain": bool(source.get("terrain")),
                "mesh_source": mesh,
                "animation_set": source.get("animation_set"),
            }
            item["base_position"] = item["position"]
            item["base_quaternion"] = item["quaternion"]
            item["base_scale"] = item["scale"]
            self.transform(item)
            self.instances.append(item)
            remaining -= count
        if total_triangles > self.TRIANGLE_LIMIT:
            self.geometry_warnings.append(
                f"Geometry limit: showing at most {self.TRIANGLE_LIMIT:,} of {total_triangles:,} triangles")
        self.configure_animation()
        if not preserve_camera:
            self.focus_placement()
        else:
            self.redraw()

    @staticmethod
    def transform(item):
        scale, quaternion, position = item["scale"], item["quaternion"], item["position"]
        w,x,y,z = quaternion
        a,b,c = 1-2*(y*y+z*z), 2*(x*y-w*z), 2*(x*z+w*y)
        d,e,f = 2*(x*y+w*z), 1-2*(x*x+z*z), 2*(y*z-w*x)
        g,h,i = 2*(x*z-w*y), 2*(y*z+w*x), 1-2*(x*x+y*y)
        sx,sy,sz = scale
        px,py,pz = position
        ax,by,cz,dx,ey,fz,gx,hy,iz = a*sx,b*sy,c*sz,d*sx,e*sy,f*sz,g*sx,h*sy,i*sz
        item["vertices"] = [[ax*x+by*y+cz*z+px, dx*x+ey*y+fz*z+py, gx*x+hy*y+iz*z+pz]
                            for x,y,z in item["local"]]
        normals = []
        for nx,ny,nz in item["local_normals"]:
            x,y,z = nx/sx,ny/sy,nz/sz
            rx,ry,rz = a*x+b*y+c*z,d*x+e*y+f*z,g*x+h*y+i*z
            length = math.sqrt(rx*rx+ry*ry+rz*rz)
            normals.append([rx/length,ry/length,rz/length] if length > 1e-12 else [0,0,1])
        item["normals"] = normals
        vertices = item["vertices"]
        item["world_bounds"] = (tuple(min(p[axis] for p in vertices) for axis in range(3)),
                                tuple(max(p[axis] for p in vertices) for axis in range(3))) if vertices else ((0,0,0),(0,0,0))

    def configure_animation(self):
        clips = next((item["animation_set"].clips for item in self.instances if item["animation_set"]), [])
        self.clip_ids = {clip["label"]: clip["id"] for clip in clips}
        self.clip_selector.configure(values=list(self.clip_ids), state="readonly" if clips else "disabled")
        self.play_button.configure(state="normal" if clips else "disabled")
        default = next((clip for clip in clips if 0x80000001 in clip.get("tokens", [])), clips[0] if clips else None)
        self.clip_name.set(default["label"] if default else "No supported native animation")
        self.animation_time = 0.0
        self.animation_last = time.perf_counter()
        if self.animation_pending is not None:
            self.after_cancel(self.animation_pending)
            self.animation_pending = None
        if clips:
            self.animation_pending = self.after(1, self.animate)

    def animation_changed(self, event=None):
        if event is not None:
            self.animation_time = 0.0
        self.animation_last = time.perf_counter()
        self.apply_animation()

    def apply_animation(self):
        clip = self.clip_ids.get(self.clip_name.get())
        if clip is None:
            return
        try:
            for item in self.instances:
                animation = item["animation_set"]
                if animation is None or item["role"] != "preview":
                    continue
                item["local"], item["local_normals"] = animation.sample(clip, self.animation_time, item["mesh_source"])
                self.transform(item)
            self.redraw()
        except (ValueError, KeyError) as error:
            note = f"Animation stopped: {error}"
            if note not in self.scene_warnings:
                self.scene_warnings.append(note)
            self.playing.set(False)

    def animate(self):
        self.animation_pending = None
        if self.closed or not self.clip_ids:
            return
        now = time.perf_counter()
        elapsed = now - self.animation_last
        self.animation_last = now
        if self.playing.get() and self.winfo_ismapped():
            try:
                speed = float(self.animation_speed.get())
                if not math.isfinite(speed) or not 0.1 <= speed <= 3:
                    speed = 1.0
            except ValueError:
                speed = 1.0
            self.animation_time += elapsed * speed
            self.apply_animation()
        # Skip elapsed frames under load; do not queue catch-up work on Tk.
        self.animation_pending = self.after(max(10, 50-int((time.perf_counter()-now)*1000)), self.animate)

    def update_placement(self, position, heading, scale=None):
        """Move the preview without changing the camera or any world instance."""
        position = tuple(float(value) for value in position)
        if len(position) != 3 or not all(math.isfinite(value) for value in position):
            return
        half = (heading - self.initial_heading) * math.pi / 255
        delta = (math.cos(half), 0.0, 0.0, -math.sin(half))
        for item in self.instances:
            if item["role"] != "preview":
                continue
            factor = (1 if scale is None else scale) / item["placement_scale"]
            offset = mul(sub(item["base_position"], self.center), factor)
            item["position"] = add(position, rotate(offset, delta))
            item["quaternion"] = qmultiply(delta, item["base_quaternion"])
            item["scale"] = mul(item["base_scale"], factor)
            self.transform(item)
        self.placement = position
        self.redraw()

    def focus_placement(self):
        vertices = [v for item in self.instances if item["role"] == "preview" or item.get("selected") for v in item["vertices"]]
        if vertices:
            low = tuple(min(v[axis] for v in vertices) for axis in range(3))
            high = tuple(max(v[axis] for v in vertices) for axis in range(3))
            self.focus = mul(add(low, high), .5)
            self.distance = min(5000, max(3, math.sqrt(dot(sub(high, low), sub(high, low))) * 1.8))
        else:
            self.focus = add(self.placement, (0, 0, 1.5))
            self.distance = 24.0
        self.redraw()

    def fit(self):
        vertices = [vertex for item in self.instances for vertex in item["vertices"]]
        if not vertices:
            return
        low = tuple(min(vertex[axis] for vertex in vertices) for axis in range(3))
        high = tuple(max(vertex[axis] for vertex in vertices) for axis in range(3))
        self.focus = mul(add(low, high), 0.5)
        radius = math.sqrt(dot(sub(high, low), sub(high, low))) / 2
        self.distance = min(5000, max(4, radius * 2.5))
        self.redraw()

    def reset(self):
        self.azimuth = math.radians(-55)
        self.elevation = math.radians(28)
        self.focus_placement()

    def camera(self):
        offset = (math.cos(self.elevation)*math.cos(self.azimuth),
                  math.cos(self.elevation)*math.sin(self.azimuth), math.sin(self.elevation))
        eye = add(self.focus, mul(offset, self.distance))
        forward = mul(offset, -1)
        right = unit(cross(forward, (0, 0, 1)))
        up = cross(right, forward)
        return eye, forward, right, up

    def orbit_start(self, event):
        self.canvas.focus_set()
        tool = self.tool.get()
        if tool == "entrance" and not event.state & 8:
            hit=self.surface_hit(event,{"world"})
            if hit and self.on_surface:self.on_surface(dict(point=hit[3],cell=hit[1]),"entrance")
            return
        if tool == "dungeon" and not event.state & 8:
            from editor_gizmo import project
            hits=[]
            for door in self.dungeon_doors:
                xy=project(door["center"],self.camera(),self.canvas.winfo_width(),self.canvas.winfo_height())
                if xy:
                    distance=(xy[0]-event.x)**2+(xy[1]-event.y)**2
                    if distance<32**2:hits.append((distance,door["index"]))
            if hits and self.on_door:
                self.on_door(min(hits)[1]);return
        if tool == "terrain" and not event.state & 8:
            hit = self.surface_hit(event,{"world"},terrain_only=True)
            if hit and self.on_surface:
                if event.state & 1:
                    self.on_surface(hit[3],"sample")
                else:
                    self.terrain_dragging = bool(self.on_surface(hit[3],"begin"))
            return
        if tool in ("move", "rotate", "scale") and not event.state & 8:
            from editor_gizmo import descriptor, hit_axis
            width, height = self.canvas.winfo_width(), self.canvas.winfo_height()
            gizmo = descriptor(self.instances, tool, self.camera(), width, height)
            if gizmo is None:
                hit = self.surface_hit(event, {"world", "saved", "preview"})
                if hit and hit[2].get("object_id") is not None and self.on_select:
                    self.on_select(hit[2]["object_id"], bool(event.state & 4))
                    self.set_tool(tool)
                return
            axis = hit_axis(event.x, event.y, gizmo, self.camera(), width, height) if tool == "move" else None
            if self.on_transform and self.on_transform(tool, {}, "begin"):
                self.transform_drag = dict(tool=tool, start=(event.x, event.y), gizmo=gizmo,
                                           camera=self.camera(), width=width, height=height, axis=axis, changed=False)
            return
        if tool == "place" and self.on_stamp is not None:
            if self.placement_enabled and self.pick(event):
                self.on_stamp()
            return
        if self.tool.get() == "select":
            hit = self.surface_hit(event, {"world", "saved", "preview"})
            if hit and hit[2].get("object_id") is not None and self.on_select is not None:
                self.on_select(hit[2]["object_id"], bool(event.state & 5))
            else:
                self.status.set("No editable placement under the pointer. Native DAT scenery is a read-only reference.")
            self.drag = None
            return
        self.placement_drag = not event.state & 8 and (self.tool.get() == "place" or bool(event.state & 1))
        if self.placement_drag:
            self.pick(event)
            self.drag = None
        else:
            self.drag = (event.x, event.y)

    def orbit_drag(self, event):
        if self.terrain_dragging:
            hit=self.surface_hit(event,{"world"},terrain_only=True)
            if hit and self.on_surface:self.on_surface(hit[3],"preview")
            return
        if self.transform_drag is not None:
            from editor_gizmo import drag_options
            gesture = self.transform_drag
            if math.hypot(event.x-gesture["start"][0], event.y-gesture["start"][1]) < 3:
                return
            options = drag_options(gesture["tool"], gesture["start"], (event.x, event.y), gesture["gizmo"],
                                   gesture["camera"], gesture["width"], gesture["height"], gesture["axis"])
            gesture["changed"] = True
            self.on_transform(gesture["tool"], options, "preview")
            return
        if self.placement_drag:
            self.pick(event)
            return
        if self.drag is None:
            return
        x, y = self.drag
        eye = self.camera()[0] if self.tool.get() == "explore" else None
        self.azimuth -= (event.x - x) * 0.009
        self.elevation = min(math.radians(85), max(math.radians(-70), self.elevation + (event.y-y)*0.007))
        if eye is not None:
            # First-person look rotates around the camera eye, not a distant
            # selected object. Keeping the eye fixed prevents sideways jumps.
            self.focus = add(eye, mul(self.camera()[1], self.distance))
        self.drag = event.x, event.y
        self.redraw()

    def set_tool(self, tool):
        self.tool.set(tool)
        self.tool_changed()

    def tool_changed(self):
        if self.terrain_dragging:
            self.terrain_dragging=False
            if self.on_surface:self.on_surface(None,"cancel")
        self.brush_preview=None
        self.stop_travel()
        if self.transform_drag is not None:
            self.cancel_transform()
        for name, button in self.tool_buttons.items():
            button.configure(style="Accent.TButton" if self.tool.get() == name else "TButton")
        self.drag = None
        self.placement_drag = False
        if self.tool.get() == "terrain":
            self.tool_help.set("Terrain: drag to sculpt or paint · radius/strength in Build · Ctrl+Z undo stroke · Ctrl+S save project")
        elif self.tool.get() == "dungeon":
            self.tool_help.set("Dungeon: choose a native room template and doorway in Build · fit/connect · save/export native project")
        elif self.tool.get() == "explore":
            self.tool_help.set("Explore: drag to look · WASD / arrows: fly and strafe · Q/E: down/up · Shift: fast · wheel: zoom\n"
                               "Camera travel does not move the placement. Choose Place / move to edit an object.")
        elif self.tool.get() in ("move", "rotate", "scale"):
            self.tool_help.set({"move": "Drag the selected object on the ground plane, or drag a red/green/blue axis. Release applies one undoable edit.",
                                "rotate": "Drag left/right to rotate the selection. Release applies; Escape cancels.",
                                "scale": "Drag up/down to scale the selection. Release applies; Escape cancels."}[self.tool.get()])
        elif self.tool.get() == "select":
            self.tool_help.set("Select: click an authored object / Ctrl-click to add or remove from selection.\n"
                               "Use Arrange selection for group movement, rotation and spaced copies.")
        else:
            self.tool_help.set("Place: choose an object, hover over a world surface, then click to place a copy. Escape cancels.\n"
                               "Alt-drag: orbit · right-drag: pan · wheel: zoom · Ctrl+Z: undo")
    def get_navigation_point(self):
        """Camera position in scene coordinates for explicit area loading."""
        return self.camera()[0]

    def stop_travel(self):
        self.held_keys.clear()
        if self.travel_pending is not None:
            self.after_cancel(self.travel_pending)
            self.travel_pending = None

    def key_input(self, key, state=0, pressed=True):
        key = key.lower()
        movement = {"w": (0, 1, 0), "s": (0, -1, 0), "a": (-1, 0, 0), "d": (1, 0, 0),
                    "q": (0, 0, -1), "e": (0, 0, 1), "up": (0, 1, 0), "down": (0, -1, 0),
                    "left": (-1, 0, 0), "right": (1, 0, 0), "prior": (0, 0, 1), "next": (0, 0, -1)}
        if not pressed:
            self.held_keys.pop(key, None)
            return "break"
        if self.tool.get()=="terrain" and key in ("bracketleft","bracketright","[","]") and self.on_surface:
            self.on_surface(None,"resize-smaller" if key in ("bracketleft","[") else "resize-larger")
            return "break"
        commands = {"s": "save", "f": "search", "z": "redo" if state & 1 else "undo", "y": "redo", "d": "duplicate"}
        if state & 4 and key in commands and self.on_command is not None:
            self.on_command(commands[key])
            return "break"
        if self.tool.get() == "explore" and key in movement:
            self.held_keys[key] = (movement[key], state)
            if self.travel_pending is None:
                self.travel_last = time.perf_counter()
                self.advance_travel(.04)
                self.travel_pending = self.after(16, self.travel_tick)
            return "break"
        if key == "escape":
            if self.terrain_dragging:
                self.terrain_dragging=False
                if self.on_surface:self.on_surface(None,"cancel")
            self.stop_travel()
            self.cancel_transform()
            if self.on_command:
                self.on_command("cancel")
        elif key == "delete" and self.on_command:
            self.on_command("delete")
        elif self.tool.get() != "explore" and key in ("q", "w", "e", "r"):
            self.set_tool({"q": "select", "w": "move", "e": "rotate", "r": "scale"}[key])
        elif key == "f":
            self.focus_placement()
        elif key == "home":
            self.fit()
        elif key in ("p", "o"):
            self.set_tool("place" if key == "p" else "orbit")
        elif key in movement:
            from types import SimpleNamespace
            self.nudge(SimpleNamespace(state=state), movement[key])
        return "break"

    def travel_tick(self):
        self.travel_pending = None
        if self.closed or self.tool.get() != "explore" or not self.canvas.winfo_ismapped():
            self.held_keys.clear()
            return
        now = time.perf_counter()
        elapsed, self.travel_last = min(.1, max(0, now-self.travel_last)), now
        if not self.held_keys:
            return
        self.advance_travel(elapsed)
        self.travel_pending = self.after(16, self.travel_tick)

    def advance_travel(self, elapsed):
        if not self.held_keys:
            return
        movement = tuple(sum(row[0][axis] for row in self.held_keys.values()) for axis in range(3))
        state = next(iter(self.held_keys.values()))[1]
        try:
            speed = float(self.travel_speed.get())
            if not math.isfinite(speed) or not 1 <= speed <= 500:
                speed = 20
        except ValueError:
            speed = 20
        speed *= 4 if state & 1 else .2 if state & 4 else 1
        _, forward, right, _ = self.camera()
        direction = add(add(mul(right, movement[0]), mul(forward, movement[1])), (0, 0, movement[2]))
        if dot(direction, direction) > 1e-12:
            self.focus = add(self.focus, mul(unit(direction), speed*elapsed))
        self.redraw()
        if self.on_camera_move is not None:
            self.on_camera_move(self.focus)

    def explore_key(self, event, movement):
        if self.tool.get() != "explore":
            return None
        step = max(.5, min(10., self.distance * .08))
        step *= 5 if event.state & 1 else .2 if event.state & 4 else 1
        _, forward, right, _ = self.camera()
        delta = add(add(mul(right, movement[0] * step), mul(forward, movement[1] * step)),
                    (0, 0, movement[2] * step))
        self.focus = add(self.focus, delta)
        self.redraw()
        if self.on_camera_move is not None:
            self.on_camera_move(self.focus)
        return "break"

    def pan_start(self, event):
        self.canvas.focus_set()
        self.placement_drag = False
        self.drag = event.x, event.y

    def pan_drag(self, event):
        if self.drag is None:
            return
        x, y = self.drag
        _, _, right, up = self.camera()
        factor = self.distance / max(100, self.canvas.winfo_height())
        self.focus = add(self.focus, add(mul(right, -(event.x-x)*factor), mul(up, (event.y-y)*factor)))
        self.drag = event.x, event.y
        self.redraw()

    def end_drag(self):
        if self.terrain_dragging:
            self.terrain_dragging=False
            if self.on_surface:self.on_surface(None,"commit")
        if self.transform_drag is not None:
            gesture, self.transform_drag = self.transform_drag, None
            if self.on_transform:
                self.on_transform(gesture["tool"], {}, "commit" if gesture["changed"] else "cancel")
        self.drag = None
        self.placement_drag = False

    def nudge(self, event, movement):
        if self.tool.get() == "explore":
            return self.explore_key(event, movement)
        try:
            step = float(self.move_step.get())
            if not math.isfinite(step) or not .001 <= step <= 100:
                raise ValueError()
        except ValueError:
            self.status.set("Move step must be a number between 0.001 and 100.")
            return "break"
        step *= .1 if event.state & 1 else 10 if event.state & 4 else 1
        if self.on_nudge is not None:
            self.on_nudge(tuple(value * step for value in movement))
        elif self.on_pick is not None:
            self.picked_cell = None
            self.on_pick(add(self.placement, mul(movement, step)))
        return "break"

    def zoom(self, event):
        if event.delta:
            self.zoom_steps(event.delta / 120)

    def zoom_steps(self, steps):
        self.distance = min(5000, max(0.5, self.distance * math.exp(-0.14 * max(-8, min(8, steps)))))
        self.redraw()

    def redraw(self):
        if not self.closed and self.pending is None:
            self.pending = self.after_idle(self.draw)

    def visible_instances(self):
        for item in self.instances:
            # Copying a saved placement can put the preview exactly on top of
            # it. Draw that geometry once; the saved instance reappears as soon
            # as the preview moves, rotates or changes size.
            if item["role"] == "saved" and any(
                other["role"] == "preview" and other["did"] == item["did"]
                and all(abs(a-b) < 0.0001 for a,b in zip(other["position"],item["position"]))
                and all(abs(a-b) < 0.0001 for a,b in zip(other["scale"],item["scale"]))
                and abs(dot(other["quaternion"],item["quaternion"])) > 0.99999
                for other in self.instances
            ):
                continue
            yield item

    def visible_triangles(self, include_surface=False, candidates=None):
        height = self.cutaway_height()
        cutaway = self.cutaway.get()
        for item in self.visible_instances():
            if candidates is not None and id(item) not in candidates:
                continue
            points, indices = item["vertices"], item["indices"]
            colors = item["triangle_colors"]
            for offset in range(0, len(indices), 3):
                a, b, c = (points[index] for index in indices[offset:offset+3])
                if cutaway and item["role"] == "world" and min(a[2], b[2], c[2]) > height:
                    continue
                color_index = offset // 3
                color = colors[color_index] if color_index < len(colors) else self.COLORS.get(item["role"], self.COLORS["world"])
                triangle = (a, b, c, color, item["role"], item["cell"])
                yield triangle + (item, offset) if include_surface else triangle

    def cutaway_height(self):
        if self.tool.get() == "dungeon":
            vertices=[v for item in self.instances if item.get("role")=="world" for v in item["vertices"]]
            if vertices:
                low,high=min(v[2] for v in vertices),max(v[2] for v in vertices)
                return low+(high-low)*.65
        return (self.focus[2] if self.tool.get() == "explore" else self.placement[2]) + 3.5

    def draw(self):
        self.pending = None
        if self.closed:
            return
        width, height = self.canvas.winfo_width(), self.canvas.winfo_height()
        if width < 40 or height < 40:
            return
        if not self.instances:
            if self.gpu:
                self.gpu.close()
                self.gpu = None
            self.canvas.delete("all")
            self.canvas.create_text(width/2, height/2, text="No native geometry loaded", fill="#bed0df")
            self.status.set(" · ".join(self.scene_warnings + self.geometry_warnings) or "Choose a cell and entity to load their DAT geometry.")
            return
        eye, forward, right, up = self.camera()
        if self.gpu_error is None:
            try:
                from preview_gl import GLRenderer
                if self.gpu is None:
                    self.canvas.delete("all")
                    logging.info("Creating native OpenGL viewport")
                    self.gpu = GLRenderer(self.canvas)
                    logging.info("Native OpenGL viewport ready: %s",self.gpu.info)
                from editor_gizmo import descriptor
                count = self.gpu.render(list(self.visible_instances()), eye, forward, right, up,
                                        width, height, self.cutaway_height() if self.cutaway.get() else None,
                                        gizmo=descriptor(self.instances, self.tool.get(), self.camera(), width, height), brush=self.brush_preview if self.tool.get()=="terrain" else None, doors=self.dungeon_doors if self.tool.get()=="dungeon" else None, portals=self.portal_markers)
                self.status.set(f"Textured 3D view · {count:,} triangles"
                                + (f" · {len(self.scene_warnings)} loading notes in Details" if self.scene_warnings else ""))
                return
            except (RuntimeError, OSError, ValueError, ImportError) as error:
                logging.exception("Native OpenGL unavailable; using software renderer")
                if self.gpu:
                    self.gpu.close()
                    self.gpu = None
                self.gpu_error = str(error)
        self.canvas.delete("all")
        focal = min(width, height) * 0.92
        light = unit((0.25, -0.4, 0.85))
        triangles = []
        for a, b, c, color, role, cell in self.visible_triangles():
            normal = cross(sub(b, a), sub(c, a))
            if dot(normal, normal) < 1e-18:
                continue
            camera_points = []
            for point in (a, b, c):
                offset = sub(point, eye)
                camera_points.append((dot(offset, right), dot(offset, up), dot(offset, forward)))
            # Clip against the near plane rather than dropping a large crossing floor.
            clipped = []
            for index, point in enumerate(camera_points):
                prior = camera_points[index-1]
                inside, prior_inside = point[2] >= 0.15, prior[2] >= 0.15
                if inside != prior_inside:
                    amount = (0.15-prior[2]) / (point[2]-prior[2])
                    clipped.append(add(prior, mul(sub(point, prior), amount)))
                if inside:
                    clipped.append(point)
            if len(clipped) < 3:
                continue
            projected = [(width/2 + focal*point[0]/point[2], height/2 - focal*point[1]/point[2]) for point in clipped]
            if max(point[0] for point in projected) < 0 or min(point[0] for point in projected) > width:
                continue
            if max(point[1] for point in projected) < 0 or min(point[1] for point in projected) > height:
                continue
            shade = 0.37 + 0.63 * abs(dot(unit(normal), light))
            rgb = tuple(max(0, min(255, int(float(channel) * shade))) for channel in color[:3])
            fill = "#%02x%02x%02x" % rgb
            triangles.append((sum(point[2] for point in clipped)/len(clipped), projected, fill))
        triangles.sort(key=lambda item: item[0], reverse=True)
        for _, points, fill in triangles:
            self.canvas.create_polygon(*[coordinate for point in points for coordinate in point], fill=fill, outline="")
        offset = sub(self.placement, eye)
        depth = dot(offset, forward)
        if depth > 0.15:
            x, y = width/2 + focal*dot(offset, right)/depth, height/2 - focal*dot(offset, up)/depth
            self.canvas.create_line(x-7, y, x+7, y, fill="#ffcb75", width=2)
            self.canvas.create_line(x, y-7, x, y+7, fill="#ffcb75", width=2)
        self.canvas.create_text(10, 10, anchor="nw", fill="#d7e4ef", text=f"{len(triangles):,} visible faces · orbit / pan / zoom")
        warnings = self.scene_warnings + self.geometry_warnings
        self.status.set("Software fallback: sampled material colors; native OpenGL unavailable."
                        + (f" {len(set(warnings))} loading notes; see Details." if warnings else ""))

    def details(self):
        window = tk.Toplevel(self)
        window.title("Native preview details")
        window.geometry("760x400")
        text = tk.Text(window, wrap="word", padx=12, pady=12)
        scroll = ttk.Scrollbar(window, orient="vertical", command=text.yview)
        text.configure(yscrollcommand=scroll.set)
        text.pack(side="left", fill="both", expand=True)
        scroll.pack(side="right", fill="y")
        notes = list(dict.fromkeys(self.scene_warnings + self.geometry_warnings))
        text.insert("end", "Actual DAT geometry with native UV textures, depth buffering and preview lighting.\n"
                    "Complete native material shaders, blending and visibility/BSP evaluation remain unfinished.\n\n")
        if self.gpu:
            text.insert("end", "OpenGL: " + str(self.gpu.info) + "\n\n")
            notes.extend(self.gpu.warnings)
        if self.gpu_error:
            text.insert("end", "OpenGL unavailable: " + self.gpu_error + "\n\n")
        text.insert("end", "\n\n".join(notes) if notes else "All requested geometry loaded without additional notes.")
        text.configure(state="disabled")

    def surface_hit(self, event, roles, terrain_only=False):
        width, height = self.canvas.winfo_width(), self.canvas.winfo_height()
        focal = min(width, height) * 0.92
        if focal <= 0:
            return
        eye, forward, right, up = self.camera()
        direction = unit(add(forward, add(mul(right, (event.x-width/2)/focal), mul(up, (height/2-event.y)/focal))))
        best = None
        best_cell = None
        best_item = None
        from preview_gl import surface_hit_visible
        from editor_gizmo import ray_bounds
        candidates = set()
        for item in self.instances:
            if item.get("role") not in roles or terrain_only and not item.get("terrain"):
                continue
            bounds = item.get("world_bounds")
            if bounds is None:
                points = item["vertices"]
                bounds = (tuple(min(p[axis] for p in points) for axis in range(3)), tuple(max(p[axis] for p in points) for axis in range(3))) if points else None
            if bounds is not None and ray_bounds(eye, direction, bounds):
                candidates.add(id(item))
        for a, b, c, _, role, cell, item, offset in self.visible_triangles(include_surface=True, candidates=candidates):
            if role not in roles:
                continue
            edge1, edge2 = sub(b, a), sub(c, a)
            p = cross(direction, edge2)
            determinant = dot(edge1, p)
            if abs(determinant) < 1e-9:
                continue
            inverse = 1 / determinant
            t = sub(eye, a)
            u = dot(t, p) * inverse
            if u < 0 or u > 1:
                continue
            q = cross(t, edge1)
            v = dot(direction, q) * inverse
            if v < 0 or u + v > 1:
                continue
            distance = dot(edge2, q) * inverse
            if role == "world" and self.cutaway.get() and eye[2] + direction[2] * distance > self.cutaway_height() + 1e-6:
                # GL clips crossing triangles against this plane, whereas the
                # software iterator only rejects wholly clipped triangles.
                # Do not let an invisible upper wall intercept a floor pick.
                continue
            if not surface_hit_visible(item, offset, u, v):
                continue
            if distance > 0.15 and (best is None or distance < best):
                best = distance
                best_cell = cell
                best_item = item
        return (best, best_cell, best_item, add(eye, mul(direction, best))) if best is not None else None

    def pick(self, event):
        if self.on_pick is None:
            return
        hit = self.surface_hit(event, {"world"})
        if hit:
            self.picked_cell = hit[1]
            self.on_pick(hit[3])
            return True
        else:
            self.status.set("No loaded world surface under that point. Orbit or zoom to a visible room surface.")
            return False

    def toggle_animation_controls(self):
        if self.animation_bar.winfo_manager():
            self.animation_bar.pack_forget()
        else:
            self.animation_bar.pack(fill="x", pady=(0, 4), before=self.canvas)

    def hover_placement(self, event):
        if self.tool.get() == "terrain" and not event.state & 0x500:
            hit=self.surface_hit(event,{"world"},terrain_only=True)
            if hit and self.on_surface:self.on_surface(hit[3],"hover")
            return
        if self.tool.get() != "place" or not self.placement_enabled or event.state & 0x500:
            return
        now = time.perf_counter()
        if now-self.hover_last >= .09:
            self.hover_last = now
            self.pick(event)

    def cancel_transform(self):
        if self.transform_drag is not None:
            gesture, self.transform_drag = self.transform_drag, None
            if self.on_transform:
                self.on_transform(gesture["tool"], {}, "cancel")

    def set_selection(self, identities):
        identities = set(identities)
        for item in self.instances:
            item["selected"] = item.get("object_id") in identities
        self.redraw()

    def clear_arrangement(self):
        if self.arrangement_base is not None:
            self.instances = [item for item, state in self.arrangement_base]
            for item, state in self.arrangement_base:
                item.clear()
                item.update(state)
            self.arrangement_base = None
            self.redraw()

    def preview_arrangement(self, rows, original_rows, reference, landscape=None, duplicate=False):
        self.clear_arrangement()
        self.arrangement_base = [(item, dict(item)) for item in self.instances]
        original = {int(row[0], 16): row for row in original_rows}
        sources = {}
        for item in self.instances:
            identity = item.get("object_id")
            if identity in original:
                sources.setdefault(identity, []).append(item)
        additions = []
        for row in rows:
            identity = int(row[0], 16)
            for source in sources.get(identity, []):
                item = dict(source) if duplicate else source
                origin = [float(value) for value in row[4:7]]
                cell = int(row[3], 16)
                if landscape and 1 <= (cell & 65535) <= 64 and 1 <= (reference & 65535) <= 64:
                    origin[0] += ((cell >> 24) - (reference >> 24)) * landscape["block_side"]
                    origin[1] += (((cell >> 16) & 255) - ((reference >> 16) & 255)) * landscape["block_side"]
                base = original[identity]
                factor = float(row[11]) if len(row) == 12 and row[11] != "-" else 1
                old_factor = float(base[11]) if len(base) == 12 and base[11] != "-" else 1
                item["position"] = tuple(origin)
                item["quaternion"] = tuple(float(value) for value in row[7:11])
                item["scale"] = mul(source["base_scale"], factor / old_factor)
                item["selected"] = True
                if duplicate:
                    item["role"], item["object_id"] = "preview", None
                    item["animation_set"] = None
                    additions.append(item)
                self.transform(item)
        self.instances.extend(additions)
        self.redraw()

    def close(self):
        self.stop_travel()
        self.closed = True
        if self.paint_poll_pending is not None:
            try:
                self.after_cancel(self.paint_poll_pending)
            except tk.TclError:
                pass
            self.paint_poll_pending = None
        if self.animation_pending is not None:
            try:
                self.after_cancel(self.animation_pending)
            except tk.TclError:
                pass
            self.animation_pending = None
        if self.gpu:
            self.gpu.close()
            self.gpu = None
        if self.pending is not None:
            try:
                self.after_cancel(self.pending)
            except tk.TclError:
                pass
            self.pending = None
