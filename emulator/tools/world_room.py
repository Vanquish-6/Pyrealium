"""Bounded rectangular AC2 cell-mesh authoring, including native collision.

Recovered build-7148 evidence: CBaseMesh::StreamPack004d2ab0,
CStaticMesh::StreamPack004d32d0, CCellMesh::StreamPack004cf3a0,
CPolygon::UnPack00499a90/MakePlane00499940, BSPNODE::UnPack0049d5f0,
BSPLEAF::UnPack0049fa40, point_intersects_solid0049d1e0/0049f860,
point_inside_cell_bsp0049cff0, CPortalPoly::StreamPack0072f890.

Walls are real solid box volumes with doorway holes. The render triangles and
physics polygons describe the same surfaces. A finite axis-aligned partition
assigns native solid leaves to wall volumes, empty leaves to room/openings.
Portal aperture polygons are excluded from solid leaves. This authoring profile
has structural/geometry tests; native-client traversal still requires acceptance.
"""
import math
import struct

from preview_mesh import decode_mesh
from world_dungeon import DungeonBuilder, Reader, encode_cell, mesh_doorways, parse_cell


DIRECTIONS=('north','east','south','west')
ALIASES=dict(n='north',e='east',s='south',w='west',north='north',east='east',south='south',west='west')


def _directions(values):
    if isinstance(values,str):values=values.replace(',',' ').split()
    if isinstance(values,dict):values=[side for side,enabled in values.items() if enabled]
    try:result={ALIASES[str(value).lower()] for value in values}
    except (KeyError,TypeError) as exc:raise ValueError('Door openings must be north, east, south or west') from exc
    return [side for side in DIRECTIONS if side in result]


def _number(value,low,high,label):
    if isinstance(value,bool) or not isinstance(value,(int,float)) or not math.isfinite(value) or not low<=value<=high:
        raise ValueError(f'{label} must be between {low:g} and {high:g}')
    return float(value)


def _sphere(low,high):
    center=[(a+b)/2 for a,b in zip(low,high)]
    return center+[math.dist(low,high)/2+.001]


def _face(low,high,axis,positive):
    """Four points with the native MakePlane cross product facing out of a box."""
    other=[i for i in range(3) if i!=axis]
    points=[]
    for a,b in ((0,0),(1,0),(1,1),(0,1)):
        point=list(low);point[axis]=high[axis] if positive else low[axis]
        point[other[0]]=high[other[0]] if a else low[other[0]]
        point[other[1]]=high[other[1]] if b else low[other[1]]
        points.append(point)
    # Cartesian orientation of the two remaining axes has a negative Y cross.
    if positive != (axis!=1):points.reverse()
    return points


def _normal(points):
    a,b,c=points[:3];u=[b[i]-a[i] for i in range(3)];v=[c[i]-a[i] for i in range(3)]
    normal=[u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]]
    length=math.sqrt(sum(n*n for n in normal))
    return [n/length for n in normal]


def _align(data):data.extend(bytes((-len(data))&3))


def _physics_bsp(boxes,poly_refs,sphere):
    """Native BSP with both children present; outside occupied boxes is empty.

    Unlike an all-polygons single leaf, this preserves point-solid semantics.
    Native sphere queries traverse adjacent leaves near splitting planes and
    test the actual face polygons referenced by each solid leaf.
    """
    cuts=[sorted({box[end][axis] for box in boxes for end in (0,1)}) for axis in range(3)]
    nodes=[0]
    def tree(axis,remaining,ranges):
        nodes[0]+=1
        if nodes[0]>4095:raise ValueError('Room collision partition exceeds its node bound')
        if remaining:
            middle=len(remaining)//2;cut=remaining[middle]
            upper=list(ranges);lower=list(ranges)
            upper[axis]=(cut,ranges[axis][1]);lower[axis]=(ranges[axis][0],cut)
            plane=[0.,0.,0.,-cut];plane[axis]=1.
            positive=tree(axis,remaining[middle+1:],upper)
            negative=tree(axis,remaining[:middle],lower)
            return struct.pack('<I4f',0x42502d4e,*plane)+positive+negative+struct.pack('<4f',*sphere)
        if axis<2:return tree(axis+1,cuts[axis+1],ranges)
        center=[(a+b)/2 if math.isfinite(a) and math.isfinite(b) else b-1 if math.isfinite(b) else a+1 for a,b in ranges]
        refs=sorted({p for box,ids in zip(boxes,poly_refs) if all(box[0][i]<center[i]<box[1][i] for i in range(3)) for p in ids})
        out=bytearray(struct.pack('<II4fI',0x4c454146,int(bool(refs)),*sphere,len(refs)))
        out.extend(struct.pack('<'+'H'*len(refs),*refs));_align(out)
        return bytes(out)
    return tree(0,cuts[0],[(-math.inf,math.inf)]*3)


