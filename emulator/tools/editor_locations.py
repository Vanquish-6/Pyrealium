from contextlib import nullcontext
"""Read-only area inventory and geometry-backed navigation anchors.

The DAT index establishes available blocks/rooms. Content labels are browsing
anchors, not claims about original region names, spawn rules, or walkability.
"""
from pathlib import Path
import math

from ac2_dat import Dat
from place_content import bounded_text


MAX_LOCATIONS = 250_000


def location_kind(cell):
    if not 0 < cell <= 0xffffffff:
        raise ValueError("Cell identifier must fit an unsigned 32-bit value")
    low = cell & 65535
    if 1 <= low <= 64:
        return "Outdoor"
    if 0x100 <= low <= 0xfffb:
        return "Dungeon"
    raise ValueError("This identifier is not an outdoor cell or interior room")


def _position(values):
    values = [float(v) for v in values]
    if len(values) != 3 or any(not math.isfinite(v) or abs(v) >= 1_000_000 for v in values):
        raise ValueError("Invalid navigation position")
    return values


def build_locations(game_folder, content_folder, catalog):
    """Return indexed areas and named anchors without decoding room payloads.

    Each row contains ``cell``, ``name``, ``kind``, and ``position``. A None
    position means resolve_location must find a point on its native geometry.
    All supplied text and archive reads remain bounded by their normal readers.
    """
    with Dat(Path(game_folder) / "cell_1.dat") as cells:
        identifiers = set(cells.index())
    blocks = {did & 0xffff0000 for did in identifiers if did & 65535 == 65535}
    rooms = {did for did in identifiers if 0x100 <= did & 65535 <= 0xfffb}
    if len(blocks) + len(rooms) > MAX_LOCATIONS:
        raise ValueError(f"Location inventory exceeds {MAX_LOCATIONS:,} areas")
    labels = {}
    for entity, visual, label in catalog:
        labels.setdefault(int(entity, 16), label)
    anchors = {}

    def add(cell, position, label):
        kind = location_kind(cell)
        if (kind == "Outdoor" and cell & 0xffff0000 not in blocks) or (kind == "Dungeon" and cell not in rooms):
            return
        position = _position(position)
        # Multiple source packs contain the same base objects. Keep the first
        # observed anchor and accumulate distinct grounded catalog labels.
        anchor = anchors.setdefault(cell, {"cell": cell, "kind": kind, "position": position, "labels": []})
        if label not in anchor["labels"] and len(anchor["labels"]) < 4:
            anchor["labels"].append(label[:120])

    folder = Path(content_folder)
    for name in ("world.tsv", "training-world.tsv", "observed-world.tsv", "local-world.tsv"):
        path = folder / name
        if not path.exists():
            continue
        lines = bounded_text(path).splitlines()
        if not lines or lines[0] not in ("pyrealium-world-v1", "pyrealium-world-v2", "pyrealium-world-v3"):
            continue
        width = {"1": 8, "2": 11, "3": 12}[lines[0][-1]]
        for line in lines[1:]:
            if not line or line.startswith("#"):
                continue
            row = line.split("\t")
            if len(row) != width:
                continue
            try:
                entity, cell = int(row[1], 16), int(row[3], 16)
                add(cell, row[4:7], labels.get(entity, f"Entity {entity:08x}"))
            except (ValueError, OverflowError):
                continue
    routes = folder / "portal-routes.tsv"
    if routes.exists():
        lines = bounded_text(routes).splitlines()
        if lines and lines[0] == "pyrealium-portal-routes-v1":
            for line in lines[1:]:
                if not line or line.startswith("#"):
                    continue
                row = line.split("\t")
                if len(row) != 12:
                    continue
                try:
                    source, destination = int(row[2], 16), int(row[6], 16)
                    add(destination, row[7:10], f"Portal destination from {source:08x}")
                except (ValueError, OverflowError):
                    continue
    result = []
    for cell, anchor in anchors.items():
        result.append(dict(cell=cell, name=" · ".join(anchor["labels"]),
                           kind=anchor["kind"], position=anchor["position"]))
    for block in sorted(blocks):
        cell = block | 1
        if cell not in anchors:
            result.append(dict(cell=cell, name=f"Outdoor block {block >> 16:04x}", kind="Outdoor", position=None))
    for cell in sorted(rooms):
        if cell not in anchors:
            result.append(dict(cell=cell, name=f"Interior room {cell:08x}", kind="Dungeon", position=None))
    if len(result) > MAX_LOCATIONS:
        raise ValueError(f"Location inventory exceeds {MAX_LOCATIONS:,} entries")
    return sorted(result, key=lambda row: (row["position"] is None, row["name"].casefold(), row["cell"]))


