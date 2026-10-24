"""Rigid selection transforms and atomic, DAT-validated arrangement edits."""
import math
import struct
from pathlib import Path

from ac2_dat import Dat
from editor_document import rows_from_text
from preview_terrain import placement_from_point, region_references, landscape_defs


MAX_SELECTION = 256


def remove_selection(original, identities):
    rows = rows_from_text(original)
    identities = set(identities)
    existing = {int(row[0], 16) for row in rows}
    if not identities or not identities <= existing:
        raise ValueError("Selected objects no longer exist")
    lines = [line for i, line in enumerate(original.splitlines())
             if not i or not line or line.startswith("#") or int(line.split("\t", 1)[0], 16) not in identities]
    result = "\n".join(lines) + "\n"
    rows_from_text(result)
    return result


def native_number(value):
    value = struct.unpack("<f", struct.pack("<f", value))[0]
    if not math.isfinite(value) or abs(value) >= 1_000_000:
        raise ValueError("Transform is outside the supported native range")
    return format(value, ".9g")


def landscape_grid(game_folder):
    with Dat(Path(game_folder) / "portal.dat") as portal:
        refs = region_references(portal.read(0x0f000000))
        result = landscape_defs(portal.read(refs["land_defs"]))
        return {name: result[name] for name in ("block_side", "cell_side", "cells")}


def arrangement_rows(rows, translation=(0, 0, 0), degrees=0, copies=0, grid=0, landscape=None, scale=1):
    """Return transformed rows; copy count 0 edits, 1..32 makes a spaced array.

    Outdoor XYZ is canonicalized using the archive's actual landscape dimensions.
    Interior group transforms stay within one authored room's coordinate frame.
    Native quaternion tilt and each object's own scale are retained.
    """
    if not rows or len(rows) > MAX_SELECTION:
        raise ValueError(f"Select between 1 and {MAX_SELECTION} objects")
    if len(translation) != 3 or not all(math.isfinite(v) and abs(v) < 1_000_000 for v in translation):
        raise ValueError("Enter finite X/Y/Z offsets")
    if not math.isfinite(degrees) or abs(degrees) > 36000 or not math.isfinite(grid) or not 0 <= grid <= 1000:
        raise ValueError("Rotation or grid spacing is outside the supported range")
    if not isinstance(copies, int) or not 0 <= copies <= 32 or len(rows) * max(1, copies) > MAX_SELECTION:
        raise ValueError("At most 32 copies and 256 resulting objects are supported per edit")
    if not math.isfinite(scale) or not .01 <= scale <= 100:
        raise ValueError("Scale multiplier must be between 0.01 and 100")
    cells = [int(row[3], 16) for row in rows]
    outdoor = [1 <= (cell & 65535) <= 64 for cell in cells]
    if any(outdoor) != all(outdoor):
        raise ValueError("Select outdoor objects or objects from one interior room")
    if not all(outdoor) and len(set(cells)) != 1:
        raise ValueError("Interior group edits require objects from the same room")
    if all(outdoor) and landscape is None:
        raise ValueError("Native landscape dimensions are required for outdoor arrangements")
    reference = cells[0]
    positions = [[float(v) for v in row[4:7]] for row in rows]
    if all(outdoor):
        for cell, point in zip(cells, positions):
            point[0] += ((cell >> 24) - (reference >> 24)) * landscape["block_side"]
            point[1] += (((cell >> 16) & 255) - ((reference >> 16) & 255)) * landscape["block_side"]
    pivot = [sum(p[axis] for p in positions) / len(positions) for axis in range(3)]
    result = []
    for step in range(1, max(1, copies) + 1):
        angle = -math.radians(degrees * step)
        cosine, sine = math.cos(angle), math.sin(angle)
        half_w, half_z = math.cos(angle / 2), math.sin(angle / 2)
        for source, point in zip(rows, positions):
            factor = scale ** step
            dx, dy = (point[0] - pivot[0])*factor, (point[1] - pivot[1])*factor
            target = [pivot[0] + cosine*dx - sine*dy + translation[0]*step,
                      pivot[1] + sine*dx + cosine*dy + translation[1]*step,
                      pivot[2]+(point[2]-pivot[2])*factor + translation[2]*step]
            if grid:
                target[:2] = [round(value / grid) * grid for value in target[:2]]
            cell = int(source[3], 16)
            if all(outdoor):
                cell, target = placement_from_point(reference, target, landscape)
            row = list(source)
            row[3] = f"{cell:08x}"
            row[4:7] = [native_number(value) for value in target]
            if degrees:
                w, x, y, z = (float(value) for value in source[7:11])
                row[7:11] = [native_number(value) for value in (
                    half_w*w - half_z*z, half_w*x - half_z*y,
                    half_w*y + half_z*x, half_w*z + half_z*w)]
            if scale != 1:
                current = float(source[11]) if len(source) == 12 and source[11] != "-" else 1
                if not 0 < current*factor <= 100:
                    raise ValueError("An object's resulting scale exceeds the native range")
                if len(row) == 11:
                    row.append(native_number(current*factor))
                else:
                    row[11] = native_number(current*factor)
            result.append(row)
    return result


