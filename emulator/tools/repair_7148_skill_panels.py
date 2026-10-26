"""Downgrade 7230 SkillUINode records to the 7148 layout in an isolated test DAT.

7148 SkillUINode::StreamPack 006079d0 stops after mUntrainable. The installed
7230 skill panels (0x232 objects listed by registry 0x70001976) append one more
u32 flag per node, written by script setter 0xae0019 that 7148 also lacks.
Unpacking them misaligns every following object and the skill window faults.
This removes only that flag word. Never modifies the game executable.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import struct

from ac2_dat import Dat
from make_7148_test_assets import locate, write_chain

REGISTRY = 0x70001976
NODE, NATIVE_NODE = 0x44000100, 0xFFFF00AE
MARKERS = (0x44000100, 0x24000100, 0x5C000100)


def panel_records(archive):
    data = archive.read(REGISTRY)
    words = struct.unpack_from(f'<{len(data) // 4}I', data)
    return sorted({w for w in words[1:] if w >> 24 == 0x70 and w != REGISTRY})


def object_starts(words):
    """Graph object boundaries: marker, zero, kind (kind high half 0xffff or a package)."""
    return [i for i in range(len(words) - 2)
            if words[i] in MARKERS and words[i + 1] == 0 and (words[i] == 0x24000100 or words[i + 2])]


def convert(data):
    if len(data) % 4 or len(data) < 12:
        raise ValueError('Unaligned panel record')
    words = list(struct.unpack_from(f'<{len(data) // 4}I', data))
    if words[1] != len(data) - 12:
        raise ValueError('Unexpected record length header')
    starts = object_starts(words)
    nodes = [i for i in starts if words[i] == NODE and words[i + 2] == NATIVE_NODE]
    hashes = [i for i in starts if words[i] == NODE and words[i + 2] == 0xFFFF0011]
    if not nodes:
        return None
    if not hashes or (words[hashes[0] + 3] & 0xFFFFFF) != len(nodes):
        raise ValueError('Node count does not match the panel node hash')
    remove = []
    for node in nodes:
        following = [s for s in starts if s > node]
        end = following[0] if following else len(words)
        tail = words[end - 4:end]
        if len(tail) != 4 or tail[3] != 0 or any(value > 1 for value in tail[:3]):
            raise ValueError(f'Unexpected SkillUINode tail at word {node}')
        remove.append(end - 2)
    for index in reversed(remove):
        del words[index]
    result = bytearray(struct.pack(f'<{len(words)}I', *words))
    struct.pack_into('<I', result, 4, len(result) - 12)
    return bytes(result)


def repair(directory):
    directory = Path(directory).resolve()
    manifest_path = directory / 'pyrealium-compatibility.json'
    manifest = json.loads(manifest_path.read_text())
    source = Path(manifest['source']).resolve()
    portal = directory / 'portal.dat'
    if directory == source or os.path.samefile(portal, source / 'portal.dat') or portal.stat().st_nlink != 1:
        raise ValueError('Repair requires the separate, non-hardlinked test portal.dat')
    backups = directory / 'skill-panels-before-repair'
    with Dat(source / 'portal.dat') as original_archive:
        original_archive.index()
        originals = {did: original_archive.read(did) for did in panel_records(original_archive)}
    converted = {}
    for did, data in originals.items():
        result = convert(data)
        if result is not None:
            converted[did] = result
    backups.mkdir(exist_ok=True)
    with Dat(portal) as archive, portal.open('r+b') as output:
        archive.index()
        for did, data in converted.items():
            current = archive.read(did)
            if current == data:
                continue
            if current != originals[did]:
                raise ValueError(f'{did:#x} differs from the installed source; refusing')
            backup = backups / f'{did:08x}.bin'
            if not backup.exists():
                with backup.open('xb') as handle:
                    handle.write(current)
                    handle.flush()
                    os.fsync(handle.fileno())
            start, length, _ = archive.entries[did]
            node, size_offset = locate(archive, did)
            write_chain(output, archive, start, 0, data + bytes(length - len(data)))
            write_chain(output, archive, node, size_offset, struct.pack('<I', len(data)))
        output.flush()
        os.fsync(output.fileno())
    with Dat(portal) as verified:
        verified.index()
        for did, data in converted.items():
            if verified.read(did) != data:
                raise ValueError(f'Read-back verification failed for {did:#x}')
    manifest['skill_panel_repair'] = {
        'registry': f'{REGISTRY:#x}',
        'records': len(converted),
        'records_sha256': hashlib.sha256(b''.join(converted[d] for d in sorted(converted))).hexdigest(),
        'scope': 'Drop the 7230-only trailing SkillUINode flag; 7148 layout otherwise unchanged',
    }
    temporary = manifest_path.with_suffix('.json.tmp')
    temporary.write_text(json.dumps(manifest, indent=2))
    temporary.replace(manifest_path)
    return manifest['skill_panel_repair']


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('test_directory', type=Path)
    args = parser.parse_args()
    print(json.dumps(repair(args.test_directory), indent=2))
