"""Extract stationary world actors from retained classic Ethernet/UDP PCAPs.

This is a capture observation importer, not an authored spawn schedule or packet
replay engine. It emits fresh minimal descriptions through the normal runtime.
Unrecognized visual/physics/description layouts are reported, never guessed.
"""
import argparse
from collections import Counter
import hashlib
import json
import math
from pathlib import Path
import struct

from ac2_dat import Dat


class Unsupported(ValueError):
    pass


class Reader:
    def __init__(self, data):
        self.data, self.pos = data, 0

    def take(self, size):
        end = self.pos+size
        if end > len(self.data):
            raise Unsupported("truncated-object")
        value = self.data[self.pos:end]
        self.pos = end
        return value

    def word(self):
        return struct.unpack("<I", self.take(4))[0]

    def identity(self):
        return struct.unpack("<Q", self.take(8))[0]

    def table(self, width):
        count = self.word()
        if count > 128:
            raise Unsupported("physics-table-limit")
        self.take(count*width)

    def position(self):
        cell, *values = struct.unpack("<I7f", self.take(32))
        if not cell or any(not math.isfinite(v) or abs(v)>=1_000_000 for v in values):
            raise Unsupported("invalid-position")
        if abs(sum(v*v for v in values[3:])-1)>0.01:
            raise Unsupported("invalid-quaternion")
        return cell, values[:3], values[3:]


def object_description(data):
    r = Reader(data)
    if r.word() != 0x10058:
        raise Unsupported("not-create-object")
    identity = r.identity()
    # Narrow capture namespace profile excludes player avatars and their state.
    if identity >> 32 != 0x81200000:
        raise Unsupported("non-world-namespace")
    if r.word() != 2:
        raise Unsupported("unsupported-visual-description")
    visual = r.word()
    flags = r.word()
    supported = 0x80|2|0x20000|0x8000|0x100|4|0x400000|0x800000|0x1000000|8|0x10|0x2000000|0x4000000|0x8000000|0x400|0x800|0x1000|0x200|0x4000
    if flags & ~supported or not flags & 0x8000:
        raise Unsupported("unsupported-or-unplaced-physics")
    r.take(4)  # Native instance and visual-ordering stamps, not reused.
    if flags & 0x80:
        r.take(4)
    if flags & 2:
        r.table(12)  # key + SliderData(value, velocity), 00640f40 / 0067cfb0.
    if flags & 0x20000:
        r.take(4)
    cell, coordinates, orientation = r.position()
    for bit, size in [(0x100,4),(4,12),(0x400000,12),(0x800000,4),(0x1000000,4),
                      (8,12),(0x10,12),(0x2000000,8),(0x4000000,4),(0x8000000,4),
                      (0x400,8),(0x800,32),(0x1000,12)]:
        if flags & bit:
            r.take(size)
    if flags & 0x200:
        r.table(16)  # key + FXScalarAndTarget(float, IID), 006410e0 / 004d5720.
    r.take(8)  # Four native timestamp ushorts.
    r.take((-r.pos) % 4)
    mask = r.word()
    fields = [(1,4),(0x2000000,4),(2,4),(0x10,4),(0x20,8),(0x40,8),(0x100,8),
              (0x200,8),(0x20000000,8),(0x40000000,8),(0x4000000,8),(0x80,4),
              (0x800,4),(0x1000,4),(0x2000,4),(0x200000,4),(0x400000,4),
              (0x8000,4),(0x10000,4),(0x20000,4),(0x40000,4),(0x80000,4),
              (0x100000,4),(0x8000000,4),(0x10000000,4)]
    allowed = sum(bit for bit, _ in fields)
    if mask & ~allowed or not mask & 0x2000000 or mask & (0x20|0x40):
        raise Unsupported("unsupported-or-contained-description")
    values = {}
    for bit, size in fields:
        if mask & bit:
            values[bit] = r.take(size)
    if r.pos != len(data):
        raise Unsupported("trailing-description-data")
    entity = struct.unpack("<I", values[0x2000000])[0]
    if entity >> 24 != 0x47 or visual >> 24 != 0x1f:
        raise Unsupported("unsupported-definition-types")
    scale = struct.unpack("<f",values[0x80])[0] if 0x80 in values else None
    if scale is not None and (not math.isfinite(scale) or not 0<scale<=100):
        raise Unsupported("invalid-scale")
    return {"source_iid":f"{identity:016x}","entity":f"{entity:08x}","visual":f"{visual:08x}","scale":scale,
            "cell":f"{cell:08x}","position":coordinates,"orientation":orientation,
            "physics_mask":f"{flags:08x}","description_mask":f"{mask:08x}",
            "message_sha256":hashlib.sha256(data).hexdigest()}


