"""Windows OpenGL viewport for native DAT preview geometry and UV textures.

Uses only ctypes and the Windows OpenGL 1.1 entry points. The child window owns
its device context and does not alter the pixel format of Tk's own windows.
Documentation: https://learn.microsoft.com/windows/win32/opengl/pixel-format-functions
https://learn.microsoft.com/windows/win32/opengl/glvertexpointer
This is an editor renderer, not an implementation of the original client shaders.
"""
from collections import OrderedDict, deque
import ctypes as C
import math
import os


class GLUnavailable(RuntimeError):
    pass


def material_rgba(item, material):
    """Separate native diffuse RGBA from the sampled fallback texture color."""
    diffuse = item.get("material_diffuse", {}).get(material)
    if diffuse is not None:
        if len(diffuse) != 4 or any(not math.isfinite(value) for value in diffuse):
            raise ValueError("Invalid native material diffuse RGBA")
        return tuple(max(0., min(1., float(value))) for value in diffuse)
    color = item.get("material_colors", {}).get(material, (150, 164, 174))
    if len(color) < 3 or any(not math.isfinite(value) for value in color):
        raise ValueError("Invalid fallback material color")
    return tuple(max(0., min(1., float(value) / 255)) for value in color[:3]) + (1.,)


def material_state(item, material, image=None):
    return item.get("material_states", {}).get(material,
                image.get("render_state", {}) if image is not None else {})


def diffuse_alpha_used(state):
    sources = state.get("alpha_sources")
    return "diffuse" in sources if sources is not None else bool(state.get("diffuse_alpha_only"))


def sample_alpha(image, uv):
    """Match the renderer's GL_LINEAR / GL_REPEAT alpha at native UVs."""
    width, height = image["width"], image["height"]
    alpha = image.get("alpha")
    if not alpha or len(alpha) != width * height:
        return 1.
    x, y = uv[0] * width - .5, uv[1] * height - .5
    left, top = math.floor(x), math.floor(y)
    dx, dy = x - left, y - top
    def pixel(px, py):
        return alpha[(py % height) * width + px % width] / 255
    return ((1-dy) * ((1-dx)*pixel(left, top) + dx*pixel(left+1, top)) +
            dy * ((1-dx)*pixel(left, top+1) + dx*pixel(left+1, top+1)))


def surface_hit_visible(item, offset, bary_u, bary_v):
    """Transparent cutout texels must not intercept a floor-placement ray."""
    group = next((group for group in item.get("groups", ())
                  if group["start"] <= offset < group["start"] + group["count"]), None)
    if group is None:
        return True
    material = group["material"]
    image = item.get("material_images", {}).get(material)
    state = material_state(item, material, image)
    if "unknown" in state.get("alpha_sources", ()):
        return True  # Unknown shader operations cannot prove invisibility.
    alpha = material_rgba(item, material)[3] if diffuse_alpha_used(state) else 1.
    if state.get("texture_alpha") and image is not None:
        indices, uvs = item["indices"][offset:offset+3], item.get("uvs", ())
        if len(indices) != 3 or any(index >= len(uvs) for index in indices):
            return True
        weights = (1-bary_u-bary_v, bary_u, bary_v)
        uv = tuple(sum(weight * uvs[index][axis] for weight, index in zip(weights, indices)) for axis in range(2))
        alpha *= sample_alpha(image, uv)
    if state.get("alpha_test") and alpha <= state.get("alpha_reference", 0):
        return False
    if (state.get("source_blend"), state.get("destination_blend")) == (5, 6) and alpha <= 1e-8:
        return False
    return True


def material_draw_commands(instances, eye):
    """Opaque first, then native blended material groups from far to near.

    Sorting groups rather than entire instances handles mixed opaque/cutout
    meshes. This editor order retains each material's authored depth-write bit.
    Static group centroids are cached against the retained geometry arrays.
    """
    commands = []
    for item in instances:
        if item.get("hidden"):
            continue
        indices, vertices = item["indices"], item["vertices"]
        groups = item.get("groups") or [dict(start=0, count=len(indices), material=0)]
        cached = item.get("_draw_centers")
        if cached is None or cached[0] is not vertices or cached[1] is not indices or cached[2] is not groups:
            cached = (vertices, indices, groups, {})
            item["_draw_centers"] = cached
        centers = cached[3]
        for group_index, group in enumerate(groups):
            start, amount = group["start"], min(group["count"], len(indices)-group["start"])
            if start >= len(indices):
                continue
            if start < 0 or amount < 0 or start % 3 or amount % 3:
                raise ValueError("Invalid material triangle group supplied to OpenGL")
            image = item.get("material_images", {}).get(group["material"])
            state = material_state(item, group["material"], image)
            blended = (state.get("source_blend", 2), state.get("destination_blend", 1)) != (2, 1)
            distance = 0.
            if blended and amount:
                if group_index not in centers:
                    selected = indices[start:start+amount]
                    if any(index < 0 or index >= len(vertices) for index in selected):
                        raise ValueError("Invalid triangle indices supplied to OpenGL")
                    centers[group_index] = tuple(sum(vertices[index][axis] for index in selected) / amount for axis in range(3))
                distance = sum((a-b)**2 for a,b in zip(centers[group_index], eye))
            commands.append((blended, -distance, len(commands), item, group))
    commands.sort(key=lambda command: command[:3])
    return [(item, group) for _, _, _, item, group in commands]


