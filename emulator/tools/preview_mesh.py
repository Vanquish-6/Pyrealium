"""Bounded read-only native AC2 render-mesh prefix decoding for the editor.

Evidence: matching build-7148 CBaseMesh::StreamPack 004d2ab0,
CDataMesh::StreamPack 004d3bf0, CCellMesh::StreamPack 004cf3a0,
CVertexArray::UnPack 00730160, VertexFormatInfo::GenerateOffsets 0062b380,
CMeshTriList::UnPack 0052c260 and RenderIndexBuffer::Stream 005bdea0.
Only render geometry is decoded. Physics polygons and BSP tails are not consumed
or claimed valid. Skin weights are retained; animation/skin deformation is not
evaluated. Coordinates returned are the authored default/bind-pose vertices.
"""
import math
import struct


class MeshFormatError(ValueError):
    pass


class Reader:
    def __init__(self, data):
        if not isinstance(data, (bytes, bytearray, memoryview)) or len(data) > 64 * 1024 * 1024:
            raise MeshFormatError("mesh exceeds the 64 MiB record limit")
        self.data = memoryview(data)
        self.pos = 0

    def take(self, size):
        if size < 0 or size > len(self.data) - self.pos:
            raise MeshFormatError(f"truncated native mesh at {self.pos:#x}")
        start = self.pos
        self.pos += size
        return self.data[start:self.pos]

    def u32(self):
        return struct.unpack("<I", self.take(4))[0]

    def floats(self, count):
        values = struct.unpack(f"<{count}f", self.take(4 * count))
        if any(not math.isfinite(value) or abs(value) > 1_000_000 for value in values):
            raise MeshFormatError("non-finite or out-of-range mesh geometry")
        return values

    def align(self):
        self.take((-self.pos) & 3)


