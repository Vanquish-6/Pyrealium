"""Downgrade unsupported examination accessors in an isolated 7148 test DAT.

7148 has no corresponding request field. Preserve its legacy request behavior,
stack cleanup and bytecode addresses. This does not implement newer examination
options or certify other scripts. Never modifies the game executable.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import struct

from ac2_dat import Dat, wlib_sections
from check_7148_scripts import examination_calls
from make_7148_test_assets import write_chain

RECORD = 0x56000005
SOURCE_HASH = 'a9a4012c80d51d68d0beaefb8bd49e87ada9a8332efa4c62d5a82e0f28e5d3b3'
FIRST_REPAIR_HASH = '53086c9c69abe6cfa1319589b65f3f93fffecb5afb79a0dffbc17ef913e6bdc1'
SITES = (0x2FE78, 0x2FEE8)
GETTER_SITE = 0x2FE58
GETTER_OLD = struct.pack('<III', 0xC, 0x9A0014, 0x10005)
# 7230 constructor 005cad30 initializes the added field to 0x203; getter
# 004754d0 returns it with tag 4. Pop the reserved return slot and receiver,
# push that typed default, then PopN(0). This includes the caller's cleanup.
GETTER_NEW = struct.pack('<III', 0x20005, 0x02030403, 5)
OLD = struct.pack('<III', 0xC, 0x9A0013, 0x20005)
# PopN(0), PopN(0), original PopN(2): a void setter consumes no arguments
# itself; the caller's existing PopN(2) performs cleanup. No offsets change.
NEW = struct.pack('<III', 5, 5, 0x20005)


def convert(data):
    digest = hashlib.sha256(data).hexdigest()
    if digest not in (SOURCE_HASH, FIRST_REPAIR_HASH):
        raise ValueError('Unrecognized WLib; refusing an offset-based repair')
    sections = wlib_sections(data)
    expected_sites = sorted((*SITES, GETTER_SITE)) if digest == SOURCE_HASH else [GETTER_SITE]
    if examination_calls(sections[1]) != expected_sites:
        raise ValueError('Unexpected examination call sites')
    position = 12
    while position < len(data):
        kind, size = struct.unpack_from('<II', data, position)
        if kind == 1:
            code_start = position + 12
            break
        position = (position + 8 + size + 3) & ~3
    else:
        raise ValueError('Missing opcode section')
    result = bytearray(data)
    for offset in SITES:
        start = code_start + offset
        expected = OLD if digest == SOURCE_HASH else NEW
        if result[start:start + len(OLD)] != expected:
            raise ValueError('Unexpected call or argument cleanup')
        result[start:start + len(NEW)] = NEW
    start = code_start + GETTER_SITE
    if result[start:start + 12] != GETTER_OLD:
        raise ValueError('Unexpected getter or argument cleanup')
    result[start:start + 12] = GETTER_NEW
    if examination_calls(wlib_sections(result)[1]):
        raise ValueError('Repair retained an unsupported setter')
    return bytes(result)


def repair(directory):
    directory = Path(directory).resolve()
    manifest_path = directory / 'pyrealium-compatibility.json'
    manifest = json.loads(manifest_path.read_text())
    source = Path(manifest['source']).resolve()
    portal = directory / 'portal.dat'
    if directory == source or os.path.samefile(portal, source / 'portal.dat') or portal.stat().st_nlink != 1:
        raise ValueError('Repair requires the separate, non-hardlinked test portal.dat')
    with Dat(portal) as archive:
        original = archive.read(RECORD)
        converted = convert(original)
        start, length, _ = archive.entries[RECORD]
        backup = directory / 'examination-wlib-before-repair-v2.bin'
        if backup.exists():
            if backup.read_bytes() != original:
                raise ValueError('Existing backup does not match')
        else:
            with backup.open('xb') as output:
                output.write(original)
                output.flush()
                os.fsync(output.fileno())
        with portal.open('r+b') as output:
            try:
                write_chain(output, archive, start, 0, converted)
                output.flush()
                os.fsync(output.fileno())
                with Dat(portal) as verified:
                    if verified.read(RECORD) != converted:
                        raise ValueError('Read-back verification failed')
            except BaseException:
                write_chain(output, archive, start, 0, original)
                output.flush()
                os.fsync(output.fileno())
                raise
    manifest['examination_repair'] = {
        'source_sha256': hashlib.sha256(original).hexdigest(),
        'output_sha256': hashlib.sha256(converted).hexdigest(),
        'bytes': length,
        'scope': 'Omit two newer setters and return typed default 0x203 from their getter; legacy request semantics only',
    }
    temporary = manifest_path.with_suffix('.json.tmp')
    temporary.write_text(json.dumps(manifest, indent=2))
    temporary.replace(manifest_path)
    return manifest['examination_repair']


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('test_directory', type=Path)
    args = parser.parse_args()
    print(json.dumps(repair(args.test_directory), indent=2))