def prepare_arrangement(original, identities, game_folder, *, translation=(0, 0, 0), degrees=0, copies=0, grid=0, scale=1):
    """Prepare one complete file edit, with no writes to the user's content."""
    rows = rows_from_text(original)
    identities = set(identities)
    selected = [row for row in rows if int(row[0], 16) in identities]
    if len(selected) != len(identities):
        raise ValueError("One or more selected objects no longer exist")
    outdoor = any(1 <= (int(row[3], 16) & 65535) <= 64 for row in selected)
    landscape = landscape_grid(game_folder) if outdoor else None
    changed = arrangement_rows(selected, translation, degrees, copies, grid, landscape, scale)
    lines = original.splitlines()
    if lines[0] == "pyrealium-world-v2" and any(len(row) == 12 for row in changed):
        lines = ["pyrealium-world-v3"] + [line+"\t-" if line and not line.startswith("#") else line for line in lines[1:]]
    if lines[0] == "pyrealium-world-v3":
        changed = [row+["-"] if len(row) == 11 else row for row in changed]
    if copies:
        occupied = {int(row[0], 16) for row in rows}
        new_ids = []
        candidate = 0x8140000000020000
        for row in changed:
            while candidate in occupied:
                candidate += 1
            if candidate >= 0x8140000000030000:
                raise ValueError("Operator placement identity range exhausted")
            row[0] = f"{candidate:016x}"
            new_ids.append(candidate)
            occupied.add(candidate)
        lines += ["# Operator-authored arrangement copies"] + ["\t".join(row) for row in changed]
    else:
        replacements = {int(row[0], 16): "\t".join(row) for row in changed}
        lines = [replacements.get(int(line.split("\t", 1)[0], 16), line)
                 if line and not line.startswith("#") and i else line for i, line in enumerate(lines)]
        new_ids = [int(row[0], 16) for row in changed]
    result = "\n".join(lines) + "\n"
    rows_from_text(result)  # Check total/per-cell limits before any DAT reads or commit.
    with Dat(Path(game_folder) / "portal.dat") as portal, Dat(Path(game_folder) / "cell_1.dat") as cells:
        checked_assets, checked_cells = set(), set()
        for row in changed:
            for value in row[1:3]:
                identity = int(value, 16)
                if identity not in checked_assets:
                    portal.read(identity)
                    checked_assets.add(identity)
            cell = int(row[3], 16)
            record = cell | 65535 if 1 <= (cell & 65535) <= 64 else cell
            if record not in checked_cells:
                cells.read(record)
                checked_cells.add(record)
    return result, new_ids
