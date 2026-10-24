"""Native dungeon record, collision reuse and doorway assembly regressions."""
import math
import struct
import unittest
from unittest.mock import patch

from world_dungeon import DungeonBuilder, encode_cell, parse_cell, mesh_doorways


class Archive:
    def __init__(self, records):self.records=records
    def index(self):return self.records
    def read(self,did):return self.records[did]


def room(cell=0x12340100,tail=b'',flags=0,doors=1):
    return dict(cell=cell,position=[0,0,0],quaternion=[1,0,0,0],flags=flags,mesh=0x01000001,
                portals=[dict(flags=2,index=i,other_cell=0,other_index=-1,stabs=[]) for i in range(doors)],
                visible_cells=[],shared_cells=[],tail=tail)


def builder(source=None):
    source=source or room()
    cells=Archive({source['cell']:encode_cell(source)})
    editor=DungeonBuilder(cells,Archive({}))
    editor.mesh_cache[0x01000001]=dict(mesh=0x01000001,bounds=[[0,0,0],[10,4,4]],
        doors=[dict(index=0,side=0,points=[[10,0,0],[10,4,0],[10,4,4],[10,0,4]]),
               dict(index=1,side=1,points=[[0,0,0],[0,0,4],[0,4,4],[0,4,0]])])
    return editor


def apply(editor,changes):
    editor.patches.update({did:after for did,(before,after) in changes.items()})


