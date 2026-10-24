"""Author one explicit stationary placement from the definition catalog.

This is an offline content-authoring tool. It does not invent historical spawn
positions, implement NPC behavior, or modify a running server's world.
"""
import argparse
import math
import os
from pathlib import Path
import struct
import tempfile

from ac2_dat import Dat


def bounded_text(path):
    with path.open("rb") as file:
        data = file.read(4*1024*1024+1)
    if len(data) > 4*1024*1024:
        raise ValueError("content file exceeds 4 MiB")
    return data.decode("utf-8")


def place(args):
    args.output.parent.mkdir(parents=True, exist_ok=True)
    lock = args.output.with_name(args.output.name + ".edit.lock")
    # Cooperating authoring processes must not overwrite each other's additions.
    with lock.open("x", encoding="utf-8") as guard:
        try:
            guard.write(str(os.getpid()))
            guard.flush()
            return _place(args)
        finally:
            guard.close()
            lock.unlink()


def _place(args):
    catalog = bounded_text(args.catalog).splitlines()
    if not catalog or catalog[0] != "pyrealium-definitions-v1":
        raise ValueError("unsupported definition catalog")
    matches = []
    for line in catalog[1:]:
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 3:
            raise ValueError("invalid catalog record")
        if int(fields[0],16) == args.entity:
            matches.append((int(fields[1],16),fields[2]))
    if len(matches) != 1:
        raise ValueError("definition is missing or ambiguous")
    visual,label = matches[0]
    if args.entity >> 24 != 0x47 or visual >> 24 != 0x1f:
        raise ValueError("unsupported entity or visual identifier")
    if any(not math.isfinite(v) or abs(v)>=1_000_000 for v in args.position):
        raise ValueError("invalid position")
    position = [struct.unpack("<f",struct.pack("<f",v))[0] for v in args.position]
    if any(abs(v)>=1_000_000 for v in position):
        raise ValueError("position rounds outside the supported native range")
    if not 0<=args.heading<=255:
        raise ValueError("heading must be 0..255")
    low = args.cell & 65535
    if not 0 < args.cell <= 0xffffffff or not (1<=low<=64 or 256<=low<=65531):
        raise ValueError("invalid cell identifier")
    with Dat(args.game_data/'portal.dat') as portal, Dat(args.game_data/'cell_1.dat') as cells:
        portal.read(args.entity)
        portal.read(visual)
        cells.read(args.cell|65535 if low<=64 else args.cell)
    original = bounded_text(args.output) if args.output.exists() else None
    text = original if original is not None else "pyrealium-world-v2\n"
    lines = text.splitlines()
    if not lines or lines[0] not in ("pyrealium-world-v2", "pyrealium-world-v3"):
        raise ValueError("authoring requires a quaternion world-v2 or world-v3 manifest")
    version = int(lines[0][-1])
    scale = getattr(args, "scale", None)
    if scale is not None:
        scale = struct.unpack("<f", struct.pack("<f", scale))[0]
        if not math.isfinite(scale) or not 0 < scale <= 100:
            raise ValueError("scale must be greater than zero and at most 100")
    occupied = set()
    count = 0
    same_cell = 0
    for line in lines[1:]:
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != (11 if version == 2 else 12):
            raise ValueError("invalid existing placement")
        identity = int(fields[0],16)
        if identity in occupied or identity>>48 != 0x8140:
            raise ValueError("invalid or duplicate existing IID")
        occupied.add(identity)
        count += 1
        same_cell += int(int(fields[3],16)==args.cell)
    if count>=16384 or same_cell>=64:
        raise ValueError("world/cell content limit reached")
    identity = 0x8140000000020000
    while identity in occupied:
        identity += 1
    if identity >= 0x8140000000030000:
        raise ValueError("operator placement identity range exhausted")
    half = args.heading*math.pi/255
    row = "\t".join([f"{identity:016x}",f"{args.entity:08x}",f"{visual:08x}",f"{args.cell:08x}",
                     *(format(v,'.9g') for v in position),
                     *(format(v,'.9g') for v in [math.cos(half),0,0,-math.sin(half)])])
    if version == 2 and scale is not None:
        lines = ["pyrealium-world-v3"] + [
            line + "\t-" if line and not line.startswith("#") else line
            for line in lines[1:]
        ]
        text = "\n".join(lines) + "\n"
        version = 3
    if version == 3:
        row += "\t" + ("-" if scale is None else format(scale, ".9g"))
    if len(row.encode("utf-8")) > 512:
        raise ValueError("placement exceeds native runtime line limit")
    result = text.rstrip("\n")+"\n# Operator-authored placement: "+label+"\n"+row+"\n"
    if len(result.encode("utf-8")) > 4*1024*1024:
        raise ValueError("result exceeds content file limit")
    args.output.parent.mkdir(parents=True,exist_ok=True)
    descriptor, name = tempfile.mkstemp(prefix=args.output.name+".", suffix=".tmp", dir=args.output.parent)
    temporary = Path(name)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8", newline="\n") as file:
            file.write(result)
            file.flush()
            os.fsync(file.fileno())
        current = bounded_text(args.output) if args.output.exists() else None
        if current != original:
            raise ValueError("manifest changed during authoring; reload it before saving")
        temporary.replace(args.output)
    finally:
        temporary.unlink(missing_ok=True)
    print(f"Authored stationary placement {identity:016x}: {label}. Restart/reload requires the normal server lifecycle.")
    return identity


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog",type=Path,required=True)
    parser.add_argument("--game-data",type=Path,required=True)
    parser.add_argument("--output",type=Path,required=True)
    parser.add_argument("--entity",type=lambda v:int(v,16),required=True)
    parser.add_argument("--cell",type=lambda v:int(v,16),required=True)
    parser.add_argument("--position",type=float,nargs=3,required=True)
    parser.add_argument("--heading",type=int,default=0)
    parser.add_argument("--scale",type=float,help="optional native scale; upgrades v2 to v3 without changing prior objects")
    place(parser.parse_args())


if __name__ == "__main__":
    main()
