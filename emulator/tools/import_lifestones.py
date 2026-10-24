"""Convert the supplied numeric lifestone placement table without executing SQL.

Only a pinned lifestone qualities/physics layout is supported. This imports
placement and visuals, not binding, eligibility, resurrection or summon rules.
"""
import argparse
import hashlib
import math
from pathlib import Path
import re
import struct

from ac2_dat import Dat

QUALITIES_SHA = "9a8e5913724fa04f601f5281ce096f8f6a01c384eeb946db3d125ed89a16d7e5"
COLUMNS = ("idLifestone", "EntityFileId", "ExamineStringFile", "ExamineStringToken",
           "LevelRequired", "HeroRequirement", "KingdomRequirement", "Summonable",
           "LandCell", "LandLayer", "LandBlock", "LandX", "LandY", "LandZ",
           "QuatW", "QuatX", "QuatY", "QuatZ")


def convert(source, game_data):
    with source.open("rb") as file:
        raw = file.read(1024 * 1024 + 1)
    if len(raw) > 1024 * 1024:
        raise ValueError("placement table exceeds 1 MiB")
    rows = []
    identities = set()
    for line in raw.decode("utf-8-sig").splitlines():
        if not line.startswith("INSERT"):
            continue
        match = re.fullmatch(r"INSERT INTO `lifestone` \(([^)]+)\) VALUES \(([^)]+)\);", line)
        if not match or tuple(part.strip("`") for part in match[1].split(",")) != COLUMNS:
            raise ValueError("unsupported placement-table statement or columns")
        values = match[2].split(",")
        if len(values) != 18:
            raise ValueError("unsupported placement row")
        identity, entity = map(int, values[:2])
        cell, layer, block = map(int, values[8:11])
        if (not 0 < identity < 65536 or identity in identities or entity != 0x470007de
                or not 1 <= cell <= 64 or layer != 0 or not -32768 <= block <= 65535):
            raise ValueError("unsupported/duplicate placement identity, definition or outdoor cell")
        if tuple(map(float, values[14:18])) != (1.0, 0.0, 0.0, 0.0):
            raise ValueError("unsupported lifestone rotation")
        if any(not math.isfinite(float(v)) or abs(float(v)) >= 1_000_000 for v in values[11:14]):
            raise ValueError("invalid lifestone coordinates")
        identities.add(identity)
        rows.append((identity, ((block & 0xffff) << 16) | cell, values[11:14]))
    if not rows:
        raise ValueError("no supported lifestone placements")
    with Dat(game_data / "portal.dat") as portal, Dat(game_data / "cell_1.dat") as cells:
        portal.read(0x470007de)
        qualities = portal.read(0x810007de)
        if hashlib.sha256(qualities).hexdigest() != QUALITIES_SHA:
            raise ValueError("unknown lifestone qualities layout")
        physics = struct.unpack_from("<I", qualities, 236)[0]
        if physics != 0x47000075:
            raise ValueError("unexpected pinned physics definition")
        physical = portal.read(physics)
        if len(physical) != 16 or struct.unpack("<4I", physical) != (11, physics, 0x40000001, 0x1f00002d):
            raise ValueError("unknown lifestone physical/visual layout")
        portal.read(0x1f00002d)
        for record in sorted({cell | 0xffff for _, cell, _ in rows}):
            cells.read(record)
    output = ["pyrealium-world-v1",
              "# Supplied lifestone placement snapshot; stationary presentation only.",
              "# Source table SHA256: " + hashlib.sha256(raw).hexdigest(),
              "# Qualities SHA256: " + QUALITIES_SHA,
              "# IID\tentity\tvisual\tcell\tX\tY\tZ\theading"]
    for identity, cell, coordinates in sorted(rows):
        output.append("\t".join([f"{0x8140000000000000 | identity:016x}", "470007de",
                                "1f00002d", f"{cell:08x}", *coordinates, "0"]))
    return "\n".join(output) + "\n", len(rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--game-data", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result, count = convert(args.source, args.game_data)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(result, encoding="utf-8", newline="\n")
    print(f"Imported {count} stationary lifestone placements into {args.output}")


if __name__ == "__main__":
    main()
