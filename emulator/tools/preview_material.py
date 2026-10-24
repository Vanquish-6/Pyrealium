"""Read-only AC2 editor material references and real DXT texture pixels.

Matching native layout evidence: MaterialInstance 005badf0, MaterialModifier
005b91b0, MaterialProperty 005daf10, RenderTexture 005bd850, RenderSurface
005b1c40, and categorized DBObj header 006d7830. This supports texture slot
3000 overrides; it does not evaluate complete material shaders or modifiers.
"""
import math
import struct


class MaterialFormatError(ValueError):
    pass


class Reader:
    def __init__(self, data):
        if len(data) > 64 * 1024 * 1024:
            raise MaterialFormatError("material or surface record exceeds 64 MiB")
        self.data = memoryview(data)
        self.pos = 0

    def take(self, count):
        if count < 0 or count > len(self.data) - self.pos:
            raise MaterialFormatError("truncated material or surface record")
        start = self.pos
        self.pos += count
        return self.data[start:self.pos]

    def u32(self):
        return struct.unpack("<I", self.take(4))[0]

    def end(self):
        if self.pos != len(self.data):
            raise MaterialFormatError("unexpected trailing material data")


def _properties(reader):
    count = reader.u32()
    if count > 256:
        raise MaterialFormatError("too many material properties")
    properties = []
    for _ in range(count):
        name, kind, length = reader.u32(), reader.u32(), reader.u32()
        if not 0 < length <= 65536:
            raise MaterialFormatError("invalid material property length")
        value = bytes(reader.take(length))
        fields = reader.u32()
        if fields > 256:
            raise MaterialFormatError("too many material property fields")
        # Two BIT32 values, field type, and texture-coordinate index (005daf10).
        bindings = [struct.unpack("<4I", reader.take(16)) for unused in range(fields)]
        properties.append(dict(name=name, kind=kind, value=value, fields=fields, bindings=bindings))
    return properties


def modifier_properties(data):
    reader = Reader(data)
    if reader.u32() >> 24 != 0x30:
        raise MaterialFormatError("not a native material modifier")
    properties = _properties(reader)
    reader.end()
    return properties


def base_properties(data, did):
    """Read RenderMaterial's embedded MaterialModifier before its shader layers.

    005b9e00 streams the DBObj DID then this->properties (005b91b0).
    Layer/stage bytecode is deliberately left opaque, not scanned for DIDs.
    """
    reader = Reader(data)
    if reader.u32() != did or did >> 24 != 0x2b:
        raise MaterialFormatError("invalid base render material")
    if reader.u32() != 0:
        raise MaterialFormatError("unsupported embedded modifier identity")
    return _properties(reader)


def base_stage_texture(data, did):
    """Bounded first-layer first-stage texture; shader tails remain opaque.

    RenderMaterial 005b9e00 -> MaterialLayer 005cc0b0 (flags/stage count)
    -> LayerStage 005be300 (first field is texture DID). This is the actual
    terrain diffuse source when no texture-slot property supplies an override.
    """
    reader = Reader(data)
    if reader.u32() != did or reader.u32() != 0:
        raise MaterialFormatError("unsupported base material prefix")
    _properties(reader)
    layers = reader.u32()
    if layers > 64:
        raise MaterialFormatError("base material layer capacity")
    if layers == 0:
        return None
    reader.u32()  # trueFlags
    reader.u32()  # falseFlags
    stages = reader.u32()
    if stages > 4:
        raise MaterialFormatError("base material stage capacity")
    if stages == 0:
        return None
    texture = reader.u32()
    return texture if texture >> 24 == 0x40 else None


