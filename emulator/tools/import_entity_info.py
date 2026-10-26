"""Write content/entity-info.tsv: English default name and vendor flag for every
entity placed by the world content files.

The name is the definition's gmWeenieDesc name resolved through
local_English.dat (tools/ac2_strings.py). An entity is a vendor when its
portal.dat record or an inherited parent sets StoreGroup (0x410002EC).

Usage: import_entity_info.py GAME_DATA
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ac2_dat import Dat
from ac2_strings import Strings, entity_name_ref
from import_creature_combat import load_index
from import_observed_population import clean_name
from probe_entity_fields import read_all

ROOT = Path(__file__).resolve().parents[1] / "content"
SOURCES = ["world.tsv", "world-observed-expanded.tsv", "training-world.tsv", "capture-spawns.tsv"]
STORE_GROUP = 0x410002EC


def entities():
    found = set()
    for name in SOURCES:
        for line in (ROOT / name).read_text(encoding="utf-8").splitlines()[1:]:
            if line and not line.startswith("#"):
                found.add(int(line.split("\t")[1], 16))
    return sorted(found)


def main():
    game = Path(sys.argv[1])
    rows = []
    with Dat(game / "portal.dat") as portal, Dat(game / "local_English.dat") as local:
        assets = portal.index()
        local.index()
        types = load_index(portal)
        strings = Strings(local)
        for entity in entities():
            if entity not in assets:
                continue
            name = ""
            try:
                ref = entity_name_ref(portal, entity)
                if ref:
                    name = clean_name(strings.get(ref[1], ref[0]) or "")
            except Exception:
                pass
            try:
                vendor = STORE_GROUP in read_all(portal, types, entity)
            except Exception:
                vendor = False
            if name or vendor:
                rows.append(f"{entity:08x}\t{int(vendor)}\t{name or '-'}")
    lines = ["pyrealium-entity-info-v1",
             "# entity vendor name: English default name from local_English.dat and",
             "# StoreGroup (410002ec) presence from portal.dat, for placed entities."] + rows
    (ROOT / "entity-info.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"entities {len(rows)} vendors {sum(row.split(chr(9))[1] == '1' for row in rows)}")
    # Placed portals carry their own name ("Portal to ..."), [table, token] in portals.json.
    import json
    named = []
    with Dat(game / "local_English.dat") as local:
        local.index()
        strings = Strings(local)
        for portal in json.loads((ROOT / "portals.json").read_text(encoding="utf-8"))["portals"]:
            table, token = portal.get("name") or (0, 0)
            text = clean_name(strings.get(table, token) or "") if table else ""
            if text:
                named.append(f"{portal['id']}\t{text}")
    lines = ["pyrealium-object-names-v1",
             "# object name: per-placement names resolved from portals.json name references."] + named
    (ROOT / "object-names.tsv").write_text("\n".join(lines) + "\n", encoding="utf-8", newline="\n")
    print(f"named portals {len(named)}")


if __name__ == "__main__":
    main()