def location_links(game_folder, cell, cells=None):
    """Return existing adjacent blocks or native room links for navigation.

    Doorway links come from CEnvCell portals. Visible/shared cells are separate
    browsing links and are explicitly named; they do not imply walkability or
    a connecting doorway. Geometry anchors are resolved only when a link opens.
    """
    from preview_cell import decode_cell
    kind = location_kind(cell)
    with (nullcontext(cells) if cells is not None else Dat(Path(game_folder) / "cell_1.dat")) as cells:
        entries = cells.index()
        if kind == "Outdoor":
            bx, by, low = cell >> 24, (cell >> 16) & 255, cell & 65535
            result = []
            for name, dx, dy in (("North", 0, 1), ("East", 1, 0), ("South", 0, -1), ("West", -1, 0)):
                x, y = bx+dx, by+dy
                if not 0 <= x < 255 or not 0 <= y < 255:
                    continue
                block = (x << 24) | (y << 16)
                if block | 65535 in entries:
                    target = block | low
                    result.append(dict(cell=target, name=f"{name} · block {block >> 16:04x}",
                                       kind="Outdoor", position=None, connection="Adjacent block"))
            return result
        room = decode_cell(_read_bounded(cells, cell, 4*1024*1024))
        if room["cell"] != cell:
            raise ValueError("Interior room identity mismatch")
        result, seen = [], {cell}
        groups = (("Doorway", [portal["other_cell"] for portal in room["portals"]]),
                  ("Visible room", room["visible_cells"]), ("Shared room", room["shared_cells"]))
        for label, targets in groups:
            for target in targets:
                if target in seen or target not in entries or not 0x100 <= target & 65535 <= 0xfffb:
                    continue
                seen.add(target)
                result.append(dict(cell=target, name=f"{label} · {target:08x}",
                                   kind="Dungeon", position=None, connection=label))
        return result


def room_below(point, instances, max_drop=20):
    """Infer a visible room floor below an Explore camera without changing data.

    Inputs use transformed preview vertices. This is a bounded downward render
    ray heuristic for editor streaming, not native BSP containment, collision,
    portal traversal, or proof that the camera is in a walkable room.
    """
    if len(point) != 3 or any(not math.isfinite(v) for v in point):
        return None
    if not math.isfinite(max_drop) or max_drop < 0:
        raise ValueError("Maximum floor drop must be finite and nonnegative")
    px, py, pz = point
    best = None
    for item in instances:
        cell = item.get("cell")
        if item.get("role") != "world" or not isinstance(cell, int) or not 0 < cell <= 0xffffffff:
            continue
        if not 0x100 <= cell & 65535 <= 0xfffb:
            continue
        vertices, indices = item.get("vertices", ()), item.get("indices", ())
        for offset in range(0, len(indices)-2, 3):
            a, b, c = (vertices[index] for index in indices[offset:offset+3])
            # Fast XY rejection before the barycentric and normal arithmetic.
            if px < min(a[0], b[0], c[0]) or px > max(a[0], b[0], c[0]):
                continue
            if py < min(a[1], b[1], c[1]) or py > max(a[1], b[1], c[1]):
                continue
            ux, uy, uz = b[0]-a[0], b[1]-a[1], b[2]-a[2]
            vx, vy, vz = c[0]-a[0], c[1]-a[1], c[2]-a[2]
            determinant = ux*vy-uy*vx
            if abs(determinant) < 1e-10:
                continue
            nx, ny = uy*vz-uz*vy, uz*vx-ux*vz
            length = math.sqrt(nx*nx+ny*ny+determinant*determinant)
            if abs(determinant)/length < .45:
                continue
            x, y = px-a[0], py-a[1]
            u, v = (x*vy-y*vx)/determinant, (ux*y-uy*x)/determinant
            if u < -1e-8 or v < -1e-8 or u+v > 1+1e-8:
                continue
            floor = a[2]+u*uz+v*vz
            drop = pz-floor
            if not -.25 <= drop <= max_drop:
                continue
            candidate = (max(0, drop), cell)
            if best is None or candidate < best:
                best = candidate
    return best[1] if best is not None else None


def _rotate(point, q):
    w, x, y, z = q
    px, py, pz = point
    return [(1-2*(y*y+z*z))*px + 2*(x*y-w*z)*py + 2*(x*z+w*y)*pz,
            2*(x*y+w*z)*px + (1-2*(x*x+z*z))*py + 2*(y*z-w*x)*pz,
            2*(x*z-w*y)*px + 2*(y*z+w*x)*py + (1-2*(x*x+y*y))*pz]


def _read_bounded(archive, did, limit):
    entry = archive.index().get(did)
    if entry is None:
        raise ValueError(f"DAT record {did:08x} is missing")
    if not 0 < entry[1] <= limit:
        raise ValueError(f"DAT record {did:08x} exceeds navigation bounds")
    return archive.read(did)