class GLRenderer:
    """All construction, rendering and teardown must occur on the Tk thread.

    render() accepts already-transformed world vertices and optional world-space
    normals. UVs, triangle groups and material_images retain their native mesh
    indexing. Texture pixels are RGBA with cutout alpha; full shader blending,
    normal maps and native material passes are outside this backend's scope.
    """
    MAX_ARRAY_VERTICES = 1_000_000
    MAX_ARRAY_INDICES = 3_000_000
    TEXTURE_BUDGET = 64 * 1024 * 1024

    def __init__(self, canvas):
        if os.name != "nt":
            raise GLUnavailable("The embedded OpenGL viewport requires Windows")
        self.canvas = canvas
        self.hwnd = self.dc = self.context = None
        self.closed = False
        self.size = None
        self.arrays = OrderedDict()
        self.textures = OrderedDict()
        self.texture_bytes = 0
        self.warnings = []
        self.repaint_requested = False
        self.input_events = deque(maxlen=128)
        self.u = C.WinDLL("user32", use_last_error=True)
        self.gdi = C.WinDLL("gdi32", use_last_error=True)
        self.gl = C.WinDLL("opengl32", use_last_error=True)
        self.kernel = C.WinDLL("kernel32", use_last_error=True)
        self._configure_api()
        try:
            self._create_window()
            self._create_context()
            self.info = {key: self._string(value) for key, value in
                         (("vendor", 0x1f00), ("renderer", 0x1f01), ("version", 0x1f02))}
        except Exception:
            self.close()
            raise

    @staticmethod
    def _function(library, name, result, arguments):
        function = getattr(library, name)
        function.restype, function.argtypes = result, arguments
        return function

    def _configure_api(self):
        ptr, uint, sint, boolean = C.c_void_p, C.c_uint32, C.c_int32, C.c_int32
        longptr, ulongptr = C.c_ssize_t, C.c_size_t
        self.WNDPROC = C.WINFUNCTYPE(longptr, ptr, uint, ulongptr, longptr)
        class WindowClass(C.Structure):
            _fields_ = [("style", uint), ("lpfnWndProc", self.WNDPROC),
                        ("cbClsExtra", sint), ("cbWndExtra", sint),
                        ("hInstance", ptr), ("hIcon", ptr), ("hCursor", ptr),
                        ("hbrBackground", ptr), ("lpszMenuName", C.c_wchar_p),
                        ("lpszClassName", C.c_wchar_p)]
        self.WindowClass = WindowClass
        class PixelFormat(C.Structure):
            _fields_ = [("nSize", C.c_uint16), ("nVersion", C.c_uint16), ("dwFlags", uint),
                        *[(name, C.c_ubyte) for name in (
                            "iPixelType", "cColorBits", "cRedBits", "cRedShift", "cGreenBits", "cGreenShift",
                            "cBlueBits", "cBlueShift", "cAlphaBits", "cAlphaShift", "cAccumBits",
                            "cAccumRedBits", "cAccumGreenBits", "cAccumBlueBits", "cAccumAlphaBits",
                            "cDepthBits", "cStencilBits", "cAuxBuffers", "iLayerType", "bReserved")],
                        ("dwLayerMask", uint), ("dwVisibleMask", uint), ("dwDamageMask", uint)]
        self.PixelFormat = PixelFormat
        f = self._function
        f(self.kernel, "GetModuleHandleW", ptr, [C.c_wchar_p])
        f(self.u, "RegisterClassW", C.c_uint16, [C.POINTER(WindowClass)])
        f(self.u, "UnregisterClassW", boolean, [C.c_wchar_p, ptr])
        f(self.u, "CreateWindowExW", ptr, [uint, C.c_wchar_p, C.c_wchar_p, uint,
                                         sint, sint, sint, sint, ptr, ptr, ptr, ptr])
        f(self.u, "DefWindowProcW", longptr, [ptr, uint, ulongptr, longptr])
        f(self.u, "DestroyWindow", boolean, [ptr])
        f(self.u, "GetDC", ptr, [ptr])
        f(self.u, "ReleaseDC", sint, [ptr, ptr])
        f(self.u, "MoveWindow", boolean, [ptr, sint, sint, sint, sint, boolean])
        f(self.u, "ValidateRect", boolean, [ptr, ptr])
        f(self.u, "SetCapture", ptr, [ptr])
        f(self.u, "ReleaseCapture", boolean, [])
        f(self.u, "SetFocus", ptr, [ptr])
        f(self.u, "GetKeyState", C.c_int16, [sint])
        class Point(C.Structure):
            _fields_ = [("x", sint), ("y", sint)]
        self.Point = Point
        f(self.u, "ScreenToClient", boolean, [ptr, C.POINTER(Point)])
        f(self.gdi, "ChoosePixelFormat", sint, [ptr, C.POINTER(PixelFormat)])
        f(self.gdi, "SetPixelFormat", boolean, [ptr, sint, C.POINTER(PixelFormat)])
        f(self.gdi, "SwapBuffers", boolean, [ptr])
        f(self.gl, "wglCreateContext", ptr, [ptr])
        f(self.gl, "wglDeleteContext", boolean, [ptr])
        f(self.gl, "wglMakeCurrent", boolean, [ptr, ptr])
        specs = {
            "glGetString": (C.c_char_p, [uint]), "glGetError": (uint, []),
            "glGetIntegerv": (None, [uint, C.POINTER(sint)]),
            "glViewport": (None, [sint, sint, sint, sint]),
            "glClearColor": (None, [C.c_float] * 4), "glClear": (None, [uint]),
            "glEnable": (None, [uint]), "glDisable": (None, [uint]),
            "glDepthFunc": (None, [uint]), "glDepthMask": (None, [C.c_ubyte]),
            "glMatrixMode": (None, [uint]), "glLoadIdentity": (None, []),
            "glLoadMatrixd": (None, [C.POINTER(C.c_double)]),
            "glFrustum": (None, [C.c_double] * 6),
            "glClipPlane": (None, [uint, C.POINTER(C.c_double)]),
            "glEnableClientState": (None, [uint]), "glDisableClientState": (None, [uint]),
            "glVertexPointer": (None, [sint, uint, sint, ptr]),
            "glNormalPointer": (None, [uint, sint, ptr]),
            "glColorPointer": (None, [sint, uint, sint, ptr]),
            "glTexCoordPointer": (None, [sint, uint, sint, ptr]),
            "glDrawElements": (None, [uint, sint, uint, ptr]),
            "glBegin": (None, [uint]), "glEnd": (None, []),
            "glVertex3f": (None, [C.c_float] * 3), "glLineWidth": (None, [C.c_float]),
            "glColor4f": (None, [C.c_float] * 4),
            "glLightfv": (None, [uint, uint, C.POINTER(C.c_float)]),
            "glLightModelfv": (None, [uint, C.POINTER(C.c_float)]),
            "glLightModeli": (None, [uint, sint]),
            "glColorMaterial": (None, [uint, uint]),
            "glGenTextures": (None, [sint, C.POINTER(uint)]),
            "glDeleteTextures": (None, [sint, C.POINTER(uint)]),
            "glBindTexture": (None, [uint, uint]),
            "glTexParameteri": (None, [uint, uint, sint]),
            "glPixelStorei": (None, [uint, sint]),
            "glTexImage2D": (None, [uint, sint, sint, sint, sint, sint, uint, uint, ptr]),
            "glTexEnvi": (None, [uint, uint, sint]),
            "glAlphaFunc": (None, [uint, C.c_float]),
            "glBlendFunc": (None, [uint, uint]),
            "glHint": (None, [uint, uint]),
        }
        for name, (result, arguments) in specs.items():
            f(self.gl, name, result, arguments)

    def _create_window(self):
        self.instance = self.kernel.GetModuleHandleW(None)
        self.class_name = f"AC2ContentPreview_{os.getpid()}_{id(self):x}"
        def procedure(hwnd, message, wparam, lparam):
            if message == 0x84:  # HTCLIENT; explicitly bridge input below.
                return 1
            if self._queue_input(hwnd, message, wparam, lparam):
                return 0
            if message == 0x14:  # WM_ERASEBKGND: the GL color buffer paints it.
                return 1
            if message == 0x0f:  # WM_PAINT
                self.u.ValidateRect(hwnd, None)
                if not self.closed:
                    # Never enter Tk from a ctypes/native window callback.
                    # Nested Tk calls overwrite _tkinter's saved Tcl thread
                    # state and can fatally abort Python on the next idle event.
                    # The owning Tk event loop consumes this flag instead.
                    self.repaint_requested = True
                return 0
            return self.u.DefWindowProcW(hwnd, message, wparam, lparam)
        self.procedure = self.WNDPROC(procedure)
        window_class = self.WindowClass()
        window_class.style = 0x0023  # CS_OWNDC | CS_HREDRAW | CS_VREDRAW
        window_class.lpfnWndProc = self.procedure
        window_class.hInstance = self.instance
        window_class.lpszClassName = self.class_name
        if not self.u.RegisterClassW(C.byref(window_class)):
            raise GLUnavailable(f"Could not register the preview window: {C.get_last_error()}")
        self.registered = True
        parent = self.canvas.winfo_id()
        self.hwnd = self.u.CreateWindowExW(0, self.class_name, "", 0x56000000,
                                          0, 0, max(1, self.canvas.winfo_width()),
                                          max(1, self.canvas.winfo_height()), parent, None,
                                          self.instance, None)
        if not self.hwnd:
            raise GLUnavailable(f"Could not create the preview window: {C.get_last_error()}")
        self.dc = self.u.GetDC(self.hwnd)
        if not self.dc:
            raise GLUnavailable("Could not obtain the preview device context")

    @staticmethod
    def signed_word(value):
        return (value & 0xffff) - (0x10000 if value & 0x8000 else 0)

    def _queue_input(self, hwnd, message, wparam, lparam):
        """Record native input without entering Tk from the WNDPROC callback.

        HTTRANSPARENT depends on native hit-test traversal and did not provide a
        reliable Tk canvas event stream on every Windows installation. Mouse
        capture keeps drag/release events paired even outside the viewport.
        """
        sequences = {0x201: "<ButtonPress-1>", 0x202: "<ButtonRelease-1>",
                     0x204: "<ButtonPress-3>", 0x205: "<ButtonRelease-3>",
                     0x200: "<Motion>", 0x20a: "<MouseWheel>"}
        if message in sequences:
            x, y = self.signed_word(lparam), self.signed_word(lparam >> 16)
            state = (1 if wparam & 4 else 0) | (4 if wparam & 8 else 0)
            state |= (0x100 if wparam & 1 else 0) | (0x400 if wparam & 2 else 0)
            if self.u.GetKeyState(0x12) < 0:
                state |= 8
            delta = 0
            if message == 0x20a:
                point = self.Point(x, y)
                self.u.ScreenToClient(hwnd, C.byref(point))
                x, y = point.x, point.y
                delta = self.signed_word(wparam >> 16)
            elif message in (0x201, 0x204):
                self.u.SetFocus(hwnd)
                self.u.SetCapture(hwnd)
            elif message in (0x202, 0x205):
                self.u.ReleaseCapture()
            event = (sequences[message], x, y, state, delta, None)
            if message == 0x200 and self.input_events and self.input_events[-1][0] == "<Motion>":
                self.input_events[-1] = event
            else:
                self.input_events.append(event)
            return True
        if message in (0x100, 0x104, 0x101, 0x105):  # key down/up, including native focus
            keys = {0x25: "Left", 0x26: "Up", 0x27: "Right", 0x28: "Down",
                    0x21: "Prior", 0x22: "Next", 0x24: "Home", 0x1b: "Escape", 0x2e: "Delete",
                    0x09: "Tab", 0x0d: "Return", 0x20: "space"}
            key = keys.get(wparam)
            if 0x30 <= wparam <= 0x5a:
                key = chr(wparam).lower()
            if key:
                state = sum(mask for vk, mask in ((0x10, 1), (0x11, 4), (0x12, 8))
                            if self.u.GetKeyState(vk) < 0)
                self.input_events.append(("<KeyPress>" if message in (0x100, 0x104) else "<KeyRelease>", 0, 0, state, 0, key))
                return True
        if message == 0x8:  # WM_KILLFOCUS: never leave movement latched.
            self.input_events.append(("<FocusOut>", 0, 0, 0, 0, None))
        if message == 0x215:  # WM_CAPTURECHANGED: end an interrupted drag.
            self.input_events.append(("<ButtonRelease-1>", 0, 0, 0, 0, None))
            self.input_events.append(("<ButtonRelease-3>", 0, 0, 0, 0, None))
        return False

    def consume_input(self):
        events = tuple(self.input_events)
        self.input_events.clear()
        return events

    def _create_context(self):
        descriptor = self.PixelFormat()
        descriptor.nSize, descriptor.nVersion = C.sizeof(descriptor), 1
        descriptor.dwFlags = 0x25  # DRAW_TO_WINDOW | SUPPORT_OPENGL | DOUBLEBUFFER
        descriptor.cColorBits, descriptor.cAlphaBits = 32, 8
        descriptor.cDepthBits, descriptor.cStencilBits = 24, 8
        index = self.gdi.ChoosePixelFormat(self.dc, C.byref(descriptor))
        if not index or not self.gdi.SetPixelFormat(self.dc, index, C.byref(descriptor)):
            raise GLUnavailable(f"No supported OpenGL pixel format: {C.get_last_error()}")
        self.context = self.gl.wglCreateContext(self.dc)
        self._current()
        limit = C.c_int32()
        self.gl.glGetIntegerv(0x0d33, C.byref(limit))  # GL_MAX_TEXTURE_SIZE
        self.max_texture_size = max(0, limit.value)
        self.gl.glClearColor(0.09, 0.13, 0.17, 1)
        self.gl.glEnable(0x0b71)  # DEPTH_TEST
        self.gl.glDepthFunc(0x0203)  # LEQUAL
        self.gl.glEnable(0x0bc0)  # ALPHA_TEST: retain texture cutouts.
        self.gl.glAlphaFunc(0x0204, 0.05)  # GREATER, editor cutoff.
        self.gl.glDisable(0x0be2)  # No guessed native translucency/material pass.
        self.gl.glEnable(0x0ba1)  # NORMALIZE, for supplied transformed normals.
        self.gl.glEnable(0x0b57)  # COLOR_MATERIAL
        self.gl.glColorMaterial(0x0408, 0x1602)  # FRONT_AND_BACK, AMBIENT_AND_DIFFUSE
        self.gl.glLightModelfv(0x0b53, (C.c_float * 4)(0.42, 0.42, 0.42, 1))
        self.gl.glLightModeli(0x0b52, 1)  # TWO_SIDE: interior mesh winding varies.
        self.gl.glLightfv(0x4000, 0x1201, (C.c_float * 4)(0.72, 0.72, 0.72, 1))
        self.gl.glEnable(0x4000)  # LIGHT0
        self.gl.glHint(0x0c50, 0x1102)  # PERSPECTIVE_CORRECTION_HINT, NICEST

    def _current(self):
        if not self.context or not self.gl.wglMakeCurrent(self.dc, self.context):
            raise GLUnavailable(f"Could not activate the OpenGL context: {C.get_last_error()}")

    def consume_repaint(self):
        pending = self.repaint_requested
        self.repaint_requested = False
        return pending

    def _string(self, token):
        value = self.gl.glGetString(token)
        return value.decode("ascii", "replace") if value else "unavailable"

    def _array(self, values, width, kind=C.c_float):
        if width and any(len(row) != width for row in values):
            raise ValueError("Invalid vertex attribute width supplied to OpenGL")
        flat = [component for row in values for component in row] if width else values
        if any(not math.isfinite(component) for component in flat):
            raise ValueError("Non-finite geometry supplied to OpenGL")
        return (kind * len(flat))(*flat)

    def _buffers(self, item):
        vertices, indices = item["vertices"], item["indices"]
        normals, uvs = item.get("normals", ()), item.get("uvs", ())
        colors = item.get("colors", ())
        if len(vertices) > self.MAX_ARRAY_VERTICES or len(indices) > self.MAX_ARRAY_INDICES:
            raise ValueError("Preview geometry exceeds the GPU upload budget")
        key = (id(vertices), id(indices), id(normals), id(uvs), id(colors))
        # Retaining source references in the entry prevents ID reuse. Static
        # geometry therefore needs neither repeated index scans nor re-uploads.
        if key in self.arrays:
            self.arrays.move_to_end(key)
            return key, self.arrays[key]
        if len(indices) % 3 or any(not isinstance(index, int) or index < 0 or index >= len(vertices) for index in indices):
            raise ValueError("Invalid triangle indices supplied to OpenGL")
        packed_colors = None
        if colors:
            if len(colors) != len(vertices) or any(not isinstance(value, int) or not 0 <= value <= 0xffffffff for value in colors):
                raise ValueError("Invalid packed native vertex colors supplied to OpenGL")
            rgba = bytearray(len(colors) * 4)
            for index, color in enumerate(colors):
                # Native packed value is ARGB; GL_UNSIGNED_BYTE consumes RGBA.
                rgba[index*4:index*4+4] = bytes(((color >> 16) & 255, (color >> 8) & 255,
                                               color & 255, (color >> 24) & 255))
            packed_colors = (C.c_ubyte * len(rgba)).from_buffer_copy(rgba)
        self.arrays[key] = dict(refs=(vertices, indices, normals, uvs, colors),
            vertices=self._array(vertices, 3),
            indices=(C.c_uint32 * len(indices))(*indices),
            normals=self._array(normals, 3) if len(normals) == len(vertices) else None,
            uvs=self._array(uvs, 2) if len(uvs) == len(vertices) else None,
            colors=packed_colors)
        self.arrays.move_to_end(key)
        return key, self.arrays[key]

    def _texture(self, image):
        key = id(image)
        if key in self.textures:
            self.textures.move_to_end(key)
            return key, self.textures[key][1]
        width, height, rgb = image["width"], image["height"], image["rgb"]
        if not 0 < width <= self.max_texture_size or not 0 < height <= self.max_texture_size:
            raise ValueError("Texture exceeds the active OpenGL dimension limit")
        pixels = width * height
        if pixels > 4_194_304 or len(rgb) != pixels * 3:
            raise ValueError("Texture dimensions or RGB payload exceed preview bounds")
        state = image.get("render_state", {})
        alpha = image.get("alpha", b"\xff" * pixels) if state.get("texture_alpha",False) else b"\xff" * pixels
        if len(alpha) != pixels:
            raise ValueError("Texture alpha payload does not match its dimensions")
        rgba = bytearray(pixels * 4)
        rgba[0::4], rgba[1::4], rgba[2::4], rgba[3::4] = rgb[0::3], rgb[1::3], rgb[2::3], alpha
        identifier = C.c_uint32()
        self.gl.glGenTextures(1, C.byref(identifier))
        if not identifier.value:
            raise GLUnavailable("OpenGL could not allocate a texture object")
        try:
            self.gl.glBindTexture(0x0de1, identifier.value)
            self.gl.glPixelStorei(0x0cf5, 1)  # UNPACK_ALIGNMENT
            for parameter in (0x2800, 0x2801):  # MAG/MIN filters
                self.gl.glTexParameteri(0x0de1, parameter, 0x2601)  # LINEAR
            for parameter in (0x2802, 0x2803):
                self.gl.glTexParameteri(0x0de1, parameter, 0x2901)  # REPEAT
            raw = (C.c_ubyte * len(rgba)).from_buffer(rgba)
            self.gl.glTexImage2D(0x0de1, 0, 0x1908, width, height, 0, 0x1908, 0x1401, raw)
            error = self.gl.glGetError()
            if error:
                raise GLUnavailable(f"OpenGL texture upload failed ({error:#x})")
        except Exception:
            self.gl.glDeleteTextures(1, C.byref(identifier))
            raise
        self.textures[key] = (image, identifier.value, len(rgba))
        self.texture_bytes += len(rgba)
        return key, identifier.value

    def render(self, instances, eye, forward, right, up, width, height, cutaway_height=None, gizmo=None, brush=None, doors=None, portals=None):
        if self.closed:
            return 0
        if width < 1 or height < 1:
            return 0
        self._current()
        if self.size != (width, height):
            self.u.MoveWindow(self.hwnd, 0, 0, width, height, False)
            self.size = width, height
        gl = self.gl
        gl.glViewport(0, 0, width, height)
        gl.glClear(0x00004000 | 0x00000100)
        near, far = 0.15, 10000.0
        focal = min(width, height) * 0.92
        gl.glMatrixMode(0x1701)  # PROJECTION
        gl.glLoadIdentity()
        gl.glFrustum(-width*near/(2*focal), width*near/(2*focal),
                     -height*near/(2*focal), height*near/(2*focal), near, far)
        dot = lambda a,b: sum(x*y for x,y in zip(a,b))
        view = (C.c_double * 16)(right[0], up[0], -forward[0], 0,
                                right[1], up[1], -forward[1], 0,
                                right[2], up[2], -forward[2], 0,
                                -dot(right,eye), -dot(up,eye), dot(forward,eye), 1)
        gl.glMatrixMode(0x1700)  # MODELVIEW
        gl.glLoadMatrixd(view)
        gl.glLightfv(0x4000, 0x1203, (C.c_float * 4)(0.25, -0.4, 0.85, 0))
        if cutaway_height is not None:
            gl.glClipPlane(0x3000, (C.c_double * 4)(0, 0, -1, cutaway_height))
        used_arrays, used_textures = set(), set()
        count = 0
        self.warnings = []
        gl.glEnableClientState(0x8074)  # VERTEX_ARRAY
        for item, group in material_draw_commands(instances, eye):
            key, buffers = self._buffers(item)
            used_arrays.add(key)
            gl.glVertexPointer(3, 0x1406, 0, buffers["vertices"])
            if buffers["colors"] is not None:
                gl.glEnableClientState(0x8076)  # COLOR_ARRAY
                gl.glColorPointer(4, 0x1401, 0, buffers["colors"])
            else:
                gl.glDisableClientState(0x8076)
            if buffers["normals"] is not None:
                gl.glEnableClientState(0x8075)
                gl.glNormalPointer(0x1406, 0, buffers["normals"])
                gl.glEnable(0x0b50)  # LIGHTING
            else:
                gl.glDisableClientState(0x8075)
                gl.glDisable(0x0b50)
            if buffers["uvs"] is not None:
                gl.glEnableClientState(0x8078)
                gl.glTexCoordPointer(2, 0x1406, 0, buffers["uvs"])
            else:
                gl.glDisableClientState(0x8078)
            if cutaway_height is not None and item.get("role") == "world":
                gl.glEnable(0x3000)
            else:
                gl.glDisable(0x3000)
            indices = item["indices"]
            images = item.get("material_images", {})
            start, amount = group["start"], min(group["count"], len(indices)-group["start"])
            image = images.get(group["material"])
            state = material_state(item, group["material"], image)
            diffuse = material_rgba(item, group["material"])
            textured = False
            if image is not None and buffers["uvs"] is not None:
                try:
                    texture_key, texture = self._texture(image)
                    used_textures.add(texture_key)
                    gl.glEnable(0x0de1)
                    gl.glBindTexture(0x0de1, texture)
                    gl.glTexEnvi(0x2300, 0x2200, 0x2100)  # TEXTURE_ENV_MODE, MODULATE
                    # Native SELECTARG texture alpha does not multiply the
                    # diffuse alpha; MODULATE sources include both.
                    alpha = diffuse[3] if diffuse_alpha_used(state) else 1.
                    tint = diffuse[:3] if group["material"] in item.get("material_diffuse", {}) else (1., 1., 1.)
                    gl.glColor4f(*tint, alpha)
                    textured = True
                except (KeyError, ValueError, GLUnavailable) as error:
                    if len(self.warnings) < 16:
                        self.warnings.append(str(error))
            if not textured:
                gl.glDisable(0x0de1)
                gl.glColor4f(*diffuse)
            if state.get("alpha_test"):
                gl.glEnable(0x0bc0)
                gl.glAlphaFunc(0x0204,state["alpha_reference"])
            else:
                gl.glDisable(0x0bc0)
            blends = {1:0,2:1,3:0x0300,4:0x0301,5:0x0302,6:0x0303,
                      7:0x0304,8:0x0305,9:0x0306,10:0x0307,11:0x0308}
            source,destination = state.get("source_blend",2),state.get("destination_blend",1)
            if (source,destination) != (2,1):
                gl.glEnable(0x0be2)
                gl.glBlendFunc(blends[source],blends[destination])
            else:
                gl.glDisable(0x0be2)
            gl.glDepthMask(bool(state.get("depth_write",True)))
            pointer = C.cast(C.byref(buffers["indices"], start * C.sizeof(C.c_uint32)), C.c_void_p)
            gl.glDrawElements(0x0004, amount, 0x1405, pointer)  # TRIANGLES, UNSIGNED_INT
            count += amount // 3
        gl.glDisable(0x3000)
        gl.glDepthMask(True)
        gl.glDisable(0x0be2)
        gl.glDisableClientState(0x8074)
        gl.glDisableClientState(0x8075)
        gl.glDisableClientState(0x8076)
        gl.glDisableClientState(0x8078)
        gl.glDisable(0x0de1)
        self._selection_overlay(instances, gizmo)
        if portals:
            gl.glDisable(0x0b50);gl.glDisable(0x0de1);gl.glLineWidth(3)
            for portal in portals:
                gl.glColor4f(.2,.85,1,1)
                center=portal["position"]
                gl.glBegin(0x0002)
                for index in range(48):
                    angle=index*2*math.pi/48
                    gl.glVertex3f(center[0]+math.cos(angle)*3,center[1],center[2]+5+math.sin(angle)*5)
                gl.glEnd()
                gl.glBegin(0x0001)
                gl.glVertex3f(center[0],center[1],center[2]+1)
                gl.glVertex3f(center[0],center[1],center[2]+8)
                gl.glEnd()
            gl.glLineWidth(1)
        if doors:
            gl.glDisable(0x0b50);gl.glDisable(0x0b71);gl.glDisable(0x0de1)
            for door in doors:
                gl.glColor4f(*( (1,.9,.1,1) if door.get("selected") else (1,.45,.1,1) if door["open"] else (.2,.9,.5,1)))
                gl.glLineWidth(4 if door.get("selected") else 2)
                gl.glBegin(0x0002)
                for point in door["points"]:gl.glVertex3f(*point)
                gl.glEnd()
                center=door["center"]
                gl.glBegin(0x0001)
                for axis in range(3):
                    for sign in (-1,1):
                        point=list(center);point[axis]+=sign*.7;gl.glVertex3f(*point)
                gl.glEnd()
            gl.glLineWidth(1);gl.glEnable(0x0b71)
        if brush:
            gl.glDisable(0x0b50);gl.glDisable(0x0b71);gl.glDisable(0x0de1)
            gl.glColor4f(1,.7,.2,1);gl.glLineWidth(2);gl.glBegin(0x0002)
            center,radius=brush["center"],brush["radius"]
            for index in range(64):
                angle=index*2*math.pi/64
                gl.glVertex3f(center[0]+math.cos(angle)*radius,center[1]+math.sin(angle)*radius,center[2]+.1)
            gl.glEnd();gl.glLineWidth(1);gl.glEnable(0x0b71)
        error = gl.glGetError()
        if error:
            raise GLUnavailable(f"OpenGL preview draw failed ({error:#x})")
        if not self.gdi.SwapBuffers(self.dc):
            raise GLUnavailable(f"Preview buffer swap failed: {C.get_last_error()}")
        # Drop stale scene buffers immediately; texture reuse is bounded by bytes.
        self.arrays = OrderedDict((key,value) for key,value in self.arrays.items() if key in used_arrays)
        while self.texture_bytes > self.TEXTURE_BUDGET and self.textures:
            key, (_, identifier, size) = self.textures.popitem(last=False)
            self.gl.glDeleteTextures(1, C.byref(C.c_uint32(identifier)))
            self.texture_bytes -= size
        # Images from an old scene need no retained GPU memory.
        for key in list(self.textures):
            if key not in used_textures:
                _, identifier, size = self.textures.pop(key)
                self.gl.glDeleteTextures(1, C.byref(C.c_uint32(identifier)))
                self.texture_bytes -= size
        return count

    def _selection_overlay(self, instances, gizmo=None):
        """A visible editing outline is independent of missing asset materials."""
        selected = [item for item in instances if (item.get("role") == "preview" or item.get("selected")) and not item.get("hidden")]
        vertices = [v for item in selected for v in item.get("vertices", ())]
        if not vertices:
            return
        low = [min(v[axis] for v in vertices) for axis in range(3)]
        high = [max(v[axis] for v in vertices) for axis in range(3)]
        corners = [(high[0] if i & 1 else low[0], high[1] if i & 2 else low[1],
                    high[2] if i & 4 else low[2]) for i in range(8)]
        gl = self.gl
        gl.glDisable(0x0b50)  # LIGHTING
        gl.glDisable(0x0b71)  # Selection remains visible through occluding walls.
        gl.glDisable(0x0bc0)
        gl.glColor4f(1, .72, .22, 1)
        gl.glLineWidth(2)
        gl.glBegin(0x0001)  # LINES
        for i in range(8):
            for axis in (1, 2, 4):
                if not i & axis:
                    gl.glVertex3f(*corners[i])
                    gl.glVertex3f(*corners[i | axis])
        anchor = selected[0].get("position", (0, 0, 0))
        size = max(.25, min(2, max(high[i]-low[i] for i in range(3)) * .2))
        for axis in range(3):
            a, b = list(anchor), list(anchor)
            a[axis] -= size
            b[axis] += size
            gl.glVertex3f(*a)
            gl.glVertex3f(*b)
        gl.glEnd()
        gl.glLineWidth(1)
        gl.glEnable(0x0b71)
        if gizmo is not None:
            center = gizmo["center"]
            gl.glDisable(0x0b71)
            gl.glLineWidth(3)
            gl.glBegin(0x0001)
            for end, color in zip(gizmo["ends"], ((1,.25,.25),(.25,1,.4),(.3,.65,1))):
                gl.glColor4f(*color, 1)
                gl.glVertex3f(*center)
                gl.glVertex3f(*end)
            gl.glEnd()
            if gizmo["tool"] == "rotate":
                gl.glColor4f(1,.75,.25,1)
                gl.glBegin(0x0002)
                for index in range(64):
                    angle = index*2*math.pi/64
                    gl.glVertex3f(center[0]+math.cos(angle)*gizmo["length"], center[1]+math.sin(angle)*gizmo["length"], center[2])
                gl.glEnd()
            gl.glLineWidth(1)
            gl.glEnable(0x0b71)

    def close(self):
        if self.closed:
            return
        self.closed = True
        if self.context:
            if self.gl.wglMakeCurrent(self.dc, self.context):
                for _, identifier, _ in self.textures.values():
                    self.gl.glDeleteTextures(1, C.byref(C.c_uint32(identifier)))
            self.gl.wglMakeCurrent(None, None)
            self.gl.wglDeleteContext(self.context)
            self.context = None
        self.textures.clear()
        self.arrays.clear()
        if self.dc and self.hwnd:
            self.u.ReleaseDC(self.hwnd, self.dc)
            self.dc = None
        if self.hwnd:
            self.u.DestroyWindow(self.hwnd)
            self.hwnd = None
        if getattr(self, "registered", False):
            self.u.UnregisterClassW(self.class_name, self.instance)
            self.registered = False
