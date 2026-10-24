"""Dependency-free interactive viewport for decoded native DAT triangles.

This draws supplied geometry, without synthesizing creatures or room surfaces.
The painter renderer uses flat lighting and optional texture-sampled triangle
colors. It is a placement aid, not the native client's animated renderer.
"""
import math
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
    TRIANGLE_LIMIT = 30000
    COLORS = {"world": (113, 132, 145), "saved": (76, 159, 213), "preview": (229, 172, 75)}

    def __init__(self, parent, on_pick=None):
        super().__init__(parent)
        self.on_pick = on_pick
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
        self.closed = False
        controls = ttk.Frame(self)
        controls.pack(fill="x", pady=(0, 4))
        ttk.Button(controls, text="Focus creature (F)", command=self.focus_placement).pack(side="left")
        ttk.Button(controls, text="Fit scene", command=self.fit).pack(side="left", padx=5)
        ttk.Button(controls, text="Reset view", command=self.reset).pack(side="left")
        ttk.Button(controls, text="Details", command=self.details).pack(side="left", padx=5)
        self.cutaway = tk.BooleanVar(value=True)
        ttk.Checkbutton(controls, text="Cut away ceiling", variable=self.cutaway,
                        command=self.redraw).pack(side="right")
        self.canvas = tk.Canvas(self, background="#17212c", highlightthickness=0, height=330)
        self.canvas.pack(fill="both", expand=True)
        self.status = tk.StringVar(value="Choose a game data folder and load the native geometry.")
        ttk.Label(self, text="Drag: orbit · right-drag: pan · wheel: zoom · Shift-click: place on world geometry",
                  wraplength=650).pack(anchor="w", pady=(5, 0))
        ttk.Label(self, textvariable=self.status, wraplength=650).pack(anchor="w", pady=(3, 0))
        self.canvas.bind("<Configure>", lambda event: self.redraw())
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
        self.canvas.bind("<Enter>", lambda event: self.canvas.focus_set())
        self.canvas.bind("<Destroy>", lambda event: self.close())

    def set_scene(self, instances, center, warnings=(), heading=0):
        """Replace the scene. center is the initial selected placement anchor."""
        instances = list(instances)
        self.center = tuple(float(value) for value in center)
        self.placement = self.center
        self.initial_heading = heading
        self.scene_warnings = list(warnings)
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
        self.focus_placement()

    @staticmethod
    def transform(item):
        scale, quaternion, position = item["scale"], item["quaternion"], item["position"]
        item["vertices"] = [add(rotate(tuple(value * factor for value, factor in zip(vertex, scale)), quaternion), position)
                            for vertex in item["local"]]

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
        if event.state & 1:
            self.pick(event)
            self.drag = None
        else:
            self.drag = (event.x, event.y)

    def orbit_drag(self, event):
        if self.drag is None:
            return
        x, y = self.drag
        self.azimuth -= (event.x - x) * 0.009
        self.elevation = min(math.radians(85), max(math.radians(-70), self.elevation + (event.y-y)*0.007))
        self.drag = event.x, event.y
        self.redraw()

    def pan_start(self, event):
        self.canvas.focus_set()
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
        self.drag = None

    def zoom(self, event):
        if event.delta:
            self.zoom_steps(event.delta / 120)

    def zoom_steps(self, steps):
        self.distance = min(5000, max(0.5, self.distance * math.exp(-0.14 * max(-8, min(8, steps)))))
        self.redraw()

    def redraw(self):
        if not self.closed and self.pending is None:
            self.pending = self.after_idle(self.draw)

    def visible_triangles(self):
        height = self.placement[2] + 3.5
        cutaway = self.cutaway.get()
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
            points, indices = item["vertices"], item["indices"]
            colors = item["triangle_colors"]
            for offset in range(0, len(indices), 3):
                a, b, c = (points[index] for index in indices[offset:offset+3])
                if cutaway and item["role"] == "world" and min(a[2], b[2], c[2]) > height:
                    continue
                color_index = offset // 3
                color = colors[color_index] if color_index < len(colors) else self.COLORS.get(item["role"], self.COLORS["world"])
                yield a, b, c, color, item["role"]

    def draw(self):
        self.pending = None
        if self.closed:
            return
        width, height = self.canvas.winfo_width(), self.canvas.winfo_height()
        if width < 40 or height < 40:
            return
        self.canvas.delete("all")
        if not self.instances:
            self.canvas.create_text(width/2, height/2, text="No native geometry loaded", fill="#bed0df")
            self.status.set(" · ".join(self.scene_warnings + self.geometry_warnings) or "Choose a cell and entity to load their DAT geometry.")
            return
        eye, forward, right, up = self.camera()
        focal = min(width, height) * 0.92
        light = unit((0.25, -0.4, 0.85))
        triangles = []
        for a, b, c, color, role in self.visible_triangles():
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
        self.status.set("DAT geometry and sampled material colors; bind pose; animation pending."
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
        text.insert("end", "Actual DAT geometry with sampled material colors and flat lighting.\n"
                    "The selected model uses its stored bind pose. Animation, full per-pixel textures,\n"
                    "complete material shaders and native visibility/BSP evaluation are unfinished.\n\n")
        text.insert("end", "\n\n".join(notes) if notes else "All requested geometry loaded without additional notes.")
        text.configure(state="disabled")

    def pick(self, event):
        if self.on_pick is None:
            return
        width, height = self.canvas.winfo_width(), self.canvas.winfo_height()
        focal = min(width, height) * 0.92
        if focal <= 0:
            return
        eye, forward, right, up = self.camera()
        direction = unit(add(forward, add(mul(right, (event.x-width/2)/focal), mul(up, (height/2-event.y)/focal))))
        best = None
        for a, b, c, _, role in self.visible_triangles():
            if role != "world":
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
            if distance > 0.15 and (best is None or distance < best):
                best = distance
        if best is not None:
            self.on_pick(add(eye, mul(direction, best)))
        else:
            self.status.set("No loaded world surface under that point. Orbit or zoom to a visible room surface.")

    def close(self):
        self.closed = True
        if self.pending is not None:
            try:
                self.after_cancel(self.pending)
            except tk.TclError:
                pass
            self.pending = None
