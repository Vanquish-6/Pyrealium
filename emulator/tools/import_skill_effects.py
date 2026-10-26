"""Extract active-skill use data from portal.dat skill state records.

Field layout follows the 2012 Pyreal server (SkillCollection.ParseSkills,
SkillEffectFile, SkillEffectPlayerFile). Records are walked the same way:
each 0x5c000100 / 0x44000100 / 0x24000100 marker is one graph object, objects
are numbered from the root (0), and root fields name other objects by number.

Root fields used: 23 kind, 30 right-hand implement mask, 34 reset seconds
(float), 36 self-effect object, 43 story behavior, 44 valid targets,
46 character behavior, 49 left-hand implement mask, 58 target-effect object,
60 vigor cost (int), 65 animation object, 69 max range (float).
Animation sub-objects are nodes whose values are numbered from 1: value 13
counts a hit, 16 is the FX, 32 the value-table object.
Value rows are (value float, level float). Effect records: field 0 duration
table, 1 FX, 2 period seconds, 3 effect type, 5 effect class, 37 value table;
self-effect records use field 36 and may reference further effect records.
Buff records list modified stat ids (node kind ffff000a) followed by their
multiplier table. Recall effects name a telepad in an encrypted string node
(kind ffff0076), resolved against the telepad table.
"""
import argparse
import hashlib
from pathlib import Path
import struct

from ac2_dat import Dat
from import_portal_routes import telepads

ROOT = 0x5C000100
NODE = 0x44000100
REF = 0x24000100
STATS = 0xFFFF000A
LIST = 0xFFFF0061
TEXT = 0xFFFF0076
ROW = 0x31ED0000
KEYS = (
    0xE5849238, 0xEB2D767D, 0xFC0FDD03, 0xCB45A6A6,
    0x7F21E6BB, 0xF5A6CDC8, 0x22D9AD12, 0xB2A3D4DE,
    0x3B834DB7, 0xC406C91C, 0x8392305E, 0x0A9F0099,
    0x0943B90C, 0xA66D953C, 0x0D929E1F, 0x0E2E9AD4,
)
LIFESTONE_RECALL = 0x139
ALLEGIANCE_RECALL = 0x134E
PORTAL_RECALL = 0x44D
SUMMON_PORTAL = 0x44F


def words(data):
    return struct.unpack_from(f"<{len(data) // 4}I", data, 0)


def floating(word):
    return struct.unpack("<f", struct.pack("<I", word))[0]


def objects(data):
    """Return (marker, word offset) for each graph object, in stream order."""
    values = words(data)
    count = values[2]
    found = []
    index = 3 + count
    while index < len(values):
        marker = values[index]
        if marker in (ROOT, NODE, REF):
            found.append((marker, index))
        index += 1
    return values, found


def struct_fields(values, offset):
    count = values[offset + 3]
    if count > 512 or offset + 4 + count > len(values):
        raise ValueError("struct field bound")
    return list(values[offset + 4:offset + 4 + count])


def list_items(values, offset):
    count = values[offset + 3] & 0xFFFFFF
    if count > 4096 or offset + 4 + count > len(values):
        raise ValueError("list bound")
    return list(values[offset + 4:offset + 4 + count])


def value_rows(values, graph, table):
    rows = []
    if table is None or table >= len(graph) or graph[table][0] != NODE:
        return rows
    for item in list_items(values, graph[table][1]):
        if item >= len(graph) or graph[item][0] != NODE:
            continue
        offset = graph[item][1]
        value, level = floating(values[offset + 4]), floating(values[offset + 5])
        if all(abs(x) < 1e7 for x in (value, level)):
            rows.append((round(level, 3), round(value, 3)))
    return sorted(set(rows))


def decrypt(data, real_len):
    out = bytearray(real_len)
    odd = real_len % 4
    c = 0
    for j, i in enumerate(range(0, real_len - odd, 4)):
        struct.pack_into("<I", out, c, struct.unpack_from("<I", data, i)[0] ^ KEYS[j % 16])
        c += 4
    out[c:real_len] = data[real_len - odd:real_len]
    return bytes(((b >> 4) | (b << 4)) & 0xFF for b in out)


