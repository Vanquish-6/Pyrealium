"""Offline two-way dungeon entrances using native visual objects and server routes.

Portal travel is server-authored; these definitions do not alter cell geometry or
pretend that an opening in a room is a teleport. Record existence is checked by
the server against the exported DAT set, allowing newly built cells here.
"""
import math
import struct

PORTAL_ENTITY = 0x47001454
PORTAL_VISUAL = 0x1f000d09
NAMESPACE = 0x8140000000000000
FIRST_ID = NAMESPACE | 0x30000
LAST_ID = NAMESPACE | 0x40000


def endpoint(cell, position, heading=0):
    """Create a JSON-safe placement; heading is the native clockwise byte 0..255."""
    low = cell & 65535 if type(cell) is int else 0
    if type(cell) is not int or not 0 < cell <= 0xffffffff or not (1 <= low <= 64 or 0x100 <= low <= 0xfffb):
        raise ValueError("Choose an outdoor cell or dungeon room, not a terrain/header record")
    if type(heading) is not int or not 0 <= heading <= 255:
        raise ValueError("Heading must be a whole number from 0 to 255")
    if not isinstance(position, (list, tuple)) or len(position) != 3:
        raise ValueError("An entrance placement needs X, Y and Z")
    values = []
    for value in position:
        if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or abs(value) >= 1_000_000:
            raise ValueError("Entrance coordinates must be finite and within the native world")
        value = struct.unpack("<f", struct.pack("<f", value))[0]
        if abs(value) >= 1_000_000:
            raise ValueError("Entrance coordinates round outside the native world")
        values.append(value)
    return {"cell": cell, "position": values, "heading": heading}


def _endpoint(value):
    if not isinstance(value, dict) or set(value) != {"cell", "position", "heading"}:
        raise ValueError("Invalid entrance placement fields")
    return endpoint(value["cell"], value["position"], value["heading"])


def make_pair(existing_ids, outside_portal, inside_portal, inside_arrival, outside_arrival, *, min_level=1, name="Dungeon entrance"):
    """Return a JSON-safe entrance pair; all four placements are explicit.

    outside_portal/inside_portal place visible useable objects. inside_arrival
    and outside_arrival specify safe landing cells, coordinates and facing.
    Include all existing object IDs and previous entrance IDs in existing_ids.
    IDs are allocated deterministically in the separate entrance-authoring range.
    """
    occupied = set(existing_ids)
    available = []
    for identity in range(FIRST_ID, LAST_ID):
        if identity not in occupied:
            available.append(identity)
            if len(available) == 2:
                break
    if len(available) != 2:
        raise ValueError("Dungeon entrance object identity range is full")
    pair = {"format": "pyrealium-entrance-v1", "name": name,
            "entrance": {"id": available[0], "source": outside_portal, "destination": inside_arrival, "min_level": min_level},
            "return": {"id": available[1], "source": inside_portal, "destination": outside_arrival, "min_level": 1}}
    return validate_pair(pair)


def validate_pair(pair):
    if not isinstance(pair, dict) or set(pair) != {"format", "name", "entrance", "return"} or pair["format"] != "pyrealium-entrance-v1":
        raise ValueError("Unsupported dungeon entrance definition")
    name = pair["name"]
    if not isinstance(name, str) or not name.strip() or len(name) > 96 or any(ord(c) < 32 for c in name):
        raise ValueError("Entrance name must contain 1..96 printable characters")
    result = {"format": pair["format"], "name": name}
    identities = set()
    for key in ("entrance", "return"):
        route = pair[key]
        if not isinstance(route, dict) or set(route) != {"id", "source", "destination", "min_level"}:
            raise ValueError("Invalid entrance route fields")
        identity, level = route["id"], route["min_level"]
        if type(identity) is not int or not FIRST_ID <= identity < LAST_ID or identity in identities:
            raise ValueError("Invalid or repeated entrance object identity")
        if type(level) is not int or not 1 <= level <= 150:
            raise ValueError("Entrance minimum level must be 1..150")
        identities.add(identity)
        result[key] = {"id": identity, "source": _endpoint(route["source"]), "destination": _endpoint(route["destination"]), "min_level": level}
    # A return should always be available, even if entrance access is restricted.
    if result["return"]["min_level"] != 1:
        raise ValueError("A dungeon return portal must be available at level one")
    return result


def pair_rows(pair, version=3):
    """Return (world object rows, route rows), without file headers or writes."""
    pair = validate_pair(pair)
    if version not in (2, 3):
        raise ValueError("Entrance export needs world manifest version two or three")
    objects, routes = [], []
    for key in ("entrance", "return"):
        route = pair[key]
        source, destination = route["source"], route["destination"]
        half = source["heading"] * math.pi / 255
        identity = f'{route["id"]:016x}'
        objects.append("\t".join([identity, f"{PORTAL_ENTITY:08x}", f"{PORTAL_VISUAL:08x}", f'{source["cell"]:08x}',
            *(format(v, ".9g") for v in source["position"]), *(format(v, ".9g") for v in [math.cos(half), 0, 0, -math.sin(half)]),
            *(["-"] if version == 3 else [])]))
        routes.append("\t".join([identity, f"{PORTAL_ENTITY:08x}", f'{source["cell"]:08x}',
            *(format(v, ".9g") for v in source["position"]), f'{destination["cell"]:08x}',
            *(format(v, ".9g") for v in destination["position"]), str(destination["heading"]), str(route["min_level"])]))
    return objects, routes


def render_routes(pairs):
    """Serialize sidecar world.routes.tsv with duplicate and size guards."""
    if len(pairs) > 512:
        raise ValueError("A world may export at most 512 entrance pairs")
    rows, seen = [], set()
    for pair in pairs:
        _, routes = pair_rows(pair)
        for row in routes:
            identity = row.split("\t", 1)[0]
            if identity in seen:
                raise ValueError("Entrance pairs share an object identity")
            seen.add(identity)
            rows.append(row)
    return "pyrealium-portal-routes-v1\n" + "\n".join(rows) + ("\n" if rows else "")
