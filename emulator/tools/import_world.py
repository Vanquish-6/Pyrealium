"""Import supplied portal and lifestone placements with pinned DAT layouts.

No SQL is executed. Unresolved portal cells are reported separately and require
an explicit --allow-unresolved switch to produce a partial placement manifest.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import re
import struct

from ac2_dat import Dat
from import_lifestones import convert as lifestones

# entity: (qualities hash, physical-DID offset, physical DID, visual DID)
PROFILES = {
    0x4700084d: ("601e45d1df0ad3e5d183a9b60dd4b0b4afbdda1b03e7a51d8d47b2a494fedfe8",156,0x47000297,0x1f00031e),
    0x47001451: ("64f930f29d25369961975c103c99ad0d5d1076331f541fc0e463fffb9b470e8b",156,0x47000297,0x1f00031e),
    0x47001454: ("960950a46cba000ddec49e91aa8005f5f604e8bb345e65d3c4e953b78b62c378",156,0x47001445,0x1f000d09),
    0x4700369a: ("e1f0fa009fec9975e49ccb9894403f6774cdffd122ec71577844ca5929e1273b",208,0x470035a9,0x1f00191d),
}
COLUMNS = ("EntityFileId", "TelepadId", "NameStringFile", "NameStringToken",
           "ExamineStringFile", "ExamineStringToken", "LevelRequired", "HeroRequirement",
           "KingdomRequirement", "Summonable", "LandCell", "LandLayer", "LandBlock",
           "LandX", "LandY", "LandZ", "QuatW", "QuatX", "QuatY", "QuatZ")


def convert(source, stone_source, game_data, allow_unresolved):
    with source.open("rb") as file:
        raw = file.read(1024 * 1024 + 1)
    if len(raw) > 1024 * 1024:
        raise ValueError("portal table exceeds 1 MiB")
    stones, stone_count = lifestones(stone_source, game_data)
    output = ["pyrealium-world-v2", "# Supplied stationary lifestone and portal placements.",
              "# Portal table SHA256: " + hashlib.sha256(raw).hexdigest()]
    output.extend(line for line in stones.splitlines() if line.startswith("#") and not line.startswith("# IID"))
    output.append("# IID\tentity\tvisual\tcell\tX\tY\tZ\tW\tQX\tQY\tQZ")
    for line in stones.splitlines():
        if line.startswith("#") or line == "pyrealium-world-v1":
            continue
        fields = line.split("\t")
        if fields[-1] != "0":
            raise ValueError("unexpected lifestone heading")
        output.append("\t".join(fields[:7] + ["1", "0", "0", "0"]))
    metadata = []
    checked = set()
    positions = set()
    with Dat(game_data / "portal.dat") as portal, Dat(game_data / "cell_1.dat") as cells:
        cell_index = cells.index()
        for line in raw.decode("utf-8-sig").splitlines():
            if not line.startswith("INSERT"):
                continue
            match = re.fullmatch(r"INSERT INTO `portal` \(([^)]+)\) VALUES \(([^)]+)\);", line)
            if not match or tuple(v.strip("`") for v in match[1].split(",")) != COLUMNS:
                raise ValueError("unsupported portal table statement or columns")
            fields = match[2].split(",")
            if len(fields) != 20:
                raise ValueError("portal field count")
            entity, telepad = map(int, fields[:2])
            cell, layer, block = map(int, fields[10:13])
            if not 0 <= cell <= 255 or not 0 <= layer <= 255 or not -32768 <= block <= 65535:
                raise ValueError("invalid cell components")
            full_cell = ((block & 65535) << 16) | (layer << 8) | cell
            record = full_cell | 65535 if 1 <= full_cell & 65535 <= 64 else full_cell
            if not (1 <= full_cell & 65535 <= 64 or 256 <= full_cell & 65535 <= 65531):
                raise ValueError("reserved or invalid portal cell")
            if telepad <= 0 or any(not math.isfinite(float(v)) or abs(float(v)) >= 1_000_000 for v in fields[13:20]):
                raise ValueError("invalid portal target or coordinates")
            if abs(sum(float(v)**2 for v in fields[16:20])-1) > 0.01:
                raise ValueError("portal quaternion is not normalized")
            if entity not in PROFILES:
                raise ValueError(f"unsupported portal definition {entity:#x}")
            expected_hash, offset, physical, visual = PROFILES[entity]
            if entity not in checked:
                portal.read(entity)
                qualities = portal.read(0x81000000 | (entity & 0xffffff))
                if hashlib.sha256(qualities).hexdigest() != expected_hash or struct.unpack_from("<I", qualities, offset)[0] != physical:
                    raise ValueError("unknown portal qualities layout")
                physical_bytes = portal.read(physical)
                if len(physical_bytes) != 16 or struct.unpack("<4I", physical_bytes) != (11, physical, 0x40000001, visual):
                    raise ValueError("unknown portal physical/visual layout")
                portal.read(visual)
                checked.add(entity)
            position = (full_cell, *map(float, fields[13:16]))
            if position in positions:
                raise ValueError("duplicate portal placement")
            positions.add(position)
            # Snapshot row ordinals are stable only for this source snapshot.
            identity = 0x8140000000010000 | (len(metadata)+1)
            placed = record in cell_index
            entry = {"id": f"{identity:016x}", "entity": f"{entity:08x}", "telepad": telepad,
                     "cell": f"{full_cell:08x}", "position": list(map(float, fields[13:16])),
                     "orientation": list(map(float, fields[16:20])),
                     "name": [int(v) for v in fields[2:4]],
                     "examine": [None if v == "NULL" else int(v) for v in fields[4:6]],
                     "requirements": dict(zip(["level", "hero", "kingdom", "summonable"], map(int,fields[6:10]))),
                     "status": "placed" if placed else "missing-cell"}
            metadata.append(entry)
            if placed:
                cells.read(record)
                output.append("\t".join([f"{identity:016x}", f"{entity:08x}", f"{visual:08x}",
                                         f"{full_cell:08x}", *fields[13:20]]))
    if not metadata:
        raise ValueError("no portal placements")
    missing = sum(entry["status"] != "placed" for entry in metadata)
    if missing and not allow_unresolved:
        raise ValueError(f"{missing} portal cells are missing; use --allow-unresolved to retain them in the report only")
    report = {"source_sha256": hashlib.sha256(raw).hexdigest(), "lifestones": stone_count,
              "placed_portals": len(metadata)-missing, "unresolved_portals": missing,
              "portal_interactions_implemented": False, "portals": metadata}
    return "\n".join(output)+"\n", report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--portals", type=Path, required=True)
    parser.add_argument("--lifestones", type=Path, required=True)
    parser.add_argument("--game-data", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--allow-unresolved", action="store_true")
    args = parser.parse_args()
    result, report = convert(args.portals, args.lifestones, args.game_data, args.allow_unresolved)
    for path, text in [(args.output,result), (args.report,json.dumps(report,indent=2)+"\n")]:
        path.parent.mkdir(parents=True,exist_ok=True)
        temporary = path.with_name(path.name+".tmp")
        temporary.write_text(text,encoding="utf-8",newline="\n")
        temporary.replace(path)
    print(f"Imported {report['lifestones']} lifestones and {report['placed_portals']} portals; {report['unresolved_portals']} portals retained as unresolved.")


if __name__ == "__main__":
    main()