def first_layer_state(data, did):
    """Known fixed-stage alpha/depth state; complete shader tails stay opaque."""
    reader = Reader(data)
    if reader.u32() != did or reader.u32() != 0:
        raise MaterialFormatError("unsupported base material prefix")
    _properties(reader)
    layers = reader.u32()
    if not 1 <= layers <= 64:
        raise MaterialFormatError("no supported base material layer")
    flags = (reader.u32(), reader.u32())
    count = reader.u32()
    if count > 4:
        raise MaterialFormatError("native stage capacity")
    stages = [struct.unpack("<14I",reader.take(56)) for unused in range(count)]
    modifier_count = reader.u32()
    if modifier_count > 64:
        raise MaterialFormatError("native layer modifier capacity")
    for unused in range(modifier_count):
        kind, coordinate = reader.u32(), reader.u32()
        # LM_UVTranslate/Scale::StreamPack005d68c0: type, coordinate,
        # two sizeof(Waveform)=44 payloads. UVRotate005d6990 has one.
        # UV motion is not evaluated here; it must not hide the alpha/depth
        # state that follows the bounded modifier array in MaterialLayer.
        if kind not in (4, 5, 6) or coordinate > 7:
            raise MaterialFormatError("unsupported native layer modifier")
        for wave in range(1 if kind == 5 else 2):
            waveform = reader.u32()
            values = struct.unpack("<10f", reader.take(40))
            if not 1 <= waveform <= 32 or not all(math.isfinite(value) for value in values):
                raise MaterialFormatError("invalid UV modifier waveform")
    source,destination,depth,write,cull,alpha_test = (reader.u32() for unused in range(6))
    if source not in range(1,12) or destination not in range(1,12) or depth not in range(1,9) or write not in (0,1) or alpha_test not in (0,1):
        raise MaterialFormatError("unsupported native layer render state")
    reference = None
    if alpha_test:
        # Waveform NONE is the constant first value (recovered WaveformType).
        if reader.u32() != 1:
            raise MaterialFormatError("animated alpha-test reference is unsupported")
        reference = struct.unpack("<f",reader.take(4))[0]
        if not 0 <= reference <= 1:
            raise MaterialFormatError("invalid alpha-test reference")
    uses_texture = False
    alpha_sources = {"diffuse"}
    for stage in stages:
        operation,first,second = stage[4:7]
        def depends(argument):
            kind = argument & 15
            return kind == 2 or kind == 1 and uses_texture
        if operation == 1:
            break
        if operation == 2:
            uses_texture = depends(first)
        elif operation == 3:
            uses_texture = depends(second)
        else:
            uses_texture = depends(first) or depends(second)
        def sources(argument):
            kind = argument & 15
            return ({"diffuse"} if kind == 0 else alpha_sources if kind == 1 else
                    {"texture"} if kind == 2 else {"unknown"})
        # Complement/alpha-replicate and shader operations beyond basic select
        # cannot establish that an alpha-zero diffuse is an invisible surface.
        if first & ~15 or second & ~15:
            alpha_sources = {"unknown"}
        elif operation == 2:
            alpha_sources = sources(first)
        elif operation == 3:
            alpha_sources = sources(second)
        elif operation == 4:  # Native TEXOP_MODULATE.
            alpha_sources = sources(first) | sources(second)
        else:
            alpha_sources = {"unknown"}
    return dict(source_blend=source,destination_blend=destination,depth_test=depth,
                depth_write=bool(write),alpha_test=bool(alpha_test),alpha_reference=reference,
                texture_alpha=uses_texture,diffuse_alpha_only=alpha_sources == {"diffuse"},
                alpha_sources=sorted(alpha_sources),uv_modifiers=modifier_count,
                layer_flags=flags,layers=layers)


def texture_levels(data):
    reader = Reader(data)
    did, source_type, count = reader.u32(), reader.u32(), reader.u32()
    if did >> 24 != 0x40 or source_type != 2 or not 0 < count <= 32:
        raise MaterialFormatError("unsupported native texture source or mip count")
    levels = [reader.u32() for _ in range(count)]
    if any(did >> 24 != 0x41 for did in levels):
        raise MaterialFormatError("invalid render-surface reference")
    reader.end()
    return levels


