"""Native AC2 room assembly using unchanged render/collision/BSP mesh records.

Record evidence: CEnvCell::StreamPack00482da0, CCellPortal::UnPack0049c980,
CBaseMesh::StreamPack004d2ab0, CStaticMesh::StreamPack004d32d0,
CCellMesh::StreamPack004cf3a0, CPolygon::UnPack00499a90,
BSPNODE::UnPackChild/UnPack0049cf50/0049d5f0, BSPLEAF::UnPack0049fa40,
CPortalPoly::StreamPack0072f890 and CLandBlockInfo::StreamPack00491720.
Original records preserve opaque entity/property tails when links are edited.
New rooms intentionally contain geometry only, avoiding duplicated runtime IDs
or unrebased opaque properties from their source template.
"""
from copy import deepcopy
import math
import struct

from preview_mesh import decode_mesh


MAX_ROOMS = 512


class Reader:
    def __init__(self, data, offset=0):
        if not isinstance(data, (bytes, bytearray)) or len(data) > 64*1024*1024:
            raise ValueError("Dungeon record exceeds the supported size")
        self.data, self.offset = data, offset

    def take(self, fmt):
        size = struct.calcsize("<"+fmt)
        if self.offset+size > len(self.data):
            raise ValueError("Truncated native dungeon record")
        values = struct.unpack_from("<"+fmt, self.data, self.offset)
        self.offset += size
        return values[0] if len(values) == 1 else list(values)

    def count(self, limit=4096):
        count = self.take("I")
        if count > limit:
            raise ValueError("Dungeon array exceeds its supported bound")
        return count

    def align(self):
        self.offset = (self.offset+3)&~3
        if self.offset > len(self.data):
            raise ValueError("Truncated dungeon alignment")


def _finite(values):
    if any(not math.isfinite(v) or abs(v) >= 1_000_000 for v in values):
        raise ValueError("Invalid dungeon transform")
    return values


def parse_cell(data):
    r = Reader(data)
    cell = r.take("I")
    if not 0x100 <= cell & 65535 <= 0xfffb:
        raise ValueError("Expected an interior cell identifier")
    position, quaternion = _finite(r.take("3f")), _finite(r.take("4f"))
    if abs(sum(v*v for v in quaternion)-1) > .01:
        raise ValueError("Room orientation is not normalized")
    flags, mesh = r.take("I"), r.take("I")
    if flags & ~31 or mesh >> 24 != 1:
        raise ValueError("Unsupported room header")
    portals = []
    for _ in range(r.count(1024)):
        f, index, other, other_index = r.take("HhHh")
        if f & ~7:
            raise ValueError("Unsupported room portal flags")
        stabs = []
        if f & 4:
            count = r.take("H")
            if count > 4096:
                raise ValueError("Portal stab capacity exceeded")
            stabs = [r.take("H") for _ in range(count)]
            r.align()
        portals.append(dict(flags=f, index=index, other_cell=other, other_index=other_index, stabs=stabs))
    r.align()
    visible = [r.take("H") for _ in range(r.count())]
    r.align()
    shared = [r.take("H") for _ in range(r.count())]
    r.align()
    return dict(cell=cell, position=position, quaternion=quaternion, flags=flags, mesh=mesh,
                portals=portals, visible_cells=visible, shared_cells=shared, tail=data[r.offset:])


def encode_cell(room):
    out = bytearray(struct.pack("<I3f4fII", room["cell"], *room["position"], *room["quaternion"], room["flags"], room["mesh"]))
    def align():
        out.extend(bytes((-len(out))&3))
    out.extend(struct.pack("<I", len(room["portals"])))
    for door in room["portals"]:
        out.extend(struct.pack("<HhHh", door["flags"], door["index"], door["other_cell"]&65535, door["other_index"]))
        if door["flags"] & 4:
            out.extend(struct.pack("<H",len(door["stabs"])))
            out.extend(struct.pack("<"+"H"*len(door["stabs"]),*door["stabs"]))
            align()
    align()
    for key in ("visible_cells", "shared_cells"):
        values = room[key]
        out.extend(struct.pack("<I",len(values)))
        out.extend(struct.pack("<"+"H"*len(values),*(value&65535 for value in values)))
        align()
    out.extend(room["tail"])
    result = bytes(out)
    parse_cell(result)
    return result


