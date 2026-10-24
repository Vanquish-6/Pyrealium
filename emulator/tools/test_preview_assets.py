"""Regressions for native null-material helpers and physics-only room meshes."""
import struct
import unittest
from unittest.mock import patch

from preview_assets import Assets, entity_visual
from preview_material import material_info, first_layer_state, MaterialFormatError
from preview_mesh import decode_mesh, MeshFormatError


def words(*values):
    return struct.pack("<" + "I"*len(values), *values)


def prop(name, kind, value):
    return words(name, kind, len(value)) + value + words(0)


def base_material(alpha_operation=3, alpha_argument=0):
    # Actual LayerStage::StreamPack order; alpha SELECTARG2 DIFFUSE.
    stage = words(0, 5, 2, 0, alpha_operation, 2, alpha_argument, 1, 1, 2, 2, 0, 0, 0)
    return (words(0x2b000002, 0, 1) + prop(3000, 3000, words(0x40000771)) +
            words(1, 0x110000, 0, 1) + stage + words(0, 5, 6, 4, 1, 2, 0))


class Archive:
    def __init__(self, alpha_argument=0):
        self.records = {
            0x3100034f: words(0x3100034f, 0x2b000002, 2, 1, 0x30000519),
            0x2b000002: base_material(alpha_argument=alpha_argument),
            0x30000519: (words(0x30000519, 2) + prop(3000, 3000, words(0)) +
                         prop(2000, 2000, struct.pack("<4f", 1, 0, 0, 0))),
        }

    def read(self, did):
        return self.records[did]


class AssetLoadingTests(unittest.TestCase):
    def test_uv_modifier_prefix_does_not_discard_foliage_alpha_state(self):
        # Native base2b0000de: UVScale(coord1), UVTranslate(coord1), then
        # source-alpha blending. Previously the modifier count stopped parsing.
        wave = words(1) + struct.pack("<10f",1,0,1,0,0,0,1,0,0,0)
        modifiers = words(2,6,1) + wave*2 + words(4,1) + wave*2
        stage = words(0,5,2,0,2,2,0,1,1,2,2,0,0,0)
        material = (words(0x2b0000de,0,0,1,0x110000,0,1) + stage +
                    modifiers + words(5,6,4,1,2,0))
        state = first_layer_state(material,0x2b0000de)
        self.assertTrue(state["texture_alpha"])
        self.assertEqual(state["alpha_sources"], ["texture"])
        self.assertEqual(state["uv_modifiers"],2)
        self.assertEqual((state["source_blend"],state["destination_blend"]),(5,6))
        with self.assertRaises(MaterialFormatError):
            first_layer_state(material[:-4],0x2b0000de)

    def test_static_definition_resolves_visual_without_catalog(self):
        # Observed native definition47000719 -> visual1f0003ee.
        self.assertEqual(entity_visual(words(0x0b,0x47000719,0x40000001,0x1f0003ee),0x47000719),0x1f0003ee)
        with self.assertRaises(ValueError):
            entity_visual(words(0x0b,0x47000719,0x40000001,0x40000001),0x47000719)
        with self.assertRaises(ValueError):
            entity_visual(words(0x0b,0x47000719,0x40000001),0x47000719)

    def test_explicit_null_texture_replaces_base_debug_texture(self):
        with patch("preview_material.texture_image", side_effect=AssertionError("No texture should be read")):
            info = material_info(Archive(), 0x3100034f)
        self.assertEqual(info["textures"][3000], 0)
        self.assertFalse(info["visible"])
        self.assertNotIn("image", info)
        self.assertNotIn("image_error", info)

    def test_texture_alpha_and_unknown_alpha_are_not_treated_as_invisible(self):
        for argument in (2, 0x10):
            self.assertTrue(material_info(Archive(alpha_argument=argument), 0x3100034f)["visible"])

    def test_helper_triangles_removed_from_render_and_pick_mesh(self):
        assets = object.__new__(Assets)
        from collections import OrderedDict
        assets.portal, assets.surfaces = Archive(), None
        assets.materials, assets.material_warnings = OrderedDict(), set()
        mesh = dict(materials=[0x3100034f, 0x31000001], groups=[
            dict(start=0, count=3, material=0), dict(start=3, count=3, material=1)],
            vertices=[(0,0,0)]*3, indices=[0,1,2,2,1,0], uvs=[])
        def info(portal, did, *args, **kwargs):
            return material_info(portal, did) if did == 0x3100034f else {"average":[20,30,40]}
        with patch("preview_material.material_info", side_effect=info):
            assets.color_mesh(mesh)
        self.assertEqual(mesh["indices"], [2,1,0])
        self.assertEqual(mesh["groups"], [dict(start=0, count=3, material=1)])
        self.assertEqual(mesh["triangle_colors"], [[20,30,40]])
        self.assertEqual(assets.material_warnings, set())

    def test_physics_only_room_mesh_has_no_render_materials_or_degrades(self):
        # This is the observed native shape of dungeon helper mesh0100009e.
        data = (words(0x0100009e, 2, 0, 2, 1) + struct.pack("<3f", 0,0,0) +
                struct.pack("<6f", 0,0,0,1,1,1) + struct.pack("<4f", 0,0,0,1) + words(0))
        mesh = decode_mesh(data)
        self.assertEqual(mesh["materials"], [])
        self.assertEqual(mesh["indices"], [])
        self.assertEqual(mesh["degrades"], [])
        with self.assertRaises(MeshFormatError):
            decode_mesh(data[:8] + words(1, 0x31000001) + data[12:])

    def test_world_only_scene_does_not_require_selected_visual(self):
        assets = object.__new__(Assets)
        assets.material_warnings = {"stale material warning"}
        assets.creature = lambda *args: self.fail("World-only scene requested creature")
        assets.animation = lambda *args: self.fail("World-only scene requested animation")
        class Cells:
            def read(self, cell):
                return (words(cell) + struct.pack("<3f",0,0,0) + struct.pack("<4f",1,0,0,0)
                        + words(0,0x0100009e,0,0,0))
        assets.cells = Cells()
        assets.mesh = lambda did: {"indices":[]}
        instances, warnings = assets.scene(0x02990130,None,[0,0,0],0,None,[],{})
        self.assertEqual(instances, [])
        self.assertEqual(warnings, [])
        self.assertEqual(assets.material_warnings, set())


if __name__ == "__main__":
    unittest.main()