def _color565(value):
    return ((value >> 11) * 255 // 31, ((value >> 5) & 63) * 255 // 63, (value & 31) * 255 // 31)


def decode_texture(data):
    """Decode a native categorized RenderSurface41 to RGB/alpha/PPM bytes.

    Supports DXT1, DXT3 and DXT5 surfaces. It fails on unknown formats instead
    of displaying arbitrary bytes as texture data. Dimensions are bounded to
    4096 per axis and 16 million pixels; callers should prefer smaller mips.
    """
    reader = Reader(data)
    did, category = reader.u32(), reader.u32()
    width, height, pixel_format, image_size = (reader.u32() for _ in range(4))
    # IsCategorized is a property of the DBObj family, not the category value.
    # Native DBObj::UnPack_Type accepts zero as well as high-bit categories.
    if did >> 24 != 0x41:
        raise MaterialFormatError("not a categorized native render surface")
    if not 0 < width <= 4096 or not 0 < height <= 4096 or width * height > 16_000_000:
        raise MaterialFormatError("texture dimensions exceed editor bounds")
    formats = {0x31545844: ("DXT1", 8), 0x33545844: ("DXT3", 16), 0x35545844: ("DXT5", 16)}
    if pixel_format not in formats:
        raise MaterialFormatError(f"unsupported native pixel format {pixel_format:#x}")
    name, block_size = formats[pixel_format]
    blocks_x, blocks_y = (width + 3) // 4, (height + 3) // 4
    if image_size != blocks_x * blocks_y * block_size:
        raise MaterialFormatError("compressed texture payload length does not match dimensions")
    payload = reader.take(image_size)
    reader.end()
    rgb, alpha = bytearray(width * height * 3), bytearray(width * height)
    for by in range(blocks_y):
        for bx in range(blocks_x):
            start = (by * blocks_x + bx) * block_size
            color_start = start if name == "DXT1" else start + 8
            c0, c1, selectors = struct.unpack_from("<HHI", payload, color_start)
            colors = [_color565(c0), _color565(c1)]
            transparent = name == "DXT1" and c0 <= c1
            if transparent:
                colors.extend([tuple((a + b) // 2 for a, b in zip(colors[0], colors[1])), (0, 0, 0)])
            else:
                colors.extend([tuple((2 * a + b) // 3 for a, b in zip(colors[0], colors[1])),
                               tuple((a + 2 * b) // 3 for a, b in zip(colors[0], colors[1]))])
            if name == "DXT3":
                alpha_bits = int.from_bytes(payload[start:start + 8], "little")
            elif name == "DXT5":
                a0, a1 = payload[start], payload[start + 1]
                alpha_values = [a0, a1]
                if a0 > a1:
                    alpha_values.extend(((7 - i) * a0 + i * a1) // 7 for i in range(1, 7))
                else:
                    alpha_values.extend(((5 - i) * a0 + i * a1) // 5 for i in range(1, 5))
                    alpha_values.extend([0, 255])
                alpha_bits = int.from_bytes(payload[start + 2:start + 8], "little")
            for py in range(4):
                y = by * 4 + py
                if y >= height:
                    break
                for px in range(4):
                    x = bx * 4 + px
                    if x >= width:
                        break
                    pixel = py * 4 + px
                    selector = (selectors >> (pixel * 2)) & 3
                    opacity = 0 if transparent and selector == 3 else 255
                    if name == "DXT3":
                        opacity = ((alpha_bits >> (pixel * 4)) & 15) * 17
                    elif name == "DXT5":
                        opacity = alpha_values[(alpha_bits >> (pixel * 3)) & 7]
                    index = y * width + x
                    rgb[index * 3:index * 3 + 3] = bytes(colors[selector])
                    alpha[index] = opacity
    total_alpha = sum(alpha)
    if total_alpha:
        average = [sum(rgb[index * 3 + channel] * opacity for index, opacity in enumerate(alpha)) // total_alpha
                   for channel in range(3)]
    else:
        average = [0, 0, 0]
    raw = bytes(rgb)
    return dict(did=did, category=category, width=width, height=height, format=name,
                rgb=raw, alpha=bytes(alpha), average=average,
                ppm=f"P6\n{width} {height}\n255\n".encode("ascii") + raw)


def texture_image(portal, texture_did, surfaces=None, max_size=256):
    """Choose the first available native mip within max_size, without rescaling.

    Lower mips generally live in portal.dat. The largest mip may instead live
    in highres.dat; pass that opened Dat as surfaces when available.
    """
    if not 4 <= max_size <= 1024:
        raise MaterialFormatError("preview mip size must be 4..1024")
    levels = texture_levels(portal.read(texture_did))
    last = None
    for level in levels:
        data = None
        for archive in (portal, surfaces):
            if archive is None:
                continue
            try:
                data = archive.read(level)
                break
            except KeyError:
                pass
        if data is None:
            continue
        if len(data) < 24:
            raise MaterialFormatError("truncated render surface")
        width, height = struct.unpack_from("<II", data, 8)
        last = data
        if width <= max_size and height <= max_size:
            return decode_texture(data)
    if last is not None:
        raise MaterialFormatError("no available mip is within the selected preview size")
    raise MaterialFormatError("texture surface mips are missing from the supplied DAT archives")


def material_info(portal, did, surfaces=None, max_size=256, include_image=True):
    """Resolve supported material-instance overrides and their real base pixels.

    Base RenderMaterial properties precede instance modifiers. Complete shader
    layers, recoloring, multitexture blending and effects remain unevaluated.
    """
    if did >> 24 == 0x2b:
        base, material_type, modifiers = did, None, []
    else:
        reader = Reader(portal.read(did))
        record, base, material_type, count = (reader.u32() for _ in range(4))
        if record != did or did >> 24 != 0x31 or base >> 24 != 0x2B or count > 256:
            raise MaterialFormatError("unsupported material-instance profile")
        modifiers = [reader.u32() for _ in range(count)]
        reader.end()
    if any(modifier >> 24 != 0x30 for modifier in modifiers):
        raise MaterialFormatError("invalid native material-modifier reference")
    base_data = portal.read(base)
    textures, properties = {}, base_properties(base_data, base)
    for prop in properties:
        if prop["kind"] == 3000 and len(prop["value"]) == 4:
            texture = struct.unpack("<I", prop["value"])[0]
            if texture == 0 or texture >> 24 == 0x40:
                textures[prop["name"]] = texture
    for modifier in modifiers:
        values = modifier_properties(portal.read(modifier))
        properties.extend(values)
        for prop in values:
            if prop["kind"] == 3000 and len(prop["value"]) == 4:
                texture = struct.unpack("<I", prop["value"])[0]
                if texture == 0 or texture >> 24 == 0x40:
                    textures[prop["name"]] = texture
    result = dict(did=did, base=base, material_type=material_type,
                  modifiers=modifiers, textures=textures,
                  scope="Actual base properties plus ordered instance overrides; complete shader evaluation is not implemented")
    try:
        result["render_state"] = first_layer_state(base_data,base)
    except MaterialFormatError as error:
        result["render_state_error"] = str(error)
    result["color_properties"] = {
        prop["name"]: list(struct.unpack("<4f", prop["value"]))
        for prop in properties if prop["kind"] == 2000 and len(prop["value"]) == 16
    }
    # MaterialProperty::Apply (005da830) copies RGBA including alpha and applies
    # null texture overrides. Ignoring either exposes the base debug texture on
    # intentionally transparent dungeon helper surfaces (3100034f/30000519).
    diffuse = result["color_properties"].get(2000, [1., 1., 1., 1.])
    if not all(math.isfinite(value) for value in diffuse):
        raise MaterialFormatError("non-finite material diffuse color")
    result["diffuse"] = diffuse
    state = result.get("render_state", {})
    result["visible"] = not (diffuse[3] <= 0 and state.get("diffuse_alpha_only") and
                              (state.get("source_blend"), state.get("destination_blend")) == (5, 6))
    result["average"] = [round(max(0, min(1, value))*255) for value in diffuse[:3]]
    base_texture = textures.get(3000)
    if 3000 not in textures:
        base_texture = base_stage_texture(base_data, base)
        if base_texture:
            textures[3000] = base_texture
            result["texture_source"] = "base first-layer stage"
    if base_texture == 0:
        result["texture_source"] = "explicit null texture override"
    elif base_texture is None:
        result["image_error"] = "No supported base or instance texture-slot3000 property"
    elif include_image:
        try:
            image = texture_image(portal, base_texture, surfaces, max_size)
            image["render_state"] = result.get("render_state", {})
            result["average"] = image["average"]
            result["image"] = image
        except (KeyError, MaterialFormatError) as error:
            result["image_error"] = str(error)
    return result