def messages(raw):
    if len(raw)<24 or raw[:4] not in [b"\xd4\xc3\xb2\xa1",b"\x4d\x3c\xb2\xa1"] or struct.unpack_from("<I",raw,20)[0]!=1:
        raise ValueError("expected little-endian classic Ethernet PCAP")
    pos, groups, storage = 24, {}, 0
    while pos<len(raw):
        if pos+16>len(raw):
            raise ValueError("truncated PCAP record header")
        length = struct.unpack_from("<I",raw,pos+8)[0]
        pos += 16
        if length>1024*1024 or pos+length>len(raw):
            raise ValueError("oversized or truncated PCAP record")
        frame = raw[pos:pos+length]
        pos += length
        if len(frame)<42 or frame[12:14]!=b"\x08\x00":
            continue
        ip = frame[14:]
        ih = (ip[0]&15)*4
        total = struct.unpack_from(">H",ip,2)[0]
        if ip[0]>>4!=4 or ih<20 or total>len(ip) or total<ih+8 or ip[9]!=17 or struct.unpack_from(">H",ip,6)[0]&0x3fff:
            continue
        udp = ip[ih:total]
        udp_length = struct.unpack_from(">H",udp,4)[0]
        if udp_length<28 or udp_length>len(udp):
            continue
        payload = udp[8:udp_length]
        flags = struct.unpack_from("<I",payload,4)[0]
        allowed_flags = 2|4|0x1000|0x2000|0x4000|0x1000000|0x2000000|0x4000000|0x8000000
        if not flags&4 or flags&~allowed_flags or len(payload)!=20+struct.unpack_from("<H",payload,16)[0]:
            continue
        offset = 20
        for flag,size in [(0x1000,None),(0x2000,None),(0x4000,4),(0x1000000,8),(0x2000000,4),(0x4000000,8),(0x8000000,6)]:
            if flags&flag:
                if size is None:
                    if offset+4>len(payload):
                        raise ValueError("truncated optional packet header")
                    count = struct.unpack_from("<I",payload,offset)[0]
                    if count>256:
                        raise ValueError("optional packet header count limit")
                    size = 4+4*count
                offset += size
        if offset>len(payload):
            raise ValueError("truncated optional packet fields")
        while offset<len(payload):
            if offset+16>len(payload):
                raise ValueError("truncated fragment header")
            blob,count,size,index,queue = struct.unpack_from("<Q4H",payload,offset)
            if not 0<count<=256 or index>=count or size<=16 or offset+size>len(payload):
                raise ValueError("invalid fragment bounds")
            key = (ip[12:20],udp[:4],payload[12:14],payload[18:20],blob,queue,count)
            fragment = payload[offset+16:offset+size]
            parts = groups.setdefault(key,{})
            if index in parts and parts[index]!=fragment:
                raise ValueError("conflicting duplicate fragment")
            if index not in parts:
                storage += len(fragment)
                parts[index] = fragment
            if len(groups)>8192 or storage>16*1024*1024 or sum(map(len,parts.values()))>65536:
                raise ValueError("capture reassembly limit")
            offset += size
    for key,parts in groups.items():
        if len(parts)==key[-1]:
            yield b"".join(parts[i] for i in range(key[-1]))