def _polygons(reader, vertices):
    result = []
    for _ in range(reader.count(65536)):
        identity, count = reader.take("HH")
        if not 3 <= count <= 256:
            raise ValueError("Unsupported native polygon vertex count")
        indices = [reader.take("H") for _ in range(count)]
        if any(i >= len(vertices) for i in indices):
            raise ValueError("Collision polygon vertex exceeds mesh")
        reader.align()
        result.append(dict(id=identity, points=[vertices[i] for i in indices]))
    return result


def _bsp(reader, physics, depth=0, budget=None):
    if depth > 256:
        raise ValueError("BSP depth exceeds supported bound")
    budget = [0] if budget is None else budget
    budget[0] += 1
    if budget[0] > 200000:
        raise ValueError("BSP node budget exceeded")
    kind = reader.take("I")
    if physics and kind == 0x4c454146:  # BSPLEAF::UnPack.
        reader.take("I")
        _finite(reader.take("4f"))
        for _ in range(reader.count(65536)):
            reader.take("H")
        reader.align()
        return
    _finite(reader.take("4f"))  # Native splitting plane.
    if kind == 0x42502d4e:
        _bsp(reader, physics, depth+1,budget)
        _bsp(reader, physics, depth+1,budget)
    elif kind in (0x422d2d4e,0x42502d2d):
        _bsp(reader, physics, depth+1,budget)
    elif kind not in (0x4250464c,0x4c454146):
        raise ValueError(f"Unsupported BSP node {kind:08x}")
    if physics:
        _finite(reader.take("4f"))


def mesh_doorways(data):
    """Decode actual native portal polygons; preserve every original mesh byte."""
    mesh = decode_mesh(data)
    r = Reader(data,mesh["render_prefix_bytes"])
    render_polygons = _polygons(r,mesh["vertices"])
    if render_polygons:
        _bsp(r,True)
    physics = _polygons(r,mesh["vertices"])
    present = r.take("I")
    if present not in (0,1):
        raise ValueError("Invalid physics BSP presence")
    if present:
        _bsp(r,True)
    _bsp(r,False)
    doors = []
    for index in range(r.count(1024)):
        side, polygon = r.take("HH")
        sphere = _finite(r.take("4f"))
        if polygon >= len(physics) or sphere[3] < 0:
            raise ValueError("Invalid native doorway polygon reference")
        points = physics[polygon]["points"]
        doors.append(dict(index=index, side=side, polygon=polygon, points=points, sphere=sphere))
    info = r.take("I")
    if r.offset != len(data):
        raise ValueError("Unsupported trailing cell-mesh fields")
    return dict(mesh=mesh["did"], bounds=mesh["bounds"], doors=doors, info=info,
                collision_preserved=True)


def rotate(point, q):
    w,x,y,z = q
    a,b,c = point
    return [(1-2*(y*y+z*z))*a+2*(x*y-w*z)*b+2*(x*z+w*y)*c,
            2*(x*y+w*z)*a+(1-2*(x*x+z*z))*b+2*(y*z-w*x)*c,
            2*(x*z-w*y)*a+2*(y*z+w*x)*b+(1-2*(x*x+y*y))*c]


def multiply(a,b):
    w,x,y,z=a;v,i,j,k=b
    return [w*v-x*i-y*j-z*k,w*i+x*v+y*k-z*j,w*j-x*k+y*v+z*i,w*k+x*j-y*i+z*v]


def world_points(points,room):
    return [[a+b for a,b in zip(rotate(point,room["quaternion"]),room["position"])] for point in points]


def _center(points):
    return [sum(p[i] for p in points)/len(points) for i in range(3)]


def _bounds(mesh,room):
    low,high=mesh["bounds"]
    points=world_points([[high[i] if mask&(1<<i) else low[i] for i in range(3)] for mask in range(8)],room)
    return [[min(p[i] for p in points) for i in range(3)],[max(p[i] for p in points) for i in range(3)]]


def _overlap(a,b,tolerance=.04):
    return all(min(a[1][i],b[1][i])-max(a[0][i],b[0][i]) > tolerance for i in range(3))


