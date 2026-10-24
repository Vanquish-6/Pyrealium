"""Project transactions and persistence against an independent native DAT fixture."""
from pathlib import Path
import tempfile
import unittest
from ac2_dat import Dat
from world_project import WorldProject,OverlayDat
from test_world_dat_export import fixture

class ProjectTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.folder=Path(self.temp.name)
        self.payloads=fixture(self.folder/'cell_1.dat')
        self.project=WorldProject(self.folder)
        self.did=0x53000004

    def test_save_reopen_overlay_and_undo_roundtrip(self):
        old=self.payloads[self.did]
        before,after=self.project.apply({self.did:(old,b'edited'),0x53000100:(None,b'room')})
        path=self.folder/'world.json';self.project.save(path)
        restored=WorldProject.load(path)
        self.assertFalse(restored.dirty)
        self.assertEqual(restored.patches,self.project.patches)
        with Dat(self.folder/'cell_1.dat') as base:
            overlay=OverlayDat(base,restored)
            self.assertEqual(overlay.read(self.did),b'edited')
            self.assertIn(0x53000100,overlay.index())
            self.assertEqual(base.read(self.did),old)
        self.project.restore(before)
        self.assertEqual(self.project.patches,{})
        self.project.restore(after)
        self.assertFalse(self.project.dirty)

    def test_stale_edit_is_atomic(self):
        self.project.apply({self.did:(self.payloads[self.did],b'edited')})
        before=dict(self.project.patches)
        with self.assertRaises(ValueError):
            self.project.apply({0x53000100:(None,b'new'),self.did:(b'wrong',b'wrong')})
        self.assertEqual(self.project.patches,before)
        self.assertNotIn(0x53000100,self.project.originals)

    def test_wrong_source_and_original_deletion_refused(self):
        self.project.apply({self.did:(b'wrong source',b'edited')})
        path=self.folder/'world.json';self.project.save(path)
        with self.assertRaisesRegex(ValueError,'source differs'):WorldProject.load(path)
        project=WorldProject(self.folder)
        with self.assertRaises(ValueError):project.apply({self.did:(self.payloads[self.did],None)})
        self.assertEqual(project.patches,{})

    def test_multi_archive_room_is_atomic_and_reversible(self):
        fixture(self.folder/'portal.dat')
        before,after=self.project.apply_bundle({'cell_1.dat':{0x53000100:(None,b'cell')},'portal.dat':{0x01f00000:(None,b'mesh')}})
        state=self.project.signature()
        with self.assertRaises(ValueError):
            self.project.apply_bundle({'cell_1.dat':{0x53000101:(None,b'another')},'portal.dat':{0x01f00000:(b'stale',b'bad')}})
        self.assertEqual(self.project.signature(),state)
        self.project.restore(before)
        self.assertFalse(self.project.patches);self.assertFalse(self.project.portal_patches)
        self.project.restore(after)
        path=self.folder/'multi.json';self.project.save(path)
        loaded=WorldProject.load(path)
        self.assertEqual(loaded.signature(),self.project.signature())
        with Dat(self.folder/'portal.dat') as base:
            self.assertEqual(OverlayDat(base,loaded,'portal.dat').read(0x01f00000),b'mesh')
