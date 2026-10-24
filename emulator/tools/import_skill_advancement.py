"""Extract referenced cumulative skill XP vectors without executing scripts.

Uses actual advancement DIDs from the existing skill catalog, never a low-ID
lookalike. Unsupported roots/vectors stop conversion instead of inventing costs.
"""
import argparse
import hashlib
from pathlib import Path
import struct
from ac2_dat import Dat
from import_skill_catalog import header, root, word, fingerprint


def convert(catalog, game_data):
    text = catalog.read_text(encoding='utf-8')
    if len(text.encode('utf-8')) > 4*1024*1024 or not text.startswith('pyrealium-skills-v1\n'):
        raise ValueError('unsupported skill catalog')
    ids = set()
    for line in text.splitlines()[1:]:
        if not line or line.startswith('#'):
            continue
        fields = line.split('\t')
        if len(fields) != 12:
            raise ValueError('skill catalog row shape')
        ids.add(int(fields[8],16))
    if not 1 <= len(ids) <= 256:
        raise ValueError('advancement record count')
    output = ['pyrealium-skill-advancement-v1',
              '# Cumulative XP by base skill level, including index zero.',
              '# DID FNV64 SHA256 values (tab-separated); immutable for k2 journal replay.']
    with Dat(game_data/'portal.dat') as portal:
        for did in sorted(ids):
            data = portal.read(did)
            count, pos = header(data,did)
            start = pos
            values, pos = root(data,pos)
            if count != 3 or word(data,start+8) != 0x01e50000 or len(values) != 3:
                raise ValueError(f'{did:08x}: unsupported advancement root')
            if values[0] != 1 or values[2] != 2 or not 1 <= values[1] <= 10000 or data[start+28:start+32] != bytes([1,4,1,0]):
                raise ValueError(f'{did:08x}: unsupported advancement fields')
            if [word(data,pos+i*4) for i in range(3)] != [0x44000100,0,0xffff0037]:
                raise ValueError(f'{did:08x}: missing native u64 vector')
            levels = word(data,pos+12)
            if levels != values[1]+1 or pos+20+levels*8 > len(data):
                raise ValueError(f'{did:08x}: inconsistent vector bounds')
            xp = struct.unpack_from('<'+'Q'*levels,data,pos+16)
            end = pos+16+levels*8
            if word(data,end) != 0 or [word(data,end+4+i*4) for i in range(3)] != [0x44000100,0,0xffff0076] or word(data,len(data)-4) != 0x1e5:
                raise ValueError(f'{did:08x}: unsupported vector footer')
            if xp[0] != 0 or any(a>b for a,b in zip(xp,xp[1:])) or max(xp)>0x7fffffffffffffff:
                raise ValueError(f'{did:08x}: invalid cumulative XP')
            output.append(f'{did:08x}\t{fingerprint(data):016x}\t{hashlib.sha256(data).hexdigest()}\t'+','.join(map(str,xp)))
    return '\n'.join(output)+'\n', len(ids)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--catalog',type=Path,required=True)
    parser.add_argument('--game-data',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    text,count=convert(args.catalog,args.game_data)
    temporary=args.output.with_name(args.output.name+'.tmp')
    temporary.write_text(text,encoding='utf-8',newline='\n')
    temporary.replace(args.output)
    print(f'Imported {count} referenced cumulative skill XP vectors.')


if __name__ == '__main__':
    main()
