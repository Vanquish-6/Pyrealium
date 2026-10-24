"""Bounded, read-only AC2 interior-cell geometry references for editor previews.

Evidence: build-7148 CEnvCell::StreamPack 00482da0, Position::UnPack
006200b0, CCellPortal::UnPack 0049c980, EntityDesc::StreamPack 00617370,
and raw Frame stream adapter 006161b0. This decodes geometry references and
transforms, not BSP collision, visibility traversal, dynamic spawns or properties.
"""
import math
import struct


class CellFormatError(ValueError):
    pass


class Reader:
    def __init__(self, data):
        if not 44 <= len(data) <= 4 * 1024 * 1024:
            raise CellFormatError("Interior-cell record size is outside preview bounds")
        self.data = memoryview(data)
        self.offset = 0

    def unpack(self, format):
        size = struct.calcsize("<" + format)
        if self.offset + size > len(self.data):
            raise CellFormatError(f"Truncated interior-cell field at {self.offset:#x}")
        result = struct.unpack_from("<" + format, self.data, self.offset)
        self.offset += size
        return result

    def u32(self):
        return self.unpack("I")[0]

    def count(self, maximum, label):
        value = self.u32()
        if value > maximum:
            raise CellFormatError(f"{label} exceeds preview bounds")
        return value

    def align(self):
        target = (self.offset + 3) & ~3
        if target > len(self.data):
            raise CellFormatError("Truncated interior-cell alignment")
        self.offset = target

    def floats(self, count):
        values = list(self.unpack(f"{count}f"))
        if any(not math.isfinite(value) or abs(value) >= 1_000_000 for value in values):
            raise CellFormatError("Invalid interior-cell transform")
        return values

    def quaternion(self):
        values = self.floats(4)
        if abs(sum(value * value for value in values) - 1) > 0.01:
            raise CellFormatError("Interior-cell quaternion is not normalized")
        return values


def decode_cell(data):
    """Return the room mesh DID and native block-space placements.

    The returned room mesh vertices need the returned position/quaternion
    applied. Entity placements already contain native block-space positions;
    applying the cell transform to those a second time would be incorrect.
    Quaternion component order is [w,x,y,z]. Entity matrices are the nine
    cached Frame floats preserved from the record, in native storage order.

    Unknown entity property payloads stop only entity-list decoding and produce
    an explicit warning. The room mesh reference remains available. Property
    metadata is intentionally not interpreted as geometry or gameplay rules.
    """
    reader = Reader(data)
    cell = reader.u32()
    if not 0x100 <= (cell & 0xffff) <= 0xfffb:
        raise CellFormatError("Preview cell is not a supported interior cell")
    position = reader.floats(3)
    quaternion = reader.quaternion()
    flags = reader.u32()
    if flags & ~0x1f:
        raise CellFormatError("Unsupported interior-cell flags")
    mesh = reader.u32()
    if mesh >> 24 != 1:
        raise CellFormatError("Interior-cell environment is not a cell-mesh DID")
    block = cell & 0xffff0000
    portals = []
    for _ in range(reader.count(1024, "Portal count")):
        portal_flags, index, other_cell, other_index = reader.unpack("HhHh")
        if portal_flags & ~7:
            raise CellFormatError("Unsupported interior-cell portal flags")
        stabs = []
        if portal_flags & 4:
            count = reader.unpack("H")[0]
            if count > 4096:
                raise CellFormatError("Portal stab list exceeds preview bounds")
            stabs = [block | value for value in reader.unpack(f"{count}H")]
            reader.align()
        portals.append({
            "flags": portal_flags, "index": index,
            "other_cell": 0 if portal_flags & 2 else block | other_cell,
            "other_index": other_index, "stabs": stabs,
        })
    reader.align()
    visible = [block | value for value in reader.unpack(
        f"{reader.count(4096, 'Visible-cell count')}H")]
    reader.align()
    shared = [block | value for value in reader.unpack(
        f"{reader.count(4096, 'Shared-cell count')}H")]
    reader.align()
    result = {
        "cell": cell, "mesh": mesh, "position": position,
        "quaternion": quaternion, "flags": flags,
        "portals": portals, "visible_cells": visible, "shared_cells": shared,
        "placements": [], "warnings": [],
    }
    if flags & 2:
        # EntityGroupDesc begins with its DBObj DID, then bounded arrays.
        result["entity_group"] = reader.u32()
        count = reader.count(4096, "Static-entity count")
        for index in range(count):
            start = reader.offset
            entity_flags = reader.u32()
            if entity_flags & ~0x1ff:
                result["warnings"].append(
                    f"Static entity {index} at {start:#x} has unsupported flags; later entities omitted")
                return result
            identity = reader.u32() if entity_flags & 1 else 0
            kind = reader.u32() if entity_flags & 2 else 0
            runtime_id = reader.unpack("Q")[0] if entity_flags & 4 else 0
            definition = reader.u32() if entity_flags & 8 else 0
            origin, rotation = [0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]
            matrix = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
            if entity_flags & 0x10:
                # This raw Frame adapter copies 64 bytes, including cached matrix.
                # It is distinct from Position's compact 28-byte Frame::UnPack.
                origin = reader.floats(3)
                rotation = reader.quaternion()
                matrix = reader.floats(9)
            scale = reader.floats(3) if entity_flags & 0x20 else [1.0, 1.0, 1.0]
            if any(value <= 0 or value > 1000 for value in scale):
                raise CellFormatError("Static-entity scale is outside preview bounds")
            placement = {
                "definition": definition, "position": origin, "quaternion": rotation,
                "scale": scale, "matrix": matrix, "type": kind,
                "identity": identity, "runtime_id": runtime_id,
            }
            result["placements"].append(placement)
            if entity_flags & 0x80:
                result["warnings"].append(
                    f"Static entity {index} has property metadata; later entities omitted")
                return result
            if entity_flags & 0x40:
                placement["version"] = reader.u32()
            if entity_flags & 0x100:
                result["warnings"].append(
                    f"Static entity {index} has an authoring name; later entities omitted")
                return result
        links = reader.count(4096, "Entity-link count")
        if links:
            result["warnings"].append("Entity links and later property metadata are omitted")
            return result
    if flags & 4:
        result["warnings"].append("Cell property metadata is omitted from the geometry preview")
    result["decoded_bytes"] = reader.offset
    return result