def vertex_layout(format_word):
    """Mirror the native offsets, including its packed matrix-index byte array."""
    # Low position mask, optional normal/size/colors, 0..8 UV pairs,
    # matrix count byte, stride-padding flag, and optional tangent vectors.
    allowed = 0x71000000 | 0x00FF0000 | 0xF00 | 0xFE
    if format_word & ~allowed:
        raise MeshFormatError(f"unsupported vertex format bits {format_word:#x}")
    position = format_word & 0xE
    if position not in (2, 4, 6, 8, 10, 12, 14):
        raise MeshFormatError("vertex format has no supported XYZ position")
    # The recovered native reader allocates 12 position bytes for every nonzero
    # position mask. Position type 4 (RHW) is uncommon; do not infer its semantics.
    if position == 4:
        raise MeshFormatError("RHW position format is not a world-space mesh")
    weights = {6: 1, 8: 2, 10: 3, 12: 4, 14: 5}.get(position, 0)
    uv_count = (format_word >> 8) & 15
    matrices = (format_word >> 16) & 255
    if uv_count > 8 or matrices > 64:
        raise MeshFormatError("vertex UV or skin-influence count outside editor bounds")
    offset = 12 + weights * 4
    normal = offset if format_word & 0x10 else None
    offset += 12 if normal is not None else 0
    offset += 4 if format_word & 0x20 else 0
    diffuse = offset if format_word & 0x40 else None
    offset += 4 if diffuse is not None else 0
    offset += 4 if format_word & 0x80 else 0
    for flag in (0x10000000, 0x20000000, 0x40000000):
        offset += 12 if format_word & flag else 0
    uv = offset if uv_count else None
    offset += uv_count * 8
    matrix_indices = offset
    offset += matrices
    matrix_weights = offset
    offset += matrices * 4
    stride = offset
    if format_word & 0x01000000 and stride < 256:
        # GenerateOffsets rounds *strictly upward*, including exact boundaries.
        stride = ((stride // 32) + 1) * 32
    if not 12 <= stride <= 1024:
        raise MeshFormatError("unsupported native vertex stride")
    return dict(stride=stride, normal=normal, diffuse=diffuse, uv=uv,
                uv_count=uv_count, matrices=matrices, matrix_indices=matrix_indices,
                matrix_weights=matrix_weights, legacy_weights=weights)


def decode_mesh(data):
    """Return real mesh vertices, native triangles, material IDs and UVs.

    Supports the observed CDataMesh/CCellMesh (DID types 06/01) render prefix.
    Returns a flat triangle-index list, plus material groups for the first
    stored degrade. Every stored degrade is bounded and structurally read.
    Trailing physics data is explicitly left opaque.
    """
    reader = Reader(data)
    did = reader.u32()
    if did >> 24 not in (1, 6):
        raise MeshFormatError(f"not a CDataMesh/CCellMesh record: {did:#010x}")
    mesh_type = reader.u32()
    if mesh_type != 2:
        raise MeshFormatError(f"unsupported native mesh type {mesh_type}")
    material_count = reader.u32()
    if not 0 < material_count <= 4096:
        raise MeshFormatError("invalid native mesh material count")
    materials = [reader.u32() for _ in range(material_count)]
    if any(material >> 24 != 0x31 for material in materials):
        raise MeshFormatError("unsupported render material reference")
    format_word, vertex_count = reader.u32(), reader.u32()
    if not 0 < vertex_count <= 65536:
        raise MeshFormatError("native vertex count outside u16 index bounds")
    layout = vertex_layout(format_word)
    vertices, normals, uvs, colors, skin = [], [], [], [], []
    raw = reader.take(vertex_count * layout["stride"])
    for index in range(vertex_count):
        base = index * layout["stride"]
        point = struct.unpack_from("<3f", raw, base)
        if any(not math.isfinite(v) or abs(v) > 1_000_000 for v in point):
            raise MeshFormatError("invalid native vertex coordinate")
        vertices.append(list(point))
        if layout["normal"] is not None:
            normal = struct.unpack_from("<3f", raw, base + layout["normal"])
            if any(not math.isfinite(v) for v in normal):
                raise MeshFormatError("invalid native vertex normal")
            normals.append(list(normal))
        if layout["uv"] is not None:
            uv = struct.unpack_from("<2f", raw, base + layout["uv"])
            if any(not math.isfinite(v) for v in uv):
                raise MeshFormatError("invalid native vertex UV")
            uvs.append(list(uv))
        if layout["diffuse"] is not None:
            color = struct.unpack_from("<I", raw, base + layout["diffuse"])[0]
            colors.append(color)
        if layout["matrices"]:
            count = layout["matrices"]
            bones = list(raw[base + layout["matrix_indices"]:base + layout["matrix_indices"] + count])
            weights = struct.unpack_from(f"<{count}f", raw, base + layout["matrix_weights"])
            if any(not math.isfinite(v) or not -0.0001 <= v <= 1.0001 for v in weights):
                raise MeshFormatError("invalid native skin weight")
            skin.append(dict(bones=bones, weights=list(weights)))
    reader.align()
    bbox = reader.floats(6)
    sphere = reader.floats(4)
    if any(bbox[i] > bbox[i + 3] for i in range(3)) or sphere[3] < 0:
        raise MeshFormatError("invalid native mesh bounds")
    degrade_count = reader.u32()
    if not 0 < degrade_count <= 64:
        raise MeshFormatError("native mesh has no supported render degrade")
    indices, groups, degrades = [], [], []
    total_indices = 0
    for degrade in range(degrade_count):
        mode, detail_levels, fragments = reader.u32(), reader.u32(), reader.u32()
        if fragments > material_count or fragments > 4096:
            raise MeshFormatError("native mesh fragment count exceeds material count")
        count_for_degrade = 0
        for _ in range(fragments):
            material_index, count = reader.u32(), reader.u32()
            total_indices += count
            if material_index >= material_count or count % 3 or total_indices > 4_000_000:
                raise MeshFormatError("invalid triangle material, count or mesh index budget")
            values = struct.unpack(f"<{count}H", reader.take(count * 2))
            reader.align()
            if any(value >= vertex_count for value in values):
                raise MeshFormatError("native triangle refers outside its vertex array")
            count_for_degrade += count
            if degrade == 0:
                start = len(indices)
                indices.extend(values)
                groups.append(dict(start=start, count=count, material=material_index))
        degrades.append(dict(mode=mode, detail_levels=detail_levels, indices=count_for_degrade))
    if not indices:
        raise MeshFormatError("native mesh has no render triangles in its first degrade")
    return dict(did=did, mesh_type=mesh_type, format=format_word,
                vertices=vertices, indices=indices, normals=normals, uvs=uvs,
                colors=colors, materials=materials, groups=groups, skin=skin,
                bounds=[list(bbox[:3]), list(bbox[3:])], sphere=list(sphere),
                degrades=degrades, render_prefix_bytes=reader.pos,
                opaque_tail_bytes=len(data) - reader.pos,
                scope="Native default-pose render geometry; physics/BSP tail and animation are not decoded")
