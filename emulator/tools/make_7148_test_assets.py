"""Create an isolated, narrowly converted DAT set for a 7148 compatibility test.

Never writes the source installation. Only two exact SHA256-pinned WState
records are converted. Other DATs are hardlinked and must be used with --rodat.
This is an experiment, not a general 7230-to-7148 converter.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
from ac2_dat import Dat

EXPECTED = {
    0x7100110F: '9309501ecfb18ad37e134ff3d6c93bf56fb7a237ffd8d93c07085601fff38f0e',
    0x7000037F: '46137060e001ce8932507555ee73a5e23fcb856d8e3a4123a6aac2d2645d946b',
}

def convert(data):
    did = struct.unpack_from('<I', data)[0]
    if hashlib.sha256(data).hexdigest() != EXPECTED.get(did):
        raise ValueError('Unrecognized source WState; no offset-based conversion allowed')
    # 7148 GameplayOptionsProfile::StreamPack 006000b0 knows mask bits 0..12.
    # Installed mask 27EA adds a trailing u32/u32 hash table at 062C: three pairs.
    if struct.unpack_from('<Q', data, 0x310)[0] != 0x27EA:
        raise ValueError('Unexpected options mask')
    if struct.unpack_from('<7I', data, 0x62C) != (0x02000003,0x10000,1,0x100,1,0x20000,1):
        raise ValueError('Unexpected added options table')
    result = bytearray(data[:0x62C] + data[0x648:])
    struct.pack_into('<I', result, 4, len(result)-12)
    struct.pack_into('<Q', result, 0x310, 0x7EA)
    return bytes(result)

def locate(dat, did):
    node_offset, seen = dat.root, set()
    while node_offset not in seen:
        seen.add(node_offset)
        node = dat.chain(node_offset, 0x4CC)
        count = struct.unpack_from('<I', node, 248)[0]
        if count > 61:
            raise ValueError('Bad node count')
        index = 0
        while index < count:
            key = struct.unpack_from('<I', node, 252+16*index)[0]
            if key == did:
                return node_offset, 252+16*index+8
            if key > did:
                break
            index += 1
        if struct.unpack_from('<I', node)[0] == 0:
            break
        node_offset = struct.unpack_from('<I', node, 4*index)[0]
    raise ValueError('Record missing or node cycle')

def write_chain(file, dat, start, logical_offset, data):
    """Overwrite existing payload bytes only; never allocate or change links."""
    blocks, seen = [], set()
    offset = start
    needed = logical_offset + len(data)
    while len(blocks)*(dat.block_size-4) < needed:
        if offset in seen or offset % dat.block_size or not dat.block_size <= offset <= dat.size-dat.block_size:
            raise ValueError('Invalid write chain')
        seen.add(offset)
        blocks.append(offset)
        dat.file.seek(offset)
        offset = struct.unpack('<I', dat.file.read(4))[0]
    written = 0
    while written < len(data):
        block, inner = divmod(logical_offset+written, dat.block_size-4)
        size = min(len(data)-written, dat.block_size-4-inner)
        file.seek(blocks[block]+4+inner)
        file.write(data[written:written+size])
        written += size

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('destination', type=Path)
    args = parser.parse_args()
    source, destination = args.source.resolve(), args.destination.resolve()
    if destination.exists() or source == destination:
        raise ValueError('Destination must be new; existing installations are never modified')
    with Dat(source/'portal.dat') as dat:
        originals = {did: dat.read(did) for did in EXPECTED}
        converted = {did: convert(data) for did,data in originals.items()}
    if shutil.disk_usage(destination.parent).free < (source/'portal.dat').stat().st_size + 256*1024*1024:
        raise ValueError('Not enough disk space for isolated portal copy plus reserve')
    destination.mkdir()
    # Copy the only archive to be modified. Never hardlink this file.
    shutil.copyfile(source/'portal.dat', destination/'portal.dat')
    with Dat(destination/'portal.dat') as dat, (destination/'portal.dat').open('r+b') as output:
        dat.index()
        for did,data in converted.items():
            start, length, _ = dat.entries[did]
            node, size_offset = locate(dat, did)
            write_chain(output, dat, start, 0, data + bytes(length-len(data)))
            write_chain(output, dat, node, size_offset, struct.pack('<I',len(data)))
        output.flush()
        os.fsync(output.fileno())
    with Dat(destination/'portal.dat') as dat:
        for did,data in converted.items():
            if dat.read(did) != data:
                raise ValueError('Converted archive verification failed')
    with Dat(source/'portal.dat') as dat:
        for did,data in originals.items():
            if dat.read(did) != data:
                raise ValueError('Source archive changed during preparation')
    for path in source.iterdir():
        if not path.is_file() or path.name.lower() == 'portal.dat':
            continue
        if path.suffix.lower() == '.dat' and path.stat().st_size:
            os.link(path, destination/path.name)
        elif path.suffix.lower() in ('.dll','.ini','.keymap','.hrf'):
            shutil.copyfile(path, destination/path.name)
    manifest = {'source':str(source), 'required_client':'7148', 'required_argument':'--rodat',
                'scope':'experimental GameplayOptionsProfile conversion only',
                'records':{hex(did):{'source_sha256':EXPECTED[did], 'output_sha256':hashlib.sha256(data).hexdigest(),
                                   'source_bytes':len(originals[did]),'output_bytes':len(data)}
                           for did,data in converted.items()}}
    (destination/'pyrealium-compatibility.json').write_text(json.dumps(manifest,indent=2))
    print(json.dumps(manifest,indent=2))

if __name__ == '__main__':
    main()
