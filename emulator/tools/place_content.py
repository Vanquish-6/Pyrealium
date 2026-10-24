"""Author one explicit stationary placement from the definition catalog.

This is an offline content-authoring tool. It does not invent historical spawn
positions, implement NPC behavior, or modify a running server's world.
"""
import argparse
import math
from pathlib import Path
import struct

from ac2_dat import Dat


def bounded_text(path):
    with path.open("rb") as file:
        data = file.read(4*1024*1024+1)
    if len(data) > 4*1024*1024:
        raise ValueError("content file exceeds 4 MiB")
    return data.decode("utf-8")


def place(args):
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
    text = bounded_text(args.output) if args.output.exists() else "pyrealium-world-v2\n"
    lines = text.splitlines()
    if not lines or lines[0] != "pyrealium-world-v2":
        raise ValueError("authoring requires a quaternion world-v2 manifest")
    occupied = set()
    count = 0
    same_cell = 0
    for line in lines[1:]:
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields) != 11:
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
    half = args.heading*math.pi/255
    row = "\t".join([f"{identity:016x}",f"{args.entity:08x}",f"{visual:08x}",f"{args.cell:08x}",
                     *(format(v,'.9g') for v in position),
                     *(format(v,'.9g') for v in [math.cos(half),0,0,-math.sin(half)])])
    result = text.rstrip("\n")+"\n# Operator-authored placement: "+label+"\n"+row+"\n"
    if len(result.encode("utf-8")) > 4*1024*1024:
        raise ValueError("result exceeds content file limit")
    args.output.parent.mkdir(parents=True,exist_ok=True)
    temporary = args.output.with_name(args.output.name+".tmp")
    temporary.write_text(result,encoding="utf-8",newline="\n")
    temporary.replace(args.output)
    print(f"Authored stationary placement {identity:016x}: {label}. Restart/reload requires the normal server lifecycle.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--catalog",type=Path,required=True)
    parser.add_argument("--game-data",type=Path,required=True)
    parser.add_argument("--output",type=Path,required=True)
    parser.add_argument("--entity",type=lambda v:int(v,16),required=True)
    parser.add_argument("--cell",type=lambda v:int(v,16),required=True)
    parser.add_argument("--position",type=float,nargs=3,required=True)
    parser.add_argument("--heading",type=int,default=0)
    place(parser.parse_args())


if __name__ == "__main__":
    main()
