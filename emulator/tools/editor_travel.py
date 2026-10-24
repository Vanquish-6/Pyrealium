"""Native AC2 world atlas and direct geographic travel, independent of objects."""
from pathlib import Path
import struct
import tkinter as tk
from tkinter import ttk

from ac2_dat import Dat
from preview_terrain import region_references, landscape_defs


def map_surface(data, identity):
    if len(data) < 24 or len(data) > 64*1024*1024:
        raise ValueError("World map surface is outside supported bounds")
    did, category, width, height, pixel_format, size = struct.unpack_from("<6I", data)
    if did != identity or not 1 <= width <= 4096 or not 1 <= height <= 4096:
        raise ValueError("Invalid world map surface identity or dimensions")
    if pixel_format != 20 or size != width*height*3 or len(data) != size+24:
        raise ValueError("World map requires the native 24-bit surface profile")
    source = data[24:]
    rgb = bytearray(size)
    rgb[0::3], rgb[1::3], rgb[2::3] = source[2::3], source[1::3], source[0::3]
    return dict(width=width, height=height, ppm=f"P6\n{width} {height}\n255\n".encode("ascii")+rgb)


def load_atlas(folder):
    with Dat(Path(folder)/"portal.dat") as portal:
        refs = region_references(portal.read(0x0f000000))
        definitions = landscape_defs(portal.read(refs["land_defs"]))
        image = None
        for identity in (refs["map_surface"], refs["auto_map_surface"]):
            entry = portal.index().get(identity)
            if entry is None:
                continue
            if not 24 <= entry[1] <= 64*1024*1024:
                raise ValueError("World map record is outside supported bounds")
            try:
                image = map_surface(portal.read(identity), identity)
                break
            except ValueError:
                continue
        if image is None:
            raise ValueError("No supported native world map surface was found in this DAT set")
        return dict(image=image, definitions=definitions, columns=refs["map_width"], rows=refs["map_height"])


def atlas_cell(x, y, columns, rows, cells):
    """Normalized map position -> native outdoor cell; north is image-up."""
    if not 0 <= x < 1 or not 0 <= y < 1 or not 1 <= columns <= 255 or not 1 <= rows <= 255:
        raise ValueError("Choose a point inside the world map")
    wx, wy = x*columns, (1-y)*rows
    bx, by = int(wx), min(rows-1, int(wy))
    ix, iy = min(cells-1, int((wx-bx)*cells)), min(cells-1, int((wy-by)*cells))
    return (bx << 24) | (by << 16) | (1+ix*cells+iy)


class WorldAtlas(ttk.Frame):
    def __init__(self, parent, on_travel, compact=False):
        super().__init__(parent)
        self.on_travel, self.data = on_travel, None
        self.compact = compact
        self.photo, self.shown = None, None
        self.zoom = 1
        self.pan = [0, 0]
        self.marker = None
        self.transform = None
        self.note = tk.StringVar(value="Loading the native AC2 world map…")
        title = ttk.Frame(self, padding=(12, 10))
        title.pack(fill="x")
        ttk.Label(title, text="World map" if compact else "Travel the world", style="Section.TLabel").pack(side="left")
        ttk.Button(title, text="Fit map", command=self.fit).pack(side="right")
        if not compact:
            ttk.Label(self, text="Click anywhere on the map to travel there. Wheel to zoom; right-drag to pan.", style="Muted.TLabel").pack(anchor="w", padx=12, pady=(0, 8))
        self.canvas = tk.Canvas(self, background="#101a27", highlightthickness=0)
        self.canvas.pack(fill="both", expand=True)
        ttk.Label(self, textvariable=self.note, wraplength=250 if compact else 950).pack(anchor="w", padx=12, pady=8)
        self.canvas.bind("<Configure>", lambda event: self.draw())
        self.canvas.bind("<Button-1>", self.travel)
        self.canvas.bind("<MouseWheel>", self.zoom_map)
        self.canvas.bind("<ButtonPress-3>", lambda event: setattr(self, "drag", (event.x, event.y, *self.pan)))
        self.canvas.bind("<B3-Motion>", self.pan_map)

    def set_atlas(self, data):
        self.data = data
        self.photo = tk.PhotoImage(data=data["image"]["ppm"], format="PPM")
        self.fit()
        self.note.set("Click a region to open it in 3D. Travel does not change placed objects.")

    def fit(self):
        self.zoom, self.pan = 1, [0, 0]
        self.shown = self.photo.subsample(2) if self.compact and self.photo is not None else self.photo
        self.draw()

    def draw(self):
        self.canvas.delete("all")
        width, height = self.canvas.winfo_width(), self.canvas.winfo_height()
        if self.shown is None:
            self.canvas.create_text(width/2, height/2, text=self.note.get(), fill="#c6d8eb", width=max(100, width-50))
            return
        w, h = self.shown.width(), self.shown.height()
        x, y = (width-w)/2+self.pan[0], (height-h)/2+self.pan[1]
        self.transform = (x, y, w, h)
        self.canvas.create_image(x, y, image=self.shown, anchor="nw")
        if self.marker:
            cell, point = self.marker
            block = self.data["definitions"]["block_side"]
            mx = ((cell >> 24)+point[0]/block)/self.data["columns"]
            my = 1-(((cell >> 16)&255)+point[1]/block)/self.data["rows"]
            px, py = x+mx*w, y+my*h
            self.canvas.create_oval(px-5, py-5, px+5, py+5, fill="#ffd479", outline="#142233", width=2)
            self.canvas.create_text(px+10, py-10, text="You are here", anchor="w", fill="white")

    def travel(self, event):
        if not self.data or not self.transform:
            return
        x, y, w, h = self.transform
        try:
            cell = atlas_cell((event.x-x)/w, (event.y-y)/h, self.data["columns"], self.data["rows"], self.data["definitions"]["cells"])
            self.note.set("Opening the selected region in 3D…")
            self.on_travel(cell)
        except ValueError as error:
            self.note.set(str(error))

    def zoom_map(self, event):
        if self.photo is None:
            return
        self.zoom = max(1, min(4, self.zoom+(1 if event.delta > 0 else -1)))
        self.shown = self.photo.zoom(self.zoom)
        if self.compact:
            self.shown = self.shown.subsample(2)
        self.draw()

    def pan_map(self, event):
        if getattr(self, "drag", None):
            x, y, px, py = self.drag
            self.pan = [px+event.x-x, py+event.y-y]
            self.draw()
