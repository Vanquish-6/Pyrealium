"""Extract bounded skill eligibility metadata from supplied DAT script states.

Does not execute scripts or infer advancement curves, effects or starter grants.
Unknown dependency object layouts remain unresolved instead of being guessed.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
from ac2_dat import Dat


def word(data,offset):
    if offset+4>len(data):
        raise ValueError("truncated script state")
    return struct.unpack_from('<I',data,offset)[0]

def fingerprint(data):
    result=0xcbf29ce484222325
    for value in data:result=((result^value)*0x100000001b3)&0xffffffffffffffff
    return result


def header(data,did):
    if len(data)>1024*1024 or word(data,0)!=did or word(data,4)!=len(data)-12:
        raise ValueError("unknown script state header")
    count=word(data,8)
    if not 1<=count<=8192 or 12+count*4>len(data):
        raise ValueError("script state reference limit")
    if list(struct.unpack_from('<'+str(count)+'I',data,12))!=list(range(count)):
        raise ValueError("nonsequential script references")
    return count,12+count*4


def root(data,pos):
    if word(data,pos)!=0x5c000100 or word(data,pos+4)!=0 or word(data,pos+8)&65535:
        raise ValueError("unknown root object layout")
    count=word(data,pos+12)
    if not 1<=count<=256 or pos+16+count*4+((count+3)//4*4)>len(data):
        raise ValueError("root field limit")
    values=list(struct.unpack_from('<'+str(count)+'I',data,pos+16))
    return values,pos+16+count*4+(count+3)//4*4


def dependency_maps(data,pos,refs):
    required=[ref for ref in refs if ref!=0xffffffff]
    if not required:
        return [{},{}]
    if min(required)<1 or max(required)>128:
        raise ValueError("dependency reference limit")
    nodes={}
    for index in range(1,max(required)+1):
        flag=word(data,pos)
        if flag==0x24000100:
            word(data,pos+4);pos+=8;continue
        if flag!=0x44000100 or word(data,pos+4)!=0:
            raise ValueError("unsupported dependency object")
        kind=word(data,pos+8);pos+=12
        if kind==0xffff006a:
            # StringInfo with no substitutions or literal, plus empty HeapObject.
            if word(data,pos+8)!=0 or word(data,pos+12)!=0:
                raise ValueError("unsupported dependency-prefix string")
            pos+=16
        elif kind in [0xffff0001,0xffff0011]:
            count=word(data,pos)&0xffffff
            if count>256 or word(data,pos)>>24>22:
                raise ValueError("dependency hash limit")
            pos+=4;pairs={}
            for _ in range(count):
                key,value=word(data,pos),word(data,pos+4);pos+=8
                if key==0 or key>65535 or value>10000 or key in pairs:
                    raise ValueError("invalid dependency entry")
                pairs[key]=value
            if word(data,pos)!=0:
                raise ValueError("dependency hash interpreted fields")
            pos+=4;nodes[index]=pairs
        else:
            raise ValueError(f"unsupported dependency native kind {kind:08x}")
    return [nodes[ref] if ref!=0xffffffff else {} for ref in refs]


def convert(game_data):
    output=['pyrealium-skills-v1'];unresolved=[];definitions=[]
    with Dat(game_data/'portal.dat') as portal:
        data=portal.read(0x700016e2);count,pos=header(data,0x700016e2);values,pos=root(data,pos)
        if values!=[0x41000011,0xffffffff,1] or [word(data,pos+i*4) for i in range(3)]!=[0x44000100,0,0xffff0011]:
            raise ValueError("unsupported skill-index graph")
        entries=word(data,pos+12)&0xffffff
        if entries!=count-2 or entries>8190:
            raise ValueError("skill index size mismatch")
        pairs=[struct.unpack_from('<II',data,pos+16+i*8) for i in range(entries)]
        pos+=16+entries*8
        if word(data,pos)!=0:
            raise ValueError("skill-index hash fields")
        pos+=4;references={}
        for ref in range(2,count):
            if word(data,pos)!=0x24000100:
                raise ValueError("skill index external-reference layout")
            references[ref]=word(data,pos+4);pos+=8
        if pos+4!=len(data) or word(data,pos)!=0x16e2 or len({key for key,_ in pairs})!=entries:
            raise ValueError("skill index identity or trailer")
        output.append('# Skill-index SHA256: '+hashlib.sha256(data).hexdigest())
        output.append('# Index FNV64: '+f'{fingerprint(data):016x}')
        for skill,ref in sorted(pairs):
            did=references[ref]
            try:
                record=portal.read(did);node_count,pos=header(record,did);fields,pos=root(record,pos)
                if len(fields)<24 or fields[0]!=skill or fields[2]>7 or fields[3]>150 or fields[20]>1000 or not 1<=fields[17]<=10000:
                    raise ValueError("unsupported skill common-field bounds")
                forbidden,required=dependency_maps(record,pos,[fields[4],fields[6]])
                if skill in forbidden or skill in required:
                    raise ValueError("self-dependent skill")
                curve=fields[15]
                if curve>>24!=0x6f:
                    raise ValueError("unknown advancement reference type")
                portal.read(curve)
                definitions.append((skill,did,hashlib.sha256(record).hexdigest(),fingerprint(record),fields[2],max(1,fields[3]),fields[20],fields[17],curve,fields[23],required,forbidden))
            except (ValueError,KeyError) as error:
                unresolved.append({'skill':skill,'record':f'{did:08x}','reason':str(error)})
    for skill,did,sha,fnv,race,level,credits,cap,curve,kind,required,forbidden in definitions:
        pairs=lambda mapping: ','.join(f'{key:x}:{value}' for key,value in sorted(mapping.items())) or '-'
        output.append('\t'.join([f'{skill:x}',f'{did:08x}',sha,f'{fnv:016x}',str(race),str(level),str(credits),str(cap),f'{curve:08x}',str(kind),pairs(required),pairs(forbidden)]))
    return '\n'.join(output)+'\n',{'definitions':len(definitions),'index_entries':entries,'unresolved':unresolved,'advancement_curves_evaluated':False}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--game-data',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--report',type=Path,required=True)
    args=p.parse_args();text,report=convert(args.game_data)
    for path,value in [(args.output,text),(args.report,json.dumps(report,indent=2)+'\n')]:
        path.parent.mkdir(parents=True,exist_ok=True);temp=path.with_name(path.name+'.tmp');temp.write_text(value,encoding='utf-8',newline='\n');temp.replace(path)
    print(f"Imported {report['definitions']} skill definitions; {len(report['unresolved'])} remain unresolved.")


if __name__=='__main__':
    main()
