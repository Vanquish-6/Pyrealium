"""Extract the supplied Entity worksheet into an asset-backed definition catalog.

Requires xlrd for legacy XLS reading. The source workbook is never modified.
Workbook labels are annotations, not authoritative in-game names or behavior.
"""
import argparse
import hashlib
import json
from pathlib import Path

from ac2_dat import Dat


def identifier(value):
    # Excel stored some all-digit hexadecimal identifiers as numeric cells.
    if isinstance(value, float):
        if not value.is_integer():
            raise ValueError("fractional identifier")
        value = str(int(value))
    return int(str(value).strip().removeprefix("0x"), 16)


def convert(source, game_data):
    import xlrd
    if source.stat().st_size > 8 * 1024 * 1024:
        raise ValueError("source workbook exceeds 8 MiB")
    book = xlrd.open_workbook(source, on_demand=True)
    try:
        sheet = book.sheet_by_name("Entity")
        if sheet.nrows > 32769 or sheet.ncols != 4 or sheet.row_values(0)[:3] != ["Entity", "Name", "Vdesc"]:
            raise ValueError("unsupported Entity worksheet layout")
        accepted = {}
        omitted = []
        with Dat(game_data / "portal.dat") as portal:
            assets = portal.index()
            for row in range(1, sheet.nrows):
                entity, label, visual, _ = sheet.row_values(row)
                if visual == "":
                    continue
                try:
                    entity_id, visual_id = identifier(entity), identifier(visual)
                    if entity_id >> 24 != 0x47 or visual_id >> 24 != 0x1f:
                        raise ValueError("expected full entity/visual DIDs")
                    if entity_id not in assets or visual_id not in assets:
                        raise ValueError("missing DAT asset")
                    label = str(label).strip()
                    if len(label.encode("utf-8")) > 256 or any(ord(c)<32 for c in label):
                        raise ValueError("unsupported label")
                    if entity_id in accepted:
                        raise ValueError("duplicate entity definition")
                    accepted[entity_id] = (visual_id,label)
                except (ValueError, TypeError) as error:
                    omitted.append({"row":row+1,"entity":str(entity),"visual":str(visual),"reason":str(error)})
    finally:
        book.release_resources()
    source_hash = hashlib.sha256(source.read_bytes()).hexdigest()
    output = ["pyrealium-definitions-v1", "# Source workbook SHA256: " + source_hash,
              "# Worksheet: Entity; labels retain source annotations.",
              "# Catalog checks asset presence, not original placement, NPC behavior or balance."]
    output.extend(f"{entity:08x}\t{visual:08x}\t{label}" for entity,(visual,label) in sorted(accepted.items()))
    return "\n".join(output)+"\n", {"source_sha256":source_hash,"definitions":len(accepted),"unresolved":omitted}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source",type=Path,required=True)
    parser.add_argument("--game-data",type=Path,required=True)
    parser.add_argument("--output",type=Path,required=True)
    parser.add_argument("--report",type=Path,required=True)
    args = parser.parse_args()
    result, report = convert(args.source,args.game_data)
    for path,text in [(args.output,result),(args.report,json.dumps(report,indent=2)+"\n")]:
        path.parent.mkdir(parents=True,exist_ok=True)
        temporary = path.with_name(path.name+".tmp")
        temporary.write_text(text,encoding="utf-8",newline="\n")
        temporary.replace(path)
    print(f"Imported {report['definitions']} definitions; {len(report['unresolved'])} unresolved visual mappings retained in report.")


if __name__ == "__main__":
    main()
