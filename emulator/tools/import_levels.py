"""Convert the supplied numeric level table without executing SQL.

This is an imported development progression profile, not verified 7148 balance.
Changing it after journals exist requires an explicit progression migration.
"""
import argparse
import hashlib
from pathlib import Path
import re


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source",type=Path,required=True)
    parser.add_argument("--output",type=Path,required=True)
    args = parser.parse_args()
    with args.source.open("rb") as file:
        raw = file.read(1024*1024+1)
    if len(raw)>1024*1024:
        raise ValueError("level table exceeds 1 MiB")
    rows = [tuple(map(int,m.group(1).split(','))) for m in re.finditer(r'\((\d+(?:,\d+){9})\)',raw.decode('utf-8-sig'))]
    if len(rows)!=150 or [row[0] for row in rows]!=list(range(1,151)):
        raise ValueError("expected exactly 150 sequential level rows")
    if rows[0][1]!=0 or any(a[1]>=b[1] or a[2]>b[2] or a[3]>b[3] or a[4]>b[4] for a,b in zip(rows,rows[1:])):
        raise ValueError("nonmonotonic progression table")
    if any(not 0<row[2]<=1_000_000 or not 0<row[3]<=1_000_000 or row[1]>2**63-1 or row[4]>1000 for row in rows):
        raise ValueError("invalid progression bounds")
    result = ['//! Imported development level profile; not verified native balance.',
              '//! Source SHA256: '+hashlib.sha256(raw).hexdigest(),
              '//! Immutable for progression journal revision g1.',
              'use super::Level;', 'pub const LEVELS: &[Level] = &[']
    for row in rows:
        result.append(f'    Level {{ number: {row[0]}, xp: {row[1]}, health: {row[2]}, vigor: {row[3]}, credits: {row[4]} }},')
    result.append('];')
    args.output.write_text('\n'.join(result)+'\n',encoding='utf-8',newline='\n')
    print(f'Imported {len(rows)} level rows.')


if __name__=='__main__':
    main()
