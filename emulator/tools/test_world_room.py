"""Independent native BSP/geometry checks for generated custom rooms."""
import math
import struct
import unittest

from preview_mesh import decode_mesh
from world_dungeon import DungeonBuilder, Reader, encode_cell, mesh_doorways, parse_cell
from world_room import build_room, encode_room_mesh


class Archive:
    def __init__(self,records):self.records=records
    def index(self):return self.records
    def read(self,did):return self.records[did]


def native_physics(data):
    mesh=decode_mesh(data);r=Reader(data,mesh['render_prefix_bytes'])
    assert r.take('I')==0
    polygons=[]
    for _ in range(r.take('I')):
        identity,count=r.take('HH');polygons.append((identity,[r.take('H') for i in range(count)]));r.align()
    assert r.take('I')==1
    def tree():
        kind=r.take('I')
        if kind==0x4c454146:
            solid=r.take('I');sphere=r.take('4f');refs=[r.take('H') for i in range(r.take('I'))];r.align()
            assert bool(solid)==bool(refs)
            assert all(0<=index<len(polygons) for index in refs)
            return dict(refs=refs,sphere=sphere)
        assert kind==0x42502d4e
        plane=r.take('4f');positive=tree();negative=tree();sphere=r.take('4f')
        return dict(plane=plane,positive=positive,negative=negative,sphere=sphere)
    return mesh,polygons,tree(),r


def solid(tree,point):
    while 'plane' in tree:
        plane=tree['plane'];positive=sum(a*b for a,b in zip(plane[:3],point))+plane[3]>=0
        tree=tree['positive' if positive else 'negative']
    # Mirrors recovered native BSPLEAF::point_intersects_solid0049f860.
    return bool(tree['refs'])


def references(tree):
    if 'refs' in tree:return set(tree['refs'])
    return references(tree['positive'])|references(tree['negative'])


def cell_contains(reader,point):
    while True:
        kind=reader.take('I');plane=reader.take('4f')
        if sum(a*b for a,b in zip(plane[:3],point))+plane[3]<-.0002:return False
        if kind==0x4250464c:return True
        assert kind==0x42502d2d


def archives():
    mesh=encode_room_mesh(0x01000001,material=0x31000001)
    template=dict(cell=0x0299010d,position=[0,0,0],quaternion=[1,0,0,0],flags=0,mesh=0x01000001,
                  portals=[dict(flags=2,index=i,other_cell=0,other_index=-1,stabs=[]) for i in range(2)],
                  visible_cells=[],shared_cells=[],tail=b'')
    return Archive({template['cell']:encode_cell(template)}),Archive({0x01000001:mesh,0x31000001:b'native material fixture'})