def _fit_error(a,b):
    if len(a)!=len(b) or not a:
        return float('inf')
    return max(max(min(math.dist(p,q) for q in b) for p in a),
               max(min(math.dist(p,q) for q in a) for p in b))


def _normal(points):
    a,b,c=points[:3]
    u,v=[b[i]-a[i] for i in range(3)],[c[i]-a[i] for i in range(3)]
    n=[u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]]
    length=math.sqrt(sum(value*value for value in n))
    if length<1e-9:raise ValueError('Degenerate doorway polygon')
    return [value/length for value in n]


def _opposed(a,b):
    return sum(x*y for x,y in zip(_normal(a),_normal(b))) < -.999


def _block(value):
    if not isinstance(value,int) or not 0<=value<=0xffffffff:
        raise ValueError("Invalid target dungeon block")
    return value<<16 if value<=65535 else value&0xffff0000


class DungeonBuilder:
    def __init__(self,cells,portal,patches=None):
        self.cells,self.portal=cells,portal
        self.patches=patches if patches is not None else {}
        self.mesh_cache={}
        self.last_cell=None

    def read(self,identity):
        return self.patches[identity] if identity in self.patches else self.cells.read(identity)

    def room_template(self,cell):
        room=parse_cell(self.read(cell))
        if room['cell']!=cell:
            raise ValueError("Cell record identifier disagrees with its archive key")
        geometry=self.mesh_cache.get(room["mesh"])
        if geometry is None:
            geometry=mesh_doorways(self.portal.read(room["mesh"]))
            if len(self.mesh_cache)>=128:
                self.mesh_cache.pop(next(iter(self.mesh_cache)))
            self.mesh_cache[room["mesh"]]=geometry
        if geometry['mesh']!=room['mesh']:
            raise ValueError("Mesh record identifier disagrees with the room reference")
        for door in room["portals"]:
            if not 0<=door["index"]<len(geometry["doors"]):
                raise ValueError("Room portal refers outside the native doorway array")
        return room,geometry

    def doorways(self,cell):
        room,mesh=self.room_template(cell)
        result=[]
        for link in room["portals"]:
            source=mesh["doors"][link["index"]]
            points=world_points(source["points"],room)
            result.append(dict(index=link["index"],points=points,center=_center(points),normal=_normal(points),
                               side=source['side'],
                               open=bool(link["flags"]&2),other_cell=0 if link["flags"]&2 else (cell&0xffff0000)|link["other_cell"]))
        return result

    def templates(self,block=None):
        identities=set(self.cells.index())|set(self.patches)
        values=[did for did in identities if 0x100<=did&65535<=0xfffb and (block is None or did>>16==block)]
        return sorted(values)

    def template_catalog(self,block,limit=128):
        """Bounded geometry catalog, grouped by native mesh and doorway count.

        Labels describe measured geometry, not invented native room names.
        Unsupported meshes are excluded; callers can still inspect those cells.
        """
        if not isinstance(limit,int) or not 1<=limit<=MAX_ROOMS:
            raise ValueError('Room catalog limit must be between 1 and 512')
        block=_block(block)>>16
        result=[];seen=set()
        for cell in self.templates(block)[:MAX_ROOMS]:
            try:room,mesh=self.room_template(cell)
            except (ValueError,KeyError):continue
            count=len(room['portals']);key=(room['mesh'],count)
            if key in seen:continue
            seen.add(key)
            kind='Closed room' if count==0 else 'End cap' if count==1 else 'Passage' if count==2 else 'Junction'
            size=[round(high-low,1) for low,high in zip(*mesh['bounds'])]
            result.append(dict(cell=cell,mesh=room['mesh'],doorways=count,doorway_count=count,
                               dimensions=size,bounds=mesh['bounds'],
                               label=f"{kind} · {count} doorway{'s' if count!=1 else ''} · {size[0]:g} × {size[1]:g} × {size[2]:g}"))
            if len(result)>=limit:break
        return result

    def _changes(self,after):
        result={}
        for did,data in after.items():
            try:before=self.read(did)
            except KeyError:before=None
            if before!=data:result[did]=(before,data)
        return result

    def _block_info(self,block,count):
        did=block|65534
        try:
            data=bytearray(self.read(did))
            flags=struct.unpack_from("<I",data)[0]
            if flags&~1 or len(data)<16:
                raise ValueError("Unsupported native landblock-info header")
            struct.pack_into("<I",data,12 if flags&1 else 8,count)
            return did,bytes(data)
        except KeyError:
            return did,struct.pack("<9I",0,did,count,0,0,0,0,0,0)

    def _new_room(self,template,cell,position=None,yaw=0):
        room,mesh=self.room_template(template)
        room=deepcopy(room)
        room["cell"]=cell
        room["position"]=_finite(list(position if position is not None else room["position"]))
        half=math.radians(yaw)/2
        room["quaternion"]=multiply([math.cos(half),0,0,math.sin(half)],room["quaternion"])
        room["flags"] &= ~6
        room["tail"]=b""
        room["visible_cells"],room["shared_cells"]=[],[]
        for door in room["portals"]:
            door.update(flags=(door["flags"]&1)|2,other_cell=0,other_index=-1,stabs=[])
        return room,mesh

    def seed(self,template_cell,target_block,position=None,yaw=0):
        block=_block(target_block)
        if self.templates(block>>16):
            raise ValueError("Target block already contains interior cells; choose an unused block")
        cell=block|0x100
        room,mesh=self._new_room(template_cell,cell,position,yaw)
        did,info=self._block_info(block,1)
        self.last_cell=cell
        return self._changes({cell:encode_cell(room),did:info})

    def fit_room(self,target_cell,target_portal,template_cell,template_portal):
        target,target_mesh=self.room_template(target_cell)
        target_link=next((door for door in target["portals"] if door["index"]==target_portal),None)
        if target_link is None or not target_link["flags"]&2:
            raise ValueError("Choose an open doorway in the target room")
        source,source_mesh=self.room_template(template_cell)
        source_link=next((door for door in source["portals"] if door["index"]==template_portal),None)
        if source_link is None:
            raise ValueError("Template does not expose this doorway")
        target_points=world_points(target_mesh["doors"][target_portal]["points"],target)
        destination=_center(target_points)
        block=target_cell&0xffff0000
        existing=[self.room_template(cell) for cell in self.templates(block>>16)]
        if len(existing)>=MAX_ROOMS:
            raise ValueError("Dungeon room limit reached")
        candidates=[]
        for yaw in (0,90,180,270):
            candidate,geometry=self._new_room(template_cell,block|0x100,[0,0,0],yaw)
            points=world_points(source_mesh["doors"][template_portal]["points"],candidate)
            center=_center(points)
            candidate["position"]=[a-b for a,b in zip(destination,center)]
            points=world_points(source_mesh["doors"][template_portal]["points"],candidate)
            if len(points)!=len(target_points):continue
            error=_fit_error(points,target_points)
            if error>.03 or not _opposed(points,target_points):continue
            bounds=_bounds(geometry,candidate)
            if any(_overlap(bounds,_bounds(mesh,room)) for room,mesh in existing):continue
            candidates.append(dict(position=candidate["position"],quaternion=candidate["quaternion"],yaw=yaw,error=error))
        if not candidates:
            raise ValueError("No collision-free exact doorway fit; choose another room or doorway")
        return candidates

    def attach(self,existing_cell,portal_index,template_cell,template_portal_index,target_cell=None,candidate_index=0):
        fits=self.fit_room(existing_cell,portal_index,template_cell,template_portal_index)
        if not 0<=candidate_index<len(fits):raise ValueError("Invalid fit candidate")
        fit=fits[candidate_index]
        block=existing_cell&0xffff0000
        identities=self.templates(block>>16)
        low=max((did&65535 for did in identities),default=0xff)+1
        cell=target_cell if target_cell is not None else block|low
        if cell!=block|low or not 0x100<=cell&65535<=0xfffb:
            raise ValueError("New cells must use the next contiguous identifier in this dungeon")
        source,mesh=self._new_room(template_cell,cell,fit["position"],fit["yaw"])
        target,_=self.room_template(existing_cell)
        target=deepcopy(target)
        a=next(p for p in target["portals"] if p["index"]==portal_index)
        b=next(p for p in source["portals"] if p["index"]==template_portal_index)
        a.update(flags=(a["flags"]&1)|1,other_cell=cell&65535,other_index=template_portal_index,stabs=[])
        b.update(flags=(b["flags"]&1)|1,other_cell=existing_cell&65535,other_index=portal_index,stabs=[])
        # Conservative visibility includes the authored block, retaining opaque tails.
        all_ids=sorted(set(identities+[cell]))
        after={}
        for identity in all_ids:
            room=source if identity==cell else target if identity==existing_cell else deepcopy(self.room_template(identity)[0])
            room["visible_cells"]=[did&65535 for did in all_ids if did!=identity]
            after[identity]=encode_cell(room)
        did,info=self._block_info(block,max(did&65535 for did in all_ids)-0x100+1)
        after[did]=info
        self.last_cell=cell
        return self._changes(after)

    def connect(self,cell_a,portal_a,cell_b,portal_b):
        """Join two existing aligned open doorways without moving either room."""
        if cell_a==cell_b or cell_a>>16!=cell_b>>16:
            raise ValueError("Connect two different rooms within the same dungeon")
        a,ma=self.room_template(cell_a);b,mb=self.room_template(cell_b)
        a,b=deepcopy(a),deepcopy(b)
        pa=next((p for p in a['portals'] if p['index']==portal_a),None)
        pb=next((p for p in b['portals'] if p['index']==portal_b),None)
        if pa is None or pb is None or not pa['flags']&2 or not pb['flags']&2:
            raise ValueError("Choose two open native doorways")
        points_a=world_points(ma['doors'][portal_a]['points'],a)
        points_b=world_points(mb['doors'][portal_b]['points'],b)
        if _fit_error(points_a,points_b)>.03 or not _opposed(points_a,points_b):
            raise ValueError("Doorway polygons do not align")
        pa.update(flags=1,other_cell=cell_b&65535,other_index=portal_b,stabs=[])
        pb.update(flags=1,other_cell=cell_a&65535,other_index=portal_a,stabs=[])
        identities=self.templates(cell_a>>16)
        after={}
        for did in identities:
            room=a if did==cell_a else b if did==cell_b else deepcopy(self.room_template(did)[0])
            room['visible_cells']=[other&65535 for other in identities if other!=did]
            after[did]=encode_cell(room)
        return self._changes(after)

    def validate(self,block,require_closed=True):
        block=_block(block)
        errors=[]
        rooms={};meshes={}
        for did in self.templates(block>>16):
            try:rooms[did],meshes[did]=self.room_template(did)
            except (ValueError,KeyError) as exc:errors.append(f"Room {did:08x}: {exc}")
        if not rooms:
            return errors+["Dungeon has no rooms"]
        expected=max(did&65535 for did in rooms)-0x100+1
        if len(rooms)!=expected:errors.append("Dungeon cell identifiers contain gaps")
        try:
            info=self.read(block|65534);flags=struct.unpack_from('<I',info)[0]
            count=struct.unpack_from('<I',info,12 if flags&1 else 8)[0]
            if count!=expected:errors.append("Landblock room count disagrees with dungeon cells")
        except (KeyError,struct.error):errors.append("Dungeon landblock-info record is missing or truncated")
        for did,room in rooms.items():
            for door in room["portals"]:
                if door["flags"]&2:
                    if require_closed:errors.append(f"Room {did:08x} doorway {door['index']} is still open")
                    continue
                other=rooms.get(block|door["other_cell"])
                reciprocal=next((p for p in other["portals"] if p["index"]==door["other_index"]),None) if other else None
                if reciprocal is None or reciprocal["flags"]&2 or reciprocal["other_cell"]!=(did&65535) or reciprocal["other_index"]!=door["index"]:
                    errors.append(f"Room {did:08x} doorway {door['index']} lacks its reciprocal link")
                else:
                    points=world_points(meshes[did]['doors'][door['index']]['points'],room)
                    others=world_points(meshes[block|door['other_cell']]['doors'][reciprocal['index']]['points'],other)
                    if _fit_error(points,others)>.03 or not _opposed(points,others):
                        errors.append(f"Room {did:08x} doorway {door['index']} polygons do not align")
        return errors


DungeonEditor = DungeonBuilder
