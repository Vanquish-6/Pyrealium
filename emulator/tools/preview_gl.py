"""Windows OpenGL viewport for native DAT preview geometry and UV textures.

Uses only ctypes and the Windows OpenGL 1.1 entry points. The child window owns
its device context and does not alter the pixel format of Tk's own windows.
Documentation: https://learn.microsoft.com/windows/win32/opengl/pixel-format-functions
https://learn.microsoft.com/windows/win32/opengl/glvertexpointer
This is an editor renderer, not an implementation of the original client shaders.
"""
from collections import OrderedDict
import ctypes as C
import math
import os


class GLUnavailable(RuntimeError):
    pass


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
            if message == 0x84:  # WM_NCHITTEST: Tk parent owns all input.
                return -1
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

    def render(self, instances, eye, forward, right, up, width, height, cutaway_height=None):
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
        for item in instances:
            if item.get("hidden"):
                continue
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
            groups = item.get("groups") or [dict(start=0, count=len(indices), material=0)]
            images = item.get("material_images", {})
            for group in groups:
                if group["start"] >= len(indices):
                    continue  # A bounded viewport may trim later material groups.
                start, amount = group["start"], min(group["count"], len(indices)-group["start"])
                if start < 0 or amount < 0 or start % 3 or amount % 3:
                    raise ValueError("Invalid material triangle group supplied to OpenGL")
                image = images.get(group["material"])
                textured = False
                if image is not None and buffers["uvs"] is not None:
                    try:
                        texture_key, texture = self._texture(image)
                        used_textures.add(texture_key)
                        gl.glEnable(0x0de1)
                        gl.glBindTexture(0x0de1, texture)
                        gl.glTexEnvi(0x2300, 0x2200, 0x2100)  # TEXTURE_ENV_MODE, MODULATE
                        gl.glColor4f(1, 1, 1, 1)
                        textured = True
                    except (KeyError, ValueError, GLUnavailable) as error:
                        if len(self.warnings) < 16:
                            self.warnings.append(str(error))
                if not textured:
                    gl.glDisable(0x0de1)
                    color = item.get("material_colors", {}).get(group["material"], (150, 164, 174))
                    gl.glColor4f(*(float(channel)/255 for channel in color[:3]), 1)
                state = image.get("render_state",{}) if image is not None else {}
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
