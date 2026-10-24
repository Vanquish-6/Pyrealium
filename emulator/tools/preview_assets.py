"""Read-only visual/setup traversal for the editor's native DAT geometry preview.

VisualDesc::StreamPack 0063d7f0, PartGroupDataDesc::StreamPack 006443c0,
CharacterSetup::UnPack 006105a0. Unsupported appearance graphs fail explicitly.
"""
from collections import OrderedDict
import math
from pathlib import Path
import struct

from ac2_dat import Dat


class Reader:
    def __init__(self, data):
        self.data, self.offset = data, 0

    def take(self, fmt):
        size = struct.calcsize("<" + fmt)
        if self.offset + size > len(self.data):
            raise ValueError("Truncated visual/setup record")
        values = struct.unpack_from("<" + fmt, self.data, self.offset)
        self.offset += size
        return values[0] if len(values) == 1 else values


def group(reader):
    flags = reader.take("I")
    if flags & ~0x1ff:
        raise ValueError("Unknown part-group field mask")
    result = {}
    for bit, name in ((1, "key"), (2, "parent"), (4, "connection"), (8, "setup"), (16, "animation")):
        if flags & bit:
            result[name] = reader.take("I")
    if flags & 0x20:
        raise ValueError("Part-group appearance-key hash is not supported yet")
    if flags & 0x40:
        result["effects"] = reader.take("I")
    if flags & 0x180:
        raise ValueError("Part-group effect overrides are not supported yet")
    return result


def visual_record(data, did):
    reader = Reader(data)
    flags = reader.take("I")
    if flags & ~0x7ff or not flags & 1 or reader.take("I") != did:
        raise ValueError("Unsupported visual description header")
    result = {"parent": 0, "groups": {}}
    for bit, name in ((2, "parent"), (4, "motion"), (8, "behavior"), (16, "modes")):
        if flags & bit:
            result[name] = reader.take("I")
    if flags & 0x20:
        scale = reader.take("3f")
        if any(not math.isfinite(v) or v <= 0 or v > 1000 for v in scale):
            raise ValueError("Invalid visual scale")
        result["scale"] = scale
    for bit in (0x40, 0x400):
        if flags & bit:
            reader.take("f")
    if flags & 0x80:
        raise ValueError("Visual icon-layer list is not supported yet")
    if flags & 0x100:
        result["global"] = group(reader)
    if flags & 0x200:
        count = reader.take("I") & 0xffffff
        if count > 256:
            raise ValueError("Visual part-group capacity")
        for unused in range(count):
            key = reader.take("I")
            if key in result["groups"]:
                raise ValueError("Duplicate visual part group")
            result["groups"][key] = group(reader)
    if reader.offset != len(data):
        raise ValueError("Unsupported trailing visual fields")
    return result


def setup_meshes(data, did):
    reader = Reader(data)
    if reader.take("I") != did:
        raise ValueError("Setup identity mismatch")
    reader.take("I")  # Native setup flags precede the mesh DID array.
    count = reader.take("I")
    if not 1 <= count <= 256:
        raise ValueError("Unsupported setup mesh count")
    meshes = [reader.take("I") for unused in range(count)]
    if any(mesh >> 24 not in (1, 6) for mesh in meshes):
        raise ValueError("Unsupported setup mesh family")
    return meshes