def _cell_bsp(width,depth,height):
    # A positive path through all six inward planes identifies the cell volume.
    planes=((1,0,0,width/2),(-1,0,0,width/2),(0,1,0,depth/2),(0,-1,0,depth/2),(0,0,1,0),(0,0,-1,height))
    return b''.join(struct.pack('<I4f',0x42502d2d if i<5 else 0x4250464c,*plane) for i,plane in enumerate(planes))


def encode_room_mesh(mesh_id,width=40,depth=40,height=20,openings=('north','south'),
                     door_width=8,door_height=12,material=None,wall_thickness=.5):
    """Encode a new native CCellMesh record, without writing any archive.

    Room width/depth are measured between the outer portal planes; wall
    thickness projects inward. Floor is z=0, ceiling z=height. Openings are
    centered, start at floor level and include their native portal polygons.
    """
    if not isinstance(mesh_id,int) or not 0x01000001<=mesh_id<=0x01ffffff:
        raise ValueError('Custom room requires a native 01 mesh identifier')
    if not isinstance(material,int) or material>>24!=0x31:
        raise ValueError('Custom room requires an existing native 31 render material')
    width=_number(width,4,512,'Width');depth=_number(depth,4,512,'Depth');height=_number(height,4,128,'Height')
    wall_thickness=_number(wall_thickness,.1,2,'Wall thickness')
    if wall_thickness>=min(width,depth)/4:raise ValueError('Walls leave insufficient interior space')
    sides=_directions(openings)
    span=min((width if side in ('north','south') else depth for side in sides),default=514)
    door_width=_number(door_width,2,span-2*wall_thickness,'Door width')
    door_height=_number(door_height,2,height-wall_thickness if sides else 128,'Door height')
    x,y,t=width/2,depth/2,wall_thickness
    boxes=[([-x,-y,-t],[x,y,0]),([-x,-y,height],[x,y,height+t])]
    for side in DIRECTIONS:
        axis=1 if side in ('north','south') else 0
        positive=side in ('north','east');edge=y if axis==1 else x
        low=[-x,-y,0];high=[x,y,height]
        low[axis]=edge-t if positive else -edge
        high[axis]=edge if positive else -edge+t
        if side not in sides:boxes.append((low,high));continue
        horizontal=1-axis
        for start,end,z0,z1 in ((low[horizontal],-door_width/2,0,height),
                                (door_width/2,high[horizontal],0,height),
                                (-door_width/2,door_width/2,door_height,height)):
            a=list(low);b=list(high);a[horizontal]=start;b[horizontal]=end;a[2]=z0;b[2]=z1
            boxes.append((a,b))
    vertices=[];indices=[];polygons=[];poly_refs=[]
    def face(points,render=True):
        normal=_normal(points);start=len(vertices);identity=len(polygons)
        edge=[points[1][i]-points[0][i] for i in range(3)];length=math.sqrt(sum(v*v for v in edge));tangent=[v/length for v in edge]
        bitangent=[normal[1]*tangent[2]-normal[2]*tangent[1],normal[2]*tangent[0]-normal[0]*tangent[2],normal[0]*tangent[1]-normal[1]*tangent[0]]
        for point in points:
            displacement=[point[i]-points[0][i] for i in range(3)]
            uv=[sum(displacement[i]*vector[i] for i in range(3))/10 for vector in (tangent,bitangent)]
            vertices.append(struct.pack('<3f3fII3f3f2f',*point,*normal,0xffffffff,0xff000000,*tangent,*bitangent,*uv))
        polygons.append([start,start+1,start+2,start+3])
        if render:indices.extend([start,start+1,start+2,start,start+2,start+3])
        return identity
    for low,high in boxes:
        poly_refs.append([face(_face(low,high,axis,positive)) for axis in range(3) for positive in (False,True)])
    portals=[]
    for side in sides:
        axis=1 if side in ('north','south') else 0;positive=side in ('north','east')
        low=[-door_width/2,-door_width/2,0];high=[door_width/2,door_width/2,door_height]
        low[axis]=high[axis]=(y if axis==1 else x)*(1 if positive else -1)
        points=_face(low,high,axis,positive)
        portals.append((face(points,False),_sphere(low,high)))
    low=[-x,-y,-t];high=[x,y,height+t];sphere=_sphere(low,high)
    out=bytearray(struct.pack('<6I',mesh_id,2,1,material,0x300001d2,len(vertices)))
    out.extend(b''.join(vertices));_align(out)
    out.extend(struct.pack('<10f',*low,*high,*sphere))
    # One native triangle-list degrade and one material fragment.
    out.extend(struct.pack('<6I',1,1,7,1,0,len(indices)))
    out.extend(struct.pack('<'+'H'*len(indices),*indices));_align(out)
    out.extend(struct.pack('<II',0,len(polygons)))  # No base picking BSP; static physics follows.
    for identity,refs in enumerate(polygons):
        out.extend(struct.pack('<HH4H',identity,4,*refs));_align(out)
    out.extend(struct.pack('<I',1))
    out.extend(_physics_bsp(boxes,poly_refs,sphere))
    out.extend(_cell_bsp(width,depth,height))
    out.extend(struct.pack('<I',len(portals)))
    for polygon,portal_sphere in portals:out.extend(struct.pack('<HH4f',1,polygon,*portal_sphere))
    out.extend(struct.pack('<I',0))  # No optional CCellMeshInfo dependency.
    result=bytes(out)
    mesh_doorways(result)  # Reject malformed geometry before exposing a patch.
    return result