def texts(data, values, graph):
    found = []
    for marker, offset in graph:
        if marker == NODE and values[offset + 2] == TEXT:
            start = (offset + 3) * 4
            length = struct.unpack_from("<H", data, start)[0]
            raw = data[start + 2:start + 2 + length * 2]
            if length and len(raw) == length * 2:
                found.append(decrypt(raw, length * 2).decode("utf-16-le", "replace"))
    return found


def stat_tables(values, graph):
    """Pair each modified-stat list with the fullest multiplier table after it.

    A stat list is followed by a one-row default (1 at level 0) and then the
    per-level magnitude table; the table with the most rows is kept.
    """
    best, pending = {}, []
    for marker, offset in graph:
        if marker != NODE:
            continue
        kind = values[offset + 2]
        if kind == STATS:
            pending = list_items(values, offset)
        elif kind == LIST and pending:
            rows = []
            for item in list_items(values, offset):
                if item < len(graph) and graph[item][0] == NODE and values[graph[item][1] + 2] == ROW:
                    o = graph[item][1]
                    rows.append((round(floating(values[o + 5]), 3), round(floating(values[o + 4]), 3)))
            rows = sorted(set(rows))
            for stat in pending:
                if rows and len(rows) > len(best.get(stat, [])):
                    best[stat] = rows
    return sorted(best.items())


def effect_record(portal, did, seen, own_field=37):
    if did in seen or did not in portal.entries:
        return []
    seen.add(did)
    data = portal.read(did)
    values, graph = objects(data)
    if not graph or graph[0][0] != ROOT:
        return []
    fields = struct_fields(values, graph[0][1])
    get = lambda index: fields[index] if index < len(fields) else None
    effect = {
        "record": did,
        "fx": get(1) or 0,
        "type": get(3) or 0,
        "class": get(5) or 0,
        "period": round(floating(get(2) or 0), 3),
        "duration": value_rows(values, graph, get(0)),
        "values": value_rows(values, graph, get(37 if own_field == 37 else 36)),
        "stats": stat_tables(values, graph),
        "telepad": next(iter(texts(data, values, graph)), ""),
    }
    out = [effect]
    if own_field != 37:
        table = get(36)
        if table is not None and table < len(graph) and graph[table][0] == NODE:
            for item in list_items(values, graph[table][1]):
                if item < len(graph) and graph[item][0] == REF:
                    out.extend(effect_record(portal, values[graph[item][1] + 1], seen))
    return out


def skill(portal, did):
    data = portal.read(did)
    values, graph = objects(data)
    if not graph or graph[0][0] != ROOT:
        raise ValueError("no root object")
    fields = struct_fields(values, graph[0][1])
    if len(fields) < 70:
        raise ValueError("short root")
    get = lambda index: fields[index] if index < len(fields) else None
    result = {
        "skill": fields[0], "record": did, "kind": get(23),
        "reset": round(floating(get(34)), 3), "vigor": struct.unpack("<i", struct.pack("<I", get(60)))[0],
        "range": round(floating(get(69)), 3), "targets": get(44), "right": get(30), "left": get(49),
        "behavior": get(46), "story": get(43), "hits": 0, "fx": 0, "values": [], "effects": [],
    }

    def refs(index):
        """Object number -> list of external DIDs listed by that object."""
        if index is None or index >= len(graph) or graph[index][0] != NODE:
            return []
        dids = []
        for item in list_items(values, graph[index][1]):
            if item < len(graph) and graph[item][0] == REF:
                dids.append(values[graph[item][1] + 1])
        return dids

    animation = get(65)
    if animation is not None and animation < len(graph) and graph[animation][0] == NODE:
        for item in list_items(values, graph[animation][1]):
            if item >= len(graph) or graph[item][0] != NODE:
                continue
            # Node values are numbered from 1 in the 2012 parser.
            sub = [None] + list_items(values, graph[item][1])
            if len(sub) > 13:
                result["hits"] += 1
            if len(sub) > 16 and sub[16] != 0xFFFFFFFF:
                result["fx"] = sub[16]
            if len(sub) > 32 and not result["values"]:
                result["values"] = value_rows(values, graph, sub[32])
    seen = set()
    for target in refs(get(58)):
        result["effects"].extend(("target", e) for e in effect_record(portal, target, seen))
    for own in refs(get(36)):
        result["effects"].extend(("self", e) for e in effect_record(portal, own, seen, own_field=36))
    return result


