"""A lightweight palette swatch drawn from the selected native model geometry."""
import math
import tkinter as tk
from tkinter import ttk


class ModelSwatch(ttk.Frame):
    def __init__(self, parent):
        super().__init__(parent)
        self.canvas = tk.Canvas(self, height=150, background="#101a27", highlightthickness=0)
        self.canvas.pack(fill="x")
        self.label = ttk.Label(self, text="Choose an object to preview its native model.", wraplength=270)
        self.label.pack(anchor="w", pady=4)
        self.model = []
        self.canvas.bind("<Configure>", lambda event: self.draw())

    def set_model(self, instances, label):
        self.model = [item for item in instances if item.get("role") == "preview"]
        self.label.configure(text=label)
        self.draw()

    def draw(self):
        canvas = self.canvas
        canvas.delete("all")
        width, height = canvas.winfo_width(), canvas.winfo_height()
        vertices = [point for item in self.model for point in item.get("vertices", ())]
        if not vertices or width < 20:
            canvas.create_text(max(80,width/2), height/2, text="Model preview", fill="#91a8c1")
            return
        center = tuple((min(p[i] for p in vertices)+max(p[i] for p in vertices))/2 for i in range(3))
        right, up, forward = (.819,.574,0), (-.27,.386,.882), (.506,-.723,.47)
        def coordinate(point):
            local = tuple(point[i]-center[i] for i in range(3))
            return tuple(sum(local[i]*axis[i] for i in range(3)) for axis in (right,up,forward))
        points = [coordinate(p) for p in vertices]
        span_x = max(p[0] for p in points)-min(p[0] for p in points)
        span_y = max(p[1] for p in points)-min(p[1] for p in points)
        scale = min((width-25)/max(.01,span_x),(height-18)/max(.01,span_y))
        faces = []
        total = sum(len(item["indices"])//3 for item in self.model)
        stride = max(1, math.ceil(total/2200))
        for item in self.model:
            local = [coordinate(p) for p in item["vertices"]]
            for offset in range(0,len(item["indices"]),3*stride):
                triangle = [local[i] for i in item["indices"][offset:offset+3]]
                if len(triangle) != 3:
                    continue
                colors = item.get("triangle_colors", ())
                color = colors[offset//3] if offset//3 < len(colors) else (156,178,195)
                fill = "#%02x%02x%02x" % tuple(max(0,min(255,int(v))) for v in color[:3])
                faces.append((sum(p[2] for p in triangle), [(width/2+p[0]*scale,height/2-p[1]*scale) for p in triangle],fill))
        for depth, triangle, fill in sorted(faces, key=lambda face: face[0]):
            canvas.create_polygon(*[value for point in triangle for value in point],fill=fill,outline="")