class RoomTests(unittest.TestCase):
    def test_native_render_and_physics_are_both_present(self):
        data=encode_room_mesh(0x01f00000,40,30,20,('north','south'),8,12,0x31000001)
        mesh,polygons,tree,_=native_physics(data)
        self.assertTrue(mesh['indices']);self.assertEqual(mesh['materials'],[0x31000001])
        self.assertEqual(len(mesh['normals']),len(mesh['vertices']))
        self.assertEqual(len(mesh['uvs']),len(mesh['vertices']))
        self.assertTrue(all(abs(sum(x*x for x in n)-1)<.001 for n in mesh['normals']))
        portals=mesh_doorways(data)['doors']
        portal_ids={p['polygon'] for p in portals}
        self.assertFalse(references(tree)&portal_ids)
        self.assertEqual(references(tree),set(range(len(polygons)))-portal_ids)
        self.assertEqual(len(portals),2)
        self.assertEqual(portals[0]['points'][0][1],15)
        for identity in portal_ids:
            self.assertFalse(set(polygons[identity][1])&set(mesh['indices']))
        # Floor top is a walkable upward-facing polygon in the real physics array.
        floor_top=polygons[5][1]
        self.assertTrue(all(mesh['vertices'][index][2]==0 for index in floor_top))
        self.assertTrue(all(mesh['normals'][index][2]>.999 for index in floor_top))

    def test_native_point_collision_has_empty_interior_and_real_door_holes(self):
        _,_,tree,_=native_physics(encode_room_mesh(0x01f00000,40,30,20,('north','south'),8,12,0x31000001))
        for point in ((0,0,5),(0,14.75,5),(0,-14.75,5),(0,15.25,5),(40,40,5)):
            self.assertFalse(solid(tree,point),point)
        for point in ((0,0,-.25),(0,0,20.25),(10,14.75,5),(0,14.75,13),(19.75,0,5),(-19.75,0,5)):
            self.assertTrue(solid(tree,point),point)

    def test_all_four_openings_and_native_cell_bsp(self):
        data=encode_room_mesh(0x01f00000,40,30,20,('N','E','S','W'),8,12,0x31000001)
        _,_,tree,reader=native_physics(data)
        for point in ((0,14.75,5),(0,-14.75,5),(19.75,0,5),(-19.75,0,5)):
            self.assertFalse(solid(tree,point),point)
        offset=reader.offset
        for point,expected in (((0,0,10),True),((19,14,1),True),((21,0,5),False),((0,16,5),False),((0,0,-1),False),((0,0,21),False)):
            self.assertEqual(cell_contains(Reader(data,offset),point),expected,point)
        self.assertEqual(len(mesh_doorways(data)['doors']),4)

    def test_bounds_and_dimensions_reject_invalid_or_unbounded_rooms(self):
        for arguments in (dict(width=math.nan),dict(width=513),dict(depth=-1),dict(height=3),dict(door_width=41),dict(door_height=20),dict(openings=('up',)),dict(wall_thickness=3)):
            with self.subTest(arguments=arguments),self.assertRaises(ValueError):
                encode_room_mesh(0x01f00000,material=0x31000001,**arguments)
        with self.assertRaises(ValueError):encode_room_mesh(0x06000001,material=0x31000001)
        with self.assertRaises(ValueError):encode_room_mesh(0x01000001,material=0x32000001)

    def test_door_dimensions_depend_on_the_chosen_wall(self):
        # A shallow chamber can have a wide opening on its wide wall.
        mesh=encode_room_mesh(0x01f00000,40,4,20,('north',),20,12,0x31000001)
        self.assertEqual(len(mesh_doorways(mesh)['doors']),1)
        closed=encode_room_mesh(0x01f00000,4,4,4,(),material=0x31000001)
        self.assertEqual(mesh_doorways(closed)['doors'],[])

    def test_atomic_bundle_allocates_both_archives_without_mutation(self):
        cells,portal=archives();old_cells=dict(cells.records);old_portal=dict(portal.records)
        bundle=build_room(cells,portal,doors=('N',),target_block=0xf100)
        self.assertEqual(cells.records,old_cells);self.assertEqual(portal.records,old_portal)
        self.assertEqual(bundle['cell'],0xf1000100)
        self.assertEqual(bundle['openings'],{'north':0})
        self.assertEqual(set(bundle['cell_1.dat']),{0xf1000100,0xf100fffe})
        room=parse_cell(bundle['cell_1.dat'][bundle['cell']][1])
        self.assertEqual(room['mesh'],bundle['mesh'])
        self.assertEqual(bundle['portal.dat'][bundle['mesh']][0],None)
        self.assertEqual(decode_mesh(bundle['portal.dat'][bundle['mesh']][1])['materials'],[0x31000001])

    def test_custom_rooms_snap_into_closed_native_dungeon(self):
        cells,portal=archives()
        first=build_room(cells,portal,doors=('N',),target_block=0xf100)
        cp={did:pair[1] for did,pair in first['cell_1.dat'].items()};pp={did:pair[1] for did,pair in first['portal.dat'].items()}
        second=build_room(cells,portal,doors=('S',),attach_cell=first['cell'],attach_portal=0,
                          cell_patches=cp,portal_patches=pp)
        self.assertNotEqual(first['mesh'],second['mesh'])
        cp.update({did:pair[1] for did,pair in second['cell_1.dat'].items()});pp.update({did:pair[1] for did,pair in second['portal.dat'].items()})
        class PortalOverlay:
            def read(self,did):return pp[did] if did in pp else portal.read(did)
        editor=DungeonBuilder(cells,PortalOverlay(),cp)
        self.assertEqual(editor.validate(0xf100),[])
        self.assertEqual(editor.doorways(first['cell'])[0]['other_cell'],second['cell'])

    def test_bad_attach_is_transactional_and_missing_material_refused(self):
        cells,portal=archives();cp={};pp={}
        with self.assertRaises((ValueError,KeyError)):
            build_room(cells,portal,doors=(),attach_cell=0x0299010d,attach_portal=0,cell_patches=cp,portal_patches=pp)
        self.assertEqual(cp,{});self.assertEqual(pp,{})
        with self.assertRaises(KeyError):build_room(cells,portal,material=0x31009999)


if __name__=='__main__':unittest.main()
