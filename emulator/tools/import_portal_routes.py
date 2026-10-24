"""Join retained portal placements to numeric telepads without executing SQL.

Missing destinations and unsupported hero/kingdom requirements remain unresolved.
This imports route data; it does not establish native portal-space acceptance.
"""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path
import re
from ac2_dat import Dat


def telepads(source):
    raw=source.read_bytes()
    if len(raw)>1024*1024:
        raise ValueError('telepad source exceeds 1 MiB')
    rows={}
    row=re.compile(r"\((?:'(?:[^'\\]|\\.)*'|[^'()])*(?:\))")
    for line in raw.decode('utf-8-sig').splitlines():
        prefix='INSERT INTO `telepad` VALUES '
        if not line.startswith(prefix):
            continue
        if not line.endswith(';'):
            raise ValueError('unterminated telepad insert')
        payload=line[len(prefix):-1]; pos=0
        while pos<len(payload):
            match=row.match(payload,pos)
            if not match:
                raise ValueError('unsupported telepad tuple')
            fields=next(csv.reader([match[0][1:-1]],quotechar="'",escapechar='\\',strict=True))
            if len(fields)!=12 or len(fields[1])>256:
                raise ValueError('telepad field shape')
            identity=int(fields[0]); active=int(fields[10])
            if not 0<identity<=1000000 or active not in [0,1] or identity in rows or len(rows)>=8192:
                raise ValueError('telepad identity/active bounds')
            try:
                cell=int(fields[2]); values=list(map(float,fields[3:10]))
                if not 0<cell<=0xffffffff or any(not math.isfinite(v) or abs(v)>=1_000_000 for v in values):
                    raise ValueError('telepad coordinate bounds')
                if abs(sum(v*v for v in values[3:])-1)>0.01 or abs(values[4])+abs(values[5])>0.00001:
                    raise ValueError('nonplanar telepad rotation')
                heading=round((math.atan2(-values[6],values[3])*255/math.pi)%255)
                entry={'cell':cell,'position':values[:3],'heading':heading,'active':active,'label':fields[1]}
            except ValueError as error:
                entry={'error':str(error),'label':fields[1]}
            rows[identity]=entry
            pos=match.end()
            if pos<len(payload):
                if payload[pos]!=',':raise ValueError('unsupported tuple separator')
                pos+=1
    if not rows:raise ValueError('no telepad rows')
    return rows,hashlib.sha256(raw).hexdigest()


def convert(placements,source,game_data):
    if placements.stat().st_size>1024*1024:raise ValueError('portal metadata size')
    metadata=json.loads(placements.read_text(encoding='utf-8'))
    portals=metadata['portals']
    if not 1<=len(portals)<=1024:raise ValueError('portal metadata count')
    pads,source_hash=telepads(source)
    output=['pyrealium-portal-routes-v1','# Portal snapshot SHA256: '+metadata['source_sha256'],
            '# Telepad source SHA256: '+source_hash,
            '# IID entity sourceCell sourceXYZ destinationCell destinationXYZ heading minLevel (tabs)']
    unresolved=[]; seen=set(); imported=0
    with Dat(game_data/'cell_1.dat') as cells:
        index=cells.index()
        for portal in portals:
            identity=int(portal['id'],16)
            if identity in seen:raise ValueError('duplicate portal identity')
            seen.add(identity)
            reason=None
            pad=pads.get(portal['telepad'])
            requirements=portal['requirements']
            if portal['status']!='placed':reason='source-not-placed'
            elif requirements['hero'] or requirements['kingdom']:reason='unsupported-eligibility'
            elif pad is None:reason='missing-telepad'
            elif 'error' in pad:reason=pad['error']
            elif pad['active']!=1:reason='inactive-telepad'
            else:
                low=pad['cell']&65535
                if not (1<=low<=64 or 256<=low<=65531):reason='invalid-destination-cell'
                else:
                    record=pad['cell']|65535 if low<=64 else pad['cell']
                    if record not in index:reason='missing-destination-cell'
                    else:cells.read(record)
            if reason:
                unresolved.append({'id':portal['id'],'telepad':portal['telepad'],'reason':reason});continue
            level=max(1,int(requirements['level']))
            if level>150:raise ValueError('portal level bound')
            row=[portal['id'],portal['entity'],portal['cell'],*map(str,portal['position']),f"{pad['cell']:08x}",*map(str,pad['position']),str(pad['heading']),str(level)]
            if len(row)!=12:raise ValueError('route row shape')
            output.append('\t'.join(row))
            imported+=1
        # Explicit development gateway into the retained human bootcamp exit
        # telepad. Its source placement is authored, not an original spawn.
        pad=pads[1473]
        if 'error' in pad or pad['active']!=1:raise ValueError('training gateway destination is unavailable')
        record=pad['cell']|65535 if pad['cell']&65535<=64 else pad['cell']
        cells.read(record)
        output.append('# Authored training gateway source; destination comes from telepad 1473.')
        output.append('\t'.join(['8140000000030002','4700084d','02990109','59.060577','244.199','-44.606205',f"{pad['cell']:08x}",*map(str,pad['position']),str(pad['heading']),'1']))
    return '\n'.join(output)+'\n',{'portal_snapshot_sha256':metadata['source_sha256'],'telepad_source_sha256':source_hash,'routes':imported+1,'imported_routes':imported,'authored_routes':1,'unresolved':unresolved}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['placements','telepads','game-data','output','report']:
        parser.add_argument('--'+name,type=Path,required=True)
    args=parser.parse_args();result,report=convert(args.placements,args.telepads,args.game_data)
    for path,text in [(args.output,result),(args.report,json.dumps(report,indent=2)+'\n')]:
        temporary=path.with_name(path.name+'.tmp');temporary.write_text(text,encoding='utf-8',newline='\n');temporary.replace(path)
    print(f"Imported {report['routes']} portal routes; {len(report['unresolved'])} remain unresolved.")


if __name__=='__main__':main()