class _Overlay:
    def __init__(self,archive,patches=None,transient=None):
        self.archive,self.patches,self.transient=archive,patches or {},transient or {}
    def index(self):return set(self.archive.index())|set(self.patches)
    def read(self,did):
        if did in self.transient:return self.transient[did]
        return self.patches[did] if did in self.patches else self.archive.read(did)


def build_room(cells,portal,width=40,depth=40,height=20,doors=('north','south'),door_width=8,door_height=12,
               texture_template_cell=0x0299010d,target_block=0xf100,cell_patches=None,portal_patches=None,
               attach_cell=None,attach_portal=None,source_opening=None,position=None,material=None):
    """Create and optionally snap a custom native room as an atomic archive bundle.

    Returns cell_1.dat and portal.dat change maps in the project's existing
    before/after patch format, plus the allocated cell/mesh and opening indices.
    There are no installed archive mutations or temporary exported templates.
    """
    cell_view=_Overlay(cells,cell_patches);portal_view=_Overlay(portal,portal_patches)
    occupied=portal_view.index()
    mesh_id=next((did for did in range(0x01f00000,0x01ffffff+1) if did not in occupied),None)
    if mesh_id is None:raise ValueError('Custom room mesh identifier range is full')
    if material is None:
        template=parse_cell(cell_view.read(texture_template_cell))
        materials=decode_mesh(portal_view.read(template['mesh']))['materials']
        if not materials:raise ValueError('Texture template has no native render material')
        material=materials[0]
    portal_view.read(material)  # Real existing dependency; never synthesize a material.
    sides=_directions(doors)
    geometry=encode_room_mesh(mesh_id,width,depth,height,sides,door_width,door_height,material)
    portal_view=_Overlay(portal_view,{mesh_id:geometry})
    block=(attach_cell>>16) if attach_cell is not None else target_block if target_block<=65535 else target_block>>16
    transient=0xfffe0100 if block!=0xfffe else 0xfffd0100
    source=dict(cell=transient,position=[0,0,0],quaternion=[1,0,0,0],flags=0,mesh=mesh_id,
                portals=[dict(flags=2,index=i,other_cell=0,other_index=-1,stabs=[]) for i in range(len(sides))],
                visible_cells=[],shared_cells=[],tail=b'')
    editor=DungeonBuilder(_Overlay(cell_view,transient={transient:encode_cell(source)}),portal_view)
    if attach_cell is None:
        changes=editor.seed(transient,target_block,position=position if position is not None else [0,0,0])
    else:
        if attach_portal is None:raise ValueError('Choose the existing doorway for attachment')
        candidates=sides if source_opening is None else _directions([source_opening])
        if not candidates:raise ValueError('A room needs an opening to attach')
        failure=None;changes=None
        for side in candidates:
            if side not in sides:raise ValueError('Selected source opening is not enabled')
            try:changes=editor.attach(attach_cell,attach_portal,transient,sides.index(side));break
            except ValueError as exc:failure=exc
        if changes is None:raise ValueError(f'Custom room cannot fit this doorway: {failure}')
    return {'cell_1.dat':changes,'portal.dat':{mesh_id:(None,geometry)},'cell':editor.last_cell,
            'mesh':mesh_id,'openings':{side:i for i,side in enumerate(sides)}}