def _room_mesh(folder, did):
    """Use the same material-filtered surfaces that rendering and picking use."""
    from preview_assets import Assets
    assets = Assets(folder)
    try:
        entry = assets.portal.index().get(did)
        if entry is None or not 0 < entry[1] <= 64*1024*1024:
            raise ValueError(f"Room mesh {did:08x} is missing or exceeds navigation bounds")
        return assets.mesh(did)
    finally:
        assets.close()


def floor_anchor(vertices, indices, target_xy=None, lower_half=False, target_z=None, surface_normals=None):
    """Choose a centroid on a real mostly-horizontal triangle, never a void.

    This is an authoring starting point, not native BSP or walkability testing.
    The lower-half preference avoids choosing the ceiling in enclosed rooms.
    """
    if not vertices or not indices:
        raise ValueError("Area has no render triangles for navigation")
    if target_xy is None:
        target_xy = [(min(v[axis] for v in vertices) + max(v[axis] for v in vertices)) / 2 for axis in (0, 1)]
    mid_z = (min(v[2] for v in vertices) + max(v[2] for v in vertices)) / 2
    candidates = []
    for offset in range(0, len(indices), 3):
        triangle=indices[offset:offset+3]
        if surface_normals is not None and sum(surface_normals[index][2] for index in triangle)/3 < .45:
            continue
        a, b, c = (vertices[index] for index in triangle)
        u, v = [b[i]-a[i] for i in range(3)], [c[i]-a[i] for i in range(3)]
        normal = [u[1]*v[2]-u[2]*v[1], u[2]*v[0]-u[0]*v[2], u[0]*v[1]-u[1]*v[0]]
        norm = math.sqrt(sum(value*value for value in normal))
        if norm < 1e-10 or abs(normal[2])/norm < .45:
            continue
        point = [sum(p[i] for p in (a, b, c))/3 for i in range(3)]
        distance = sum((point[i]-target_xy[i])**2 for i in (0, 1))
        # A room's native translation is commonly its authored floor level.
        # Prefer its floor band over overhead platforms/decoration that happen
        # to cross the mesh's XY center. Still require a real render triangle.
        level = abs(point[2]-target_z) if target_z is not None else 0
        candidates.append((lower_half and point[2] > mid_z + .01,
                           level > 2, 0 if level <= 2 else level, distance, point[2], point))
    if not candidates:
        raise ValueError("No floor-like native surface was found in this area")
    return _position(min(candidates)[-1])


def resolve_location(game_folder, cell, cells=None, portal=None, mesh_loader=None):
    """Return a geometry-backed position for a DAT-indexed cell or room.

    Outdoor anchors stay within the requested cell and respect terrain holes.
    Interior positions include the native room quaternion and translation.
    """
    from preview_cell import decode_cell
    from preview_terrain import region_references, landscape_defs, decode_terrain
    kind = location_kind(cell)
    folder = Path(game_folder)
    with (nullcontext(cells) if cells is not None else Dat(folder / "cell_1.dat")) as cells, (nullcontext(portal) if portal is not None else Dat(folder / "portal.dat")) as portal:
        if kind == "Dungeon":
            room = decode_cell(_read_bounded(cells, cell, 4*1024*1024))
            if room["cell"] != cell:
                raise ValueError("Interior room identity mismatch")
            mesh = mesh_loader(room["mesh"]) if mesh_loader is not None else _room_mesh(folder, room["mesh"])
            vertices = [[a+b for a, b in zip(_rotate(v, room["quaternion"]), room["position"])] for v in mesh["vertices"]]
            return floor_anchor(vertices, mesh["indices"], target_xy=room["position"][:2],
                                lower_half=True, target_z=room["position"][2],surface_normals=[_rotate(v,room["quaternion"]) for v in mesh["normals"]] if mesh.get("normals") else None)
        refs = region_references(_read_bounded(portal, 0x0f000000, 4*1024*1024))
        definitions = landscape_defs(_read_bounded(portal, refs["land_defs"], 4*1024*1024))
        mesh = decode_terrain(_read_bounded(cells, cell | 65535, 4*1024*1024), definitions)
        side, count = definitions["cell_side"], definitions["cells"]
        x, y = divmod((cell & 65535)-1, count)
        if x >= count:
            raise ValueError("Outdoor cell exceeds native landscape dimensions")
        vertices, indices = mesh["vertices"], []
        for offset in range(0, len(mesh["indices"]), 3):
            triangle = mesh["indices"][offset:offset+3]
            point = [sum(vertices[i][axis] for i in triangle)/3 for axis in (0, 1)]
            if x*side <= point[0] < (x+1)*side and y*side <= point[1] < (y+1)*side:
                indices.extend(triangle)
        return floor_anchor(vertices, indices, [(x+.5)*side, (y+.5)*side])