class Assets:
    """Owned by one loader thread; bounded decoded geometry cache."""
    def __init__(self, folder):
        self.folder = Path(folder)
        self.portal = Dat(self.folder / "portal.dat")
        try:
            self.cells = Dat(self.folder / "cell_1.dat")
        except Exception:
            self.portal.close()
            raise
        self.cache = OrderedDict()
        self.materials = OrderedDict()
        self.animations = OrderedDict()
        self.material_warnings = set()
        try:
            self.surfaces = Dat(self.folder / "highres.dat") if (self.folder / "highres.dat").exists() else None
        except Exception:
            self.portal.close()
            self.cells.close()
            raise

    def close(self):
        self.portal.close()
        self.cells.close()
        if self.surfaces:
            self.surfaces.close()

    def mesh(self, did):
        from preview_mesh import decode_mesh
        if did in self.cache:
            self.cache.move_to_end(did)
            return self.cache[did]
        value = decode_mesh(self.portal.read(did))
        self.color_mesh(value)
        self.cache[did] = value
        if len(self.cache) > 32:
            self.cache.popitem(last=False)
        return value

    def color_mesh(self, mesh):
        from preview_material import material_info
        colors = [(130, 145, 155)] * (len(mesh["indices"]) // 3)
        mesh["material_images"] = {}
        for group in mesh["groups"]:
            material = mesh["materials"][group["material"]]
            if material not in self.materials:
                try:
                    self.materials[material] = material_info(self.portal, material, self.surfaces, max_size=256)
                except (OSError, ValueError, KeyError) as error:
                    self.materials[material] = {"average": [130, 145, 155], "image_error": str(error)}
                if len(self.materials) > 32:
                    self.materials.popitem(last=False)
            info = self.materials[material]
            self.materials.move_to_end(material)
            image = info.get("image")
            if image:
                mesh["material_images"][group["material"]] = image
            if info.get("image_error"):
                self.material_warnings.add(f"Material {material:08x}: {info['image_error']}")
            for offset in range(group["start"], group["start"] + group["count"], 3):
                color = info.get("average", [130, 145, 155])
                if image and mesh["uvs"]:
                    indices = mesh["indices"][offset:offset+3]
                    u, v = (sum(mesh["uvs"][index][axis] for index in indices)/3 for axis in range(2))
                    x, y = int((u % 1) * image["width"]), int((v % 1) * image["height"])
                    pixel = (y * image["width"] + x) * 3
                    color = tuple(image["rgb"][pixel:pixel+3])
                colors[offset//3] = color
        mesh["triangle_colors"] = colors

    def visual(self, did, trail=()):
        if did in trail or len(trail) >= 16 or did >> 24 != 0x1f:
            raise ValueError("Cyclic, invalid or excessive visual inheritance")
        result = visual_record(self.portal.read(did), did)
        if result["parent"]:
            parent = self.visual(result["parent"], trail + (did,))
            parent["groups"].update(result["groups"])
            if "scale" in result:
                parent["scale"] = result["scale"]
            result = parent
        return result

    def creature(self, visual):
        description = self.visual(visual)
        groups = list(description["groups"].values())
        if len(groups) != 1 or groups[0].get("parent", 0):
            raise ValueError("Attached multi-part visual groups need connection-frame support")
        setup = groups[0].get("setup", 0)
        if setup >> 24 != 4:
            raise ValueError("Visual has no supported character setup")
        return [(self.mesh(did), description.get("scale", (1, 1, 1)))
                for did in setup_meshes(self.portal.read(setup), setup)]

    def animation(self, visual):
        from preview_animation import load_animation_set
        if visual in self.animations:
            self.animations.move_to_end(visual)
            return self.animations[visual]
        description = self.visual(visual)
        groups = list(description["groups"].values())
        if len(groups) != 1 or groups[0].get("animation", 0) >> 24 != 3:
            raise ValueError("Visual has no supported single-group animation map")
        animation = load_animation_set(self.portal, groups[0]["setup"], groups[0]["animation"])
        self.animations[visual] = animation
        if len(self.animations) > 4:
            self.animations.popitem(last=False)
        return animation

    def scene(self, cell, visual, position, heading, scale, placements, catalog):
        from preview_cell import decode_cell
        warnings, instances = [], []
        triangle_count = 0

        def add(mesh, origin, quaternion, sizes, role):
            nonlocal triangle_count
            count = len(mesh["indices"]) // 3
            if triangle_count + count > 50000:
                raise ValueError("Scene geometry exceeds the 50,000-triangle authoring budget")
            triangle_count += count
            instances.append(dict(mesh=mesh, position=origin, quaternion=quaternion,
                                  scale=sizes, role=role, placement_scale=scale or 1))

        # Reserve the selected model first so background limits never omit it.
        rotation = (math.cos(heading * math.pi / 255), 0, 0, -math.sin(heading * math.pi / 255))
        try:
            for mesh, native_scale in self.creature(visual):
                add(mesh, position, rotation, [v * (scale or 1) for v in native_scale], "preview")
        except (OSError, ValueError, KeyError) as error:
            warnings.append(f"Selected visual {visual:08x}: {error}")
        try:
            animation = self.animation(visual)
            for instance in instances:
                if instance["role"] == "preview":
                    try:
                        animation.prepare_mesh(instance["mesh"])
                        instance["animation_set"] = animation
                    except ValueError as error:
                        warnings.append(f"Animation mesh {instance['mesh']['did']:08x}: {error}")
            warnings.extend(animation.warnings)
        except (OSError, ValueError, KeyError) as error:
            warnings.append(f"Animation: {error}")
        if not 0x100 <= (cell & 65535) <= 0xfffb:
            warnings.append("Outdoor terrain geometry is not supported by this preview yet")
            return instances, warnings
        current = decode_cell(self.cells.read(cell))
        if current["cell"] != cell:
            raise ValueError("Cell record identity mismatch")
        neighbors = dict.fromkeys([cell] + current["visible_cells"] + current["shared_cells"])
        loaded = set()
        static = []
        for identity in list(neighbors)[:32]:
            if identity >> 16 != cell >> 16 or not 0x100 <= (identity & 65535) <= 0xfffb:
                continue
            try:
                room = current if identity == cell else decode_cell(self.cells.read(identity))
                if room["cell"] != identity:
                    raise ValueError("Neighbor identity mismatch")
                add(self.mesh(room["mesh"]), room["position"], room["quaternion"], (1, 1, 1), "world")
                loaded.add(identity)
                static.extend(room["placements"])
            except (OSError, ValueError, KeyError) as error:
                warnings.append(f"Room {identity:08x}: {error}")
        if len(neighbors) > 32:
            warnings.append("Neighbor room preview is limited to 32 cells")
        # Static DAT placements are already in block space, unlike room meshes.
        static.sort(key=lambda row: sum((a-b)**2 for a,b in zip(row["position"], position)))
        missing = 0
        seen = set()
        for row in static[:64]:
            key = (row["definition"], *row["position"])
            if key in seen:
                continue
            seen.add(key)
            target = catalog.get(row["definition"])
            if target is None:
                missing += 1
                continue
            try:
                for mesh, sizes in self.creature(target):
                    add(mesh, row["position"], row["quaternion"], [a*b for a,b in zip(sizes,row["scale"])], "world")
            except (OSError, ValueError, KeyError):
                missing += 1
        if missing:
            warnings.append(f"{missing} nearby static props have unsupported or unmapped visuals")
        for row in placements[:16384]:
            if int(row[3], 16) not in loaded:
                continue
            try:
                for mesh, sizes in self.creature(int(row[2], 16)):
                    factor = float(row[11]) if len(row) == 12 and row[11] != "-" else 1
                    add(mesh, [float(v) for v in row[4:7]], [float(v) for v in row[7:11]],
                        [v * factor for v in sizes], "saved")
            except (OSError, ValueError, KeyError) as error:
                warnings.append(f"Saved object {row[0]}: {error}")
        warnings.extend(sorted(self.material_warnings)[:8])
        return instances, warnings
