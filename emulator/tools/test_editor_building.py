"""Building control integration: native project history and clicked entrance anchors."""
import unittest
from unittest.mock import patch
import test_content_editor as workflows

class BuildingWorkflowTests(unittest.TestCase):
    setUp=workflows.EditorWorkflowTests.setUp
    cleanup_editor=workflows.EditorWorkflowTests.cleanup_editor

    def test_custom_room_and_mesh_share_one_undo(self):
        g=self.editor.geometry;b=g.building
        result={'cell_1.dat':{0x7ffc0100:(None,b'cell')},'portal.dat':{0x01f00000:(None,b'mesh')},'cell':0x7ffc0100}
        with patch.object(self.editor,'navigate_to') as travel:
            b.finish_job('build:custom',result)
            travel.assert_called_once_with(0x7ffc0100)
        self.assertEqual(len(self.editor.history),1)
        self.assertTrue(g.project.portal_patches)
        self.editor.undo()
        self.assertFalse(g.project.patches);self.assertFalse(g.project.portal_patches)
        self.editor.redo()
        self.assertEqual(g.project.portal_patches[0x01f00000],b'mesh')

    def test_entrance_pair_creation_undo_and_id_allocation(self):
        from world_entrance import endpoint
        b=self.editor.geometry.building
        b.anchors={'outside_portal':endpoint(0x74ba0001,[10,10,0]),'outside_arrival':endpoint(0x74ba0001,[25,10,0]),'inside_portal':endpoint(0x7ffc0100,[0,0,0]),'inside_arrival':endpoint(0x7ffc0100,[15,0,0])}
        b.create_entrance()
        self.assertEqual(len(self.editor.geometry.project.entrances),1)
        self.assertEqual(len(self.editor.history),1)
        self.editor.undo();self.assertEqual(self.editor.geometry.project.entrances,[])
        self.editor.redo();self.assertEqual(len(self.editor.geometry.project.entrances),1)

    def test_click_capture_canonicalizes_outdoor_area_and_rejects_close_arrival(self):
        b=self.editor.geometry.building
        self.editor.active_scene_cell=0x74ba0001
        self.editor.outdoor_grid={'block_side':320,'cell_side':40,'cells':8}
        b.capture=('outside','portal')
        self.assertTrue(b.capture_surface({'point':[330,10,5],'cell':None}))
        self.assertEqual(b.anchors['outside_portal']['cell'],0x75ba0001)
        self.assertEqual(b.anchors['outside_portal']['position'],[10,10,5])
        self.assertFalse(b.capture_surface({'point':[331,10,5],'cell':None}))
        self.assertNotIn('outside_arrival',b.anchors)
        with patch.object(b,'entrance_dialog'):
            self.assertTrue(b.capture_surface({'point':[345,10,5],'cell':None}))
        self.assertIsNone(b.capture)

    def test_free_block_skips_existing_dungeons(self):
        from types import SimpleNamespace
        b=self.editor.geometry.building
        builder=SimpleNamespace(templates=lambda:[0xf1000100,0xf1010100])
        self.assertEqual(b.free_block(builder,0xf100),0xf102)

    def test_build_tab_save_routes_to_project_while_exploring(self):
        self.editor.library.select(self.editor.page_geometry)
        self.editor.view3d.set_tool('explore')
        with patch.object(self.editor.geometry,'save') as save:
            self.editor.save();save.assert_called_once_with()
