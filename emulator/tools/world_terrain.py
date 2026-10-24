"""Native AC2 terrain authoring with byte-preserving, seam-coherent edits.

Layout evidence: CLandBlockData::StreamPack 0048f310 and
CLandBlockMesh::ConstructVertices 0048ca50 in the recovered build-7148 export.
The editable height is a byte index into Land_Height_Table, not a float. Terrain
type occupies the low LandscapeDefs terrain_bits of each packed cell-info word.
Brush behavior is editor policy; the serialized payload is native AC2 data.
"""
import math
import struct

from preview_terrain import (Reader, region_references, landscape_defs,
                             terrain_types, decode_terrain, apply_surfaces,
                             surface_types)


MAX_RECORD_BYTES = 4*1024*1024
MAX_STROKE_VERTICES = 16384
MAX_STROKE_BLOCKS = 81


def decode_terrain_name(raw):
    """PStringBase<char>::UnPack 00406cb0, then 00401fc0/00401f90.

    Complete uint32 groups are XORed with the native repeating key, followed
    by a nibble swap of every character, including trailing non-word bytes.
    """
    keys = (0xe5849238, 0xeb2d767d, 0xfc0fdd03, 0xcb45a6a6,
            0x7f21e6bb, 0xf5a6cdc8, 0x22d9ad12, 0xb2a3d4de,
            0x3b834db7, 0xc406c91c, 0x8392305e, 0x0a9f0099,
            0x0943b90c, 0xa66d953c, 0x0d929e1f, 0x0e2e9ad4)
    value = bytearray(raw)
    for offset in range(0, len(value)-3, 4):
        word = struct.unpack_from("<I", value, offset)[0] ^ keys[(offset//4)&15]
        struct.pack_into("<I", value, offset, word)
    value = bytes(((b>>4)|(b<<4))&255 for b in value)
    try:
        return value.decode("utf-8").rstrip("\0")
    except UnicodeDecodeError:
        return value.decode("cp1252", errors="replace").rstrip("\0")


def decode_block(data, definitions, identity=None):
    """Decode only the native editable arrays and retain every other byte."""
    side = definitions["vertex_side"]
    count = side*side
    if not 16 <= len(data) <= MAX_RECORD_BYTES:
        raise ValueError("Terrain record size is outside authoring bounds")
    did, lbi, pathmap, stored = struct.unpack_from("<4I", data)
    if did & 65535 != 65535 or stored != count or count > 16641 or (identity is not None and did != identity):
        raise ValueError("Terrain record identity or height dimensions do not match")
    info_offset = 20+count
    expected = (info_offset+count*4+3)&~3
    if len(data) != expected or struct.unpack_from("<I", data, 16+count)[0] != count:
        raise ValueError("Terrain record has truncated arrays or unsupported trailing data")
    return dict(did=did, original=bytes(data), heights=bytearray(data[16:16+count]),
                infos=list(struct.unpack_from(f"<{count}I", data, info_offset)),
                info_offset=info_offset)


def encode_block(block, definitions):
    """Preserve references, counts, padding, and every unrelated packed bit."""
    count = definitions["vertex_side"]**2
    if len(block["heights"]) != count or len(block["infos"]) != count:
        raise ValueError("Terrain dimensions changed during authoring")
    result = bytearray(block["original"])
    result[16:16+count] = bytes(block["heights"])
    struct.pack_into(f"<{count}I", result, block["info_offset"], *block["infos"])
    # Re-read to assert this is still the exact supported native profile.
    decode_block(result, definitions, block["did"])
    return bytes(result)


def _terrain_labels(data):
    reader = Reader(data)
    reader.u32()
    version, has_default = reader.u32(), reader.u32()
    if version != 0 or has_default not in (0, 1):
        raise ValueError("Unsupported terrain description")
    labels = {}
    def record():
        size = reader.take("H")
        if size == 65535:
            size = reader.u32()
        if size > 65536 or reader.pos+size > len(data):
            raise ValueError("Terrain label exceeds record bounds")
        label = decode_terrain_name(data[reader.pos:reader.pos+size])
        reader.pos += size
        reader.align()
        surface, index, edge, default = reader.take("IIfI")
        if default not in (0, 1):
            raise ValueError("Unsupported terrain default flag")
        if default:
            reader.take("II")
        for _ in range(reader.count()):
            reader.take("II")
        labels[index] = label
    if has_default:
        record()
    for _ in range(reader.count()):
        record()
    reader.align()
    reader.end()
    return labels


class TerrainEditor:
    """Read-only archives plus an optional live project-record overlay.

    ``stroke`` returns ``{did: (before_bytes, after_bytes)}`` and never changes
    either archive or the overlay. Apply the returned bytes as one project undo
    transaction. Each global border vertex is changed in every existing owner
    block, including diagonal neighbors at a corner.
    """
    def __init__(self, cells, portal, patches=None):
        self.cells, self.portal = cells, portal
        self.patches = patches if patches is not None else {}
        self.references = region_references(portal.read(0x0f000000))
        self.definitions = landscape_defs(portal.read(self.references["land_defs"]))
        if len(self.references["terrain"]) != 1 or len(self.references["surface"]) != 1:
            raise ValueError("Authoring requires the supported single-region terrain profile")
        data = portal.read(self.references["terrain"][0][0])
        self.types, self.default_type = terrain_types(data)
        self.labels = _terrain_labels(data)
        self.available = set(cells.index())
        self.height_candidates = tuple(enumerate(self.definitions["heights"]))

    def palette(self):
        return [dict(id=index, label=self.labels.get(index) or f"Terrain {index}", surface=value["surface"])
                for index, value in sorted(self.types.items())]

    def record(self, did):
        if did in self.patches:
            data = self.patches[did]
        else:
            entry = self.cells.index().get(did)
            if entry is None:
                raise ValueError(f"Terrain block {did:08x} is missing")
            if not 16 <= entry[1] <= MAX_RECORD_BYTES:
                raise ValueError("Terrain record exceeds authoring bounds")
            data = self.cells.read(did)
        if not isinstance(data, (bytes, bytearray)) or len(data) > MAX_RECORD_BYTES:
            raise ValueError("Invalid terrain project payload")
        return bytes(data)

    def mesh(self, did):
        surfaces = surface_types(self.portal.read(self.references["surface"][0][0]))
        return apply_surfaces(decode_terrain(self.record(did), self.definitions), self.definitions,
                              self.types, self.default_type, surfaces)

    def _owners(self, gx, gy):
        edge = self.definitions["vertex_side"]-1
        def axis(value):
            block, local = divmod(value, edge)
            values = [(block, local)]
            if local == 0:
                values.append((block-1, edge))
            return [(b, v) for b, v in values if 0 <= b < 255]
        return sorted((bx<<24 | by<<16 | 65535, x*self.definitions["vertex_side"]+y)
                      for bx, x in axis(gx) for by, y in axis(gy)
                      if (bx<<24 | by<<16 | 65535) in self.available or (bx<<24 | by<<16 | 65535) in self.patches)

    def stroke(self, cell, point, radius, strength, mode, target_height=None, terrain_type=None):
        mode = str(mode).casefold()
        definitions = self.definitions
        if not isinstance(cell, int) or not 0 < cell <= 0xffffffff or not 1 <= (cell & 65535) <= definitions["cells"]**2:
            raise ValueError("Terrain brushes require a native outdoor cell")
        if len(point) != 3 or not all(math.isfinite(v) and abs(v) < 1_000_000 for v in point):
            raise ValueError("Terrain brush center is invalid")
        if mode not in ("raise", "lower", "flatten", "smooth", "paint"):
            raise ValueError("Unknown terrain brush")
        if not math.isfinite(radius) or not 0 < radius <= definitions["block_side"]*2:
            raise ValueError("Brush radius must be positive and at most two native landblocks")
        if not math.isfinite(strength) or not 0 < strength <= 1000:
            raise ValueError("Brush strength must be positive and at most 1000")
        if mode == "flatten" and target_height is None:
            target_height = float(point[2])
        if target_height is not None and (not math.isfinite(target_height) or abs(target_height) >= 1_000_000):
            raise ValueError("Flatten height is invalid")
        mask = (1<<definitions["terrain_bits"])-1
        if mode == "paint" and (not isinstance(terrain_type, int) or terrain_type not in self.types or not 0 <= terrain_type <= mask):
            raise ValueError("Choose an existing native terrain type")
        step = definitions["block_side"]/(definitions["vertex_side"]-1)
        wx, wy = (cell>>24)*definitions["block_side"]+point[0], ((cell>>16)&255)*definitions["block_side"]+point[1]
        limit = 255*definitions["block_side"]
        if not 0 <= wx <= limit or not 0 <= wy <= limit:
            raise ValueError("Brush center is outside the native world")
        cache = {}
        def block(did):
            if did not in cache:
                if len(cache) >= MAX_STROKE_BLOCKS:
                    raise ValueError("Terrain stroke exceeds the block budget")
                cache[did] = decode_block(self.record(did), definitions, did)
            return cache[did]
        def height(gx, gy):
            owners = self._owners(gx, gy)
            if not owners:
                return None
            did, index = owners[0]
            return definitions["heights"][block(did)["heights"][index]]
        pending = []
        min_x, max_x = math.ceil((wx-radius)/step), math.floor((wx+radius)/step)
        min_y, max_y = math.ceil((wy-radius)/step), math.floor((wy+radius)/step)
        if (max_x-min_x+1)*(max_y-min_y+1) > MAX_STROKE_VERTICES*2:
            raise ValueError("Terrain stroke exceeds the vertex budget")
        for gx in range(min_x, max_x+1):
            for gy in range(min_y, max_y+1):
                distance = math.hypot(gx*step-wx, gy*step-wy)
                if distance >= radius:
                    continue
                owners = self._owners(gx, gy)
                if not owners:
                    continue
                weight = 1-distance/radius
                if mode == "paint":
                    pending.append((owners, None))
                    if len(pending) > MAX_STROKE_VERTICES:
                        raise ValueError("Terrain stroke exceeds the vertex budget")
                    continue
                old = height(gx, gy)
                if mode == "raise":
                    wanted = old+strength*weight
                elif mode == "lower":
                    wanted = old-strength*weight
                elif mode == "flatten":
                    wanted = old+(target_height-old)*min(1, strength)*weight
                else:
                    neighbors = [height(gx+x, gy+y) for x in (-1,0,1) for y in (-1,0,1)]
                    neighbors = [v for v in neighbors if v is not None]
                    wanted = old+(sum(neighbors)/len(neighbors)-old)*min(1, strength)*weight
                # The native table contains a few non-monotonic entries; do not
                # bisect it or assume byte order is elevation order.
                value = min(self.height_candidates, key=lambda pair: (abs(pair[1]-wanted), pair[0]))[0]
                pending.append((owners, value))
                if len(pending) > MAX_STROKE_VERTICES:
                    raise ValueError("Terrain stroke exceeds the vertex budget")
        # All source heights above came from the same snapshot. Smoothing never
        # feeds earlier writes into later samples and failed strokes write nothing.
        for owners, value in pending:
            for did, index in owners:
                target = block(did)
                if mode == "paint":
                    target["infos"][index] = (target["infos"][index]&~mask)|terrain_type
                else:
                    target["heights"][index] = value
        result = {}
        for did, value in sorted(cache.items()):
            after = encode_block(value, definitions)
            if after != value["original"]:
                result[did] = (value["original"], after)
        return result
