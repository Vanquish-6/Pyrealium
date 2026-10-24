"""Read-only AC2 editor material references and real DXT texture pixels.

Matching native layout evidence: MaterialInstance 005badf0, MaterialModifier
005b91b0, MaterialProperty 005daf10, RenderTexture 005bd850, RenderSurface
005b1c40, and categorized DBObj header 006d7830. This supports texture slot
3000 overrides; it does not evaluate complete material shaders or modifiers.
"""
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


def modifier_properties(data):
    reader = Reader(data)
    if reader.u32() >> 24 != 0x30:
        raise MaterialFormatError("not a native material modifier")
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
        reader.take(fields * 16)
        properties.append(dict(name=name, kind=kind, value=value, fields=fields))
    reader.end()
    return properties


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
    if did >> 24 != 0x41 or category & 0x80000000 == 0:
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

    Material shaders, inheritance/defaults from RenderMaterial2b, recoloring,
    multitexture blending and effect modifiers remain unevaluated. A missing
    slot3000 override is reported, never replaced by a guessed texture.
    """
    reader = Reader(portal.read(did))
    record, base, material_type, count = (reader.u32() for _ in range(4))
    if record != did or did >> 24 != 0x31 or base >> 24 != 0x2B or count > 256:
        raise MaterialFormatError("unsupported material-instance profile")
    modifiers = [reader.u32() for _ in range(count)]
    reader.end()
    if any(modifier >> 24 != 0x30 for modifier in modifiers):
        raise MaterialFormatError("invalid native material-modifier reference")
    textures, properties = {}, []
    for modifier in modifiers:
        values = modifier_properties(portal.read(modifier))
        properties.extend(values)
        for prop in values:
            if prop["kind"] == 3000 and len(prop["value"]) == 4:
                texture = struct.unpack("<I", prop["value"])[0]
                if texture >> 24 == 0x40:
                    textures[prop["name"]] = texture
    result = dict(did=did, base=base, material_type=material_type,
                  modifiers=modifiers, textures=textures,
                  scope="Actual texture slot3000 override; complete native shader/material evaluation is not implemented")
    result["color_properties"] = {
        prop["name"]: list(struct.unpack("<4f", prop["value"]))
        for prop in properties if prop["kind"] == 2000 and len(prop["value"]) == 16
    }
    base_texture = textures.get(3000)
    if base_texture is None:
        result["image_error"] = "No explicit texture-slot3000 override; base material defaults are not interpreted"
    elif include_image:
        try:
            image = texture_image(portal, base_texture, surfaces, max_size)
            result["average"] = image["average"]
            result["image"] = image
        except (KeyError, MaterialFormatError) as error:
            result["image_error"] = str(error)
    return result