class DungeonTests(unittest.TestCase):
    def test_native_tail_and_portal_stabs_roundtrip(self):
        source=room(tail=b'opaque entity and property bytes',flags=6,doors=2)
        source['portals'][0].update(flags=5,other_cell=0x101,other_index=1,stabs=[0x100,0x101,0x102])
        source['visible_cells']=[0x101,0x102,0x103]
        source['shared_cells']=[0x102]
        data=encode_cell(source)
        self.assertEqual(encode_cell(parse_cell(data)),data)
        self.assertEqual(parse_cell(data)['tail'],source['tail'])

    def test_seed_is_geometry_only_and_does_not_mutate_archive(self):
        editor=builder(room(tail=b'opaque original IDs',flags=6))
        original=dict(editor.cells.records)
        changes=editor.seed(0x12340100,0x7777)
        self.assertEqual(editor.patches,{})
        self.assertEqual(editor.cells.records,original)
        self.assertEqual(set(changes),{0x77770100,0x7777fffe})
        self.assertIsNone(changes[0x77770100][0])
        new=parse_cell(changes[0x77770100][1])
        self.assertEqual(new['mesh'],0x01000001)
        self.assertEqual(new['flags']&6,0)
        self.assertEqual(new['tail'],b'')
        self.assertEqual(struct.unpack('<9I',changes[0x7777fffe][1])[:3],(0,0x7777fffe,1))

    def test_attach_exact_shapes_and_reciprocal_native_links(self):
        editor=builder()
        apply(editor,editor.seed(0x12340100,0x7777))
        self.assertTrue(editor.validate(0x7777))
        changes=editor.attach(0x77770100,0,0x12340100,0)
        apply(editor,changes)
        self.assertEqual(editor.last_cell,0x77770101)
        self.assertEqual(editor.validate(0x7777),[])
        first=parse_cell(editor.read(0x77770100));second=parse_cell(editor.read(0x77770101))
        self.assertEqual(first['portals'][0]['other_cell'],0x101)
        self.assertEqual(second['portals'][0]['other_cell'],0x100)
        self.assertEqual(first['visible_cells'],[0x101])
        self.assertEqual(second['visible_cells'],[0x100])
        # Mesh stays the original native mesh; render and collision records are reused.
        self.assertTrue(all(did>>24!=1 for did in changes))
        with self.assertRaisesRegex(ValueError,'open doorway'):
            editor.attach(0x77770100,0,0x12340100,0)

    def test_attachment_preserves_existing_opaque_runtime_data(self):
        source=room(cell=0x77770100,flags=6,tail=b'keep original entities')
        editor=builder(source)
        changes=editor.attach(0x77770100,0,0x77770100,0)
        self.assertEqual(parse_cell(changes[0x77770100][1])['tail'],source['tail'])
        self.assertEqual(parse_cell(changes[0x77770101][1])['tail'],b'')

    def test_guard_invalid_transform_ids_and_mismatch(self):
        editor=builder()
        with self.assertRaises(ValueError):editor.seed(0x12340100,-1)
        with self.assertRaises(ValueError):editor.seed(0x12340100,0x1234)
        with self.assertRaises(ValueError):editor.seed(0x12340100,0x7777,position=[math.nan,0,0])
        apply(editor,editor.seed(0x12340100,0x7777))
        with self.assertRaisesRegex(ValueError,'contiguous'):
            editor.attach(0x77770100,0,0x12340100,0,target_cell=0x77770110)
        editor.mesh_cache[0x01000001]['doors'][0]['points'][0][2]=2
        # Both ends share a mesh, but rotating this asymmetric aperture cannot fit without overlap.
        with self.assertRaisesRegex(ValueError,'exact doorway fit'):
            editor.fit_room(0x77770100,0,0x12340100,0)

    def test_validate_geometry_and_count_in_addition_to_links(self):
        editor=builder()
        apply(editor,editor.seed(0x12340100,0x7777))
        apply(editor,editor.attach(0x77770100,0,0x12340100,0))
        shifted=parse_cell(editor.read(0x77770101));shifted['position'][0]+=1
        editor.patches[0x77770101]=encode_cell(shifted)
        self.assertTrue(any('align' in error for error in editor.validate(0x7777)))
        editor.patches[0x7777fffe]=struct.pack('<9I',0,0x7777fffe,99,0,0,0,0,0,0)
        self.assertTrue(any('count' in error for error in editor.validate(0x7777)))

    def test_connect_checks_polygons_then_adds_reciprocal_links(self):
        editor=builder()
        apply(editor,editor.seed(0x12340100,0x7777))
        apply(editor,editor.attach(0x77770100,0,0x12340100,0))
        for cell in (0x77770100,0x77770101):
            opened=parse_cell(editor.read(cell))
            opened['portals'][0].update(flags=2,other_cell=0,other_index=-1)
            editor.patches[cell]=encode_cell(opened)
        apply(editor,editor.connect(0x77770100,0,0x77770101,0))
        self.assertEqual(editor.validate(0x7777),[])

    def test_connect_rejects_same_facing_coplanar_polygons(self):
        editor=builder()
        apply(editor,editor.seed(0x12340100,0x7777))
        second=parse_cell(editor.read(0x77770100));second['cell']=0x77770101
        editor.patches[second['cell']]=encode_cell(second)
        with self.assertRaisesRegex(ValueError,'align'):
            editor.connect(0x77770100,0,0x77770101,0)

    def test_catalog_labels_measured_native_templates_with_bound(self):
        editor=builder()
        catalog=editor.template_catalog(0x1234,1)
        self.assertEqual(len(catalog),1)
        self.assertEqual(catalog[0]['cell'],0x12340100)
        self.assertEqual(catalog[0]['doorway_count'],1)
        self.assertEqual(catalog[0]['dimensions'],[10,4,4])
        self.assertIn('End cap',catalog[0]['label'])
        with self.assertRaises(ValueError):editor.template_catalog(0x1234,513)

    def test_native_portal_polygon_is_sourced_from_physics_array(self):
        vertices=[[10,0,0],[10,4,0],[10,4,4],[10,0,4]]
        # Empty render polygons; one native physics polygon; no physics BSP;
        # CELL_BSP leaf; one CPortalPoly; mesh-info DID.
        data=struct.pack('<IIHH4H',0,1,42,4,0,1,2,3)
        data+=struct.pack('<II4fIHH4fI',0,0x4c454146,1,0,0,-10,1,1,0,10,2,2,3,0)
        with patch('world_dungeon.decode_mesh',return_value=dict(did=0x01000001,bounds=[[0,0,0],[10,4,4]],vertices=vertices,render_prefix_bytes=0)):
            geometry=mesh_doorways(data)
            self.assertEqual(geometry['doors'][0]['points'],vertices)
            self.assertEqual(geometry['doors'][0]['side'],1)
            self.assertTrue(geometry['collision_preserved'])
            with self.assertRaises(ValueError):mesh_doorways(data[:-1])
            with self.assertRaises(ValueError):mesh_doorways(data+b'extra')


if __name__=='__main__':unittest.main()
