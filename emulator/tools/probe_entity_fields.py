"""Print every portal.dat entity property token for the given 0x47 records.

Usage: probe_entity_fields.py PORTAL_DAT ENTITY_HEX [ENTITY_HEX ...]
Inherited values are merged parent-first, as in import_creature_combat.read_entity.
"""
import struct
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ac2_dat import Dat
from import_creature_combat import (FLOAT, INTEGER, LONG, BOOL, FILEREF, LABEL, Reader, load_index)


def read_all(archive, types, entity, stack=()):
    if entity in stack or len(stack) > 8:
        return {}
    reader = Reader(archive.read(entity))
    if reader.remaining() < 12:
        return {}
    file_type = reader.u32()
    reader.u32()
    flags = reader.u32()
    values = {}
    if flags == 0x40000007:
        values.update(read_all(archive, types, reader.u32(), stack + (entity,)))
    elif flags == 0x40000002:
        reader.u32()
    elif flags == 0x40000001:
        reader.u32()
        if file_type == 0x2B:
            reader.skip(12)
    else:
        return values
    if file_type != 0x8B:
        return values
    for _ in range(reader.u32()):
        reader.u32()
        for _ in range(reader.u32()):
            token = reader.u32()
            kind = types[token]
            if kind == FLOAT:
                values[token] = round(reader.f32(), 4)
            elif kind in (INTEGER, BOOL, FILEREF, LABEL):
                values[token] = f"{reader.u32():#x}"
            elif kind == LONG:
                values[token] = reader.u64()
            else:
                reader.skip_value(kind)
    return values


def main():
    with Dat(Path(sys.argv[1])) as archive:
        archive.index()
        types = load_index(archive)
        for text in sys.argv[2:]:
            entity = int(text, 16)
            values = read_all(archive, types, entity)
            print(f"== {entity:08x}")
            for token, value in sorted(values.items()):
                print(f"  {token:08x} {value}")


if __name__ == "__main__":
    main()