def convert(paths, base, game_data, skip_broken=False):
    with base.open("rb") as file:
        base_bytes = file.read(4*1024*1024+1)
    if len(base_bytes)>4*1024*1024:
        raise ValueError("base manifest exceeds 4 MiB")
    text = base_bytes.decode("utf-8").replace("\r\n","\n")
    if not text.startswith("pyrealium-world-v2\n"):
        raise ValueError("base must be a world-v2 manifest")
    lines = text.splitlines()
    occupied, positions, counts = set(), set(), Counter()
    for line in lines[1:]:
        if not line or line.startswith("#"):
            continue
        fields = line.split("\t")
        if len(fields)!=11 or len(line.encode("utf-8"))>512:
            raise ValueError("invalid base placement")
        identity = int(fields[0],16)
        if identity in occupied or identity>>48!=0x8140:
            raise ValueError("invalid or duplicate base identity")
        occupied.add(identity)
        counts[int(fields[3],16)] += 1
        positions.add((fields[1],fields[3],*(round(float(v),2) for v in fields[4:7])))
    if len(occupied)>16384 or any(count>64 for count in counts.values()):
        raise ValueError("base manifest exceeds runtime content capacity")
    accepted, sources, claimed_cells = [], [], set()
    with Dat(game_data/'portal.dat') as portal, Dat(game_data/'cell_1.dat') as cells:
        assets, cell_index = portal.index(), cells.index()
        for path in paths:
            try:
                with path.open("rb") as file:
                    raw = file.read(64*1024*1024+1)
                if len(raw)>64*1024*1024:
                    raise ValueError("capture exceeds 64 MiB")
                blobs = list(messages(raw))
            except ValueError as error:
                if not skip_broken:
                    raise
                sources.append({"file":path.name,"sha256":"","accepted":0,"rejected":{"unreadable-capture":str(error)}})
                continue
            source_hash = hashlib.sha256(raw).hexdigest()
            source = {"file":path.name,"sha256":source_hash,"accepted":0,"rejected":Counter()}
            seen, source_cells = set(), set()
            for data in blobs:
                if data[:4]!=b"\x58\x00\x01\x00":
                    continue
                try:
                    actor = object_description(data)
                    if actor["source_iid"] in seen:
                        raise Unsupported("repeated-observation")
                    seen.add(actor["source_iid"])
                    entity,visual,cell = (int(actor[key],16) for key in ["entity","visual","cell"])
                    if cell in claimed_cells:
                        raise Unsupported("cell-covered-by-earlier-capture")
                    low = cell&65535
                    if not (1<=low<=64 or 256<=low<=65531):
                        raise Unsupported("invalid-world-cell")
                    record = cell|65535 if low<=64 else cell
                    if entity not in assets or visual not in assets or record not in cell_index:
                        raise Unsupported("missing-dat-asset-or-cell")
                    position = (actor["entity"],actor["cell"],*(round(v,2) for v in actor["position"]))
                    if position in positions:
                        raise Unsupported("duplicate-placement")
                    if counts[cell]>=64 or len(occupied)>=16384:
                        raise Unsupported("world-or-cell-capacity")
                    digest = hashlib.sha256(bytes.fromhex(source_hash)+bytes.fromhex(actor["source_iid"])).digest()
                    identity = 0x8140800000000000 | (int.from_bytes(digest[:6],"big")&0x7fffffffffff)
                    if identity in occupied:
                        raise ValueError("observed object identity collision")
                    # Read backing records, not just directory entries.
                    portal.read(entity); portal.read(visual); cells.read(record)
                    occupied.add(identity); positions.add(position); counts[cell] += 1
                    actor.update({"id":f"{identity:016x}","source_sha256":source_hash})
                    accepted.append(actor)
                    source_cells.add(cell)
                    source["accepted"] += 1
                except Unsupported as error:
                    source["rejected"][str(error)] += 1
            source["rejected"] = dict(sorted(source["rejected"].items()))
            sources.append(source)
            claimed_cells.update(source_cells)
    lines[0] = "pyrealium-world-v3"
    lines = [line+"\t-" if line and not line.startswith("#") and index>0 else line for index,line in enumerate(lines)]
    lines = [line+"\tscale" if line.startswith("# IID") else line for line in lines]
    lines.append("# Capture-observed stationary actors; not authored spawn schedules or AI.")
    for actor in sorted(accepted,key=lambda actor:actor["id"]):
        lines.append("\t".join([actor["id"],actor["entity"],actor["visual"],actor["cell"],
                                *(format(v,'.9g') for v in actor["position"]+actor["orientation"]),
                                "-" if actor["scale"] is None else format(actor["scale"],'.9g')]))
    report = {"capture_build_verified":False,"checksums_verified":False,"stationary_only":True,
              "overlap_policy":"first accepted capture per cell, in command-line input order",
              "omitted_state":["mode","sliders","effects","velocity","look/targets","description overrides except scale","native lifetime stamps"],
              "base_sha256":hashlib.sha256(base_bytes).hexdigest(),
              "placed_actors":len(accepted),"sources":sources,"actors":accepted}
    return "\n".join(lines)+"\n",report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--capture",type=Path,action="append",required=True)
    parser.add_argument("--base",type=Path,required=True)
    parser.add_argument("--game-data",type=Path,required=True)
    parser.add_argument("--output",type=Path,required=True)
    parser.add_argument("--report",type=Path,required=True)
    args = parser.parse_args()
    result,report = convert(args.capture,args.base,args.game_data)
    for path,value in [(args.output,result),(args.report,json.dumps(report,indent=2)+"\n")]:
        path.parent.mkdir(parents=True,exist_ok=True)
        temporary = path.with_name(path.name+".tmp")
        temporary.write_text(value,encoding="utf-8",newline="\n")
        temporary.replace(path)
    print(f"Imported {report['placed_actors']} stationary actors from {len(report['sources'])} captures.")
    for source in report["sources"]:
        print(source["file"],source["accepted"],source["rejected"])


if __name__ == "__main__":
    main()