def table(rows):
    return ",".join(f"{level:g}:{value:g}" for level, value in rows) or "-"


def effect_text(side, e):
    stats = "|".join(f"{stat:x}={table(rows)}" for stat, rows in e["stats"]) or "-"
    return "/".join([
        side, f"{e['record']:08x}", f"{e['type']:x}", f"{e['class']:x}", f"{e['fx']:08x}",
        f"{e['period']:g}", table(e["duration"]), table(e["values"]), stats, e["telepad"] or "-",
    ])


def destinations(pads, prefix):
    found = []
    for identity, pad in sorted(pads.items()):
        label = pad["label"].rstrip(":")
        if label.startswith(prefix) and "error" not in pad and pad["active"] == 1:
            x, y, z = pad["position"]
            found.append(f"{pad['cell']:08x}:{x:g}:{y:g}:{z:g}:{pad['heading']}")
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game-data", type=Path, required=True)
    parser.add_argument("--telepads", type=Path, required=True)
    parser.add_argument("--catalog", type=Path, default=Path(__file__).resolve().parents[1] / "content" / "skills.tsv")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--travel", type=Path, required=True)
    args = parser.parse_args()
    catalog = [line.split("\t") for line in args.catalog.read_text(encoding="utf-8").splitlines()
               if line and not line.startswith(("#", "pyrealium"))]
    pads, pad_hash = telepads(args.telepads)
    out = [
        "pyrealium-skill-effects-v2",
        "# Active-skill use data read from portal.dat skill state records.",
        "# skill record kind reset_s vigor range targets right left story fx damage_table effects",
        "# Tables are level:value pairs. Effects are separated by ';' with fields",
        "# side/record/type/class/fx/period_s/duration_table/value_table/stats/telepad;",
        "# stats are stat=table pairs separated by '|'.",
    ]
    travel = [
        "pyrealium-skill-travel-v1",
        f"# Recall destinations from the telepad table (SHA256 {pad_hash}).",
        "# skill class destinations (cell:x:y:z:heading separated by ';', or lifestone/portal)",
    ]
    failed = 0
    with Dat(args.game_data / "portal.dat") as portal:
        portal.index()
        out.insert(2, "# portal.dat skill index SHA256: " + hashlib.sha256(portal.read(0x700016E2)).hexdigest())
        for row in catalog:
            did = int(row[1], 16)
            try:
                s = skill(portal, did)
            except (ValueError, KeyError, struct.error):
                failed += 1
                continue
            effects = ";".join(effect_text(side, e) for side, e in s["effects"]) or "-"
            out.append("\t".join([
                f"{s['skill']:x}", f"{did:08x}", f"{s['kind']:x}", f"{s['reset']:g}", str(s["vigor"]),
                f"{s['range']:g}", f"{s['targets']:x}", f"{s['right']:x}", f"{s['left']:x}",
                f"{s['story']:08x}", f"{s['fx']:08x}", table(s["values"]), effects,
            ]))
            for _side, e in s["effects"]:
                where = []
                if e["class"] == LIFESTONE_RECALL:
                    where = ["lifestone"]
                elif e["class"] == PORTAL_RECALL:
                    where = ["portal"]
                elif e["class"] == ALLEGIANCE_RECALL:
                    where = destinations(pads, "dng_hum_allegiancerecall")
                elif e["telepad"]:
                    name = e["telepad"].rstrip("0123456789")
                    where = destinations(pads, name)
                if where:
                    travel.append(f"{s['skill']:x}\t{e['class']:x}\t{';'.join(where)}")
                    break
    args.output.write_text("\n".join(out) + "\n", encoding="utf-8", newline="\n")
    args.travel.write_text("\n".join(travel) + "\n", encoding="utf-8", newline="\n")
    print(f"skills {len(catalog)} written {len(out) - 7} failed {failed} travel {len(travel) - 3}")


if __name__ == "__main__":
    main()
