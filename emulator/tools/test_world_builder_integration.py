"""Native terrain + dungeon + project + export integration on installed assets."""
from pathlib import Path
import tempfile
import unittest
from preview_assets import Assets
from world_project import WorldProject,OverlayDat,digest
from world_terrain import TerrainEditor
from world_dungeon import DungeonBuilder,parse_cell
from world_dat_export import export_archive
from ac2_dat import Dat

class NativeBuilderIntegration(unittest.TestCase):
    def test_native_geometry_save_reload_render_export(self):
        folder=Path("C:/Asheron's Call 2")
        if not (folder/'cell_1.dat').exists():self.skipTest('Installed native assets unavailable')
        assets=Assets(folder)
        self.addCleanup(assets.close)
        project=WorldProject(folder)
        assets.cells=OverlayDat(assets.cells,project)
        terrain=TerrainEditor(assets.cells,assets.portal)
        mesh=terrain.mesh(0x74baffff)
        point=list(mesh['vertices'][0])
        edits=terrain.stroke(0x74ba0001,point,radius=20,strength=15,mode='raise')
        self.assertTrue(edits)
        before,after=project.apply(edits)
        self.assertNotEqual(terrain.mesh(0x74baffff)['vertices'],mesh['vertices'])
        builder=DungeonBuilder(assets.cells,assets.portal)
        project.apply(builder.seed(0x0299010d,0x7ffe))
        self.assertTrue(builder.validate(0x7ffe))
        project.apply(builder.attach(0x7ffe0100,0,0x0299010d,0))
        self.assertEqual(builder.validate(0x7ffe),[])
        instances,warnings=assets.scene(0x7ffe0100,0,[0,0,0],0,1,[],{})
        self.assertEqual({row['cell'] for row in instances},{0x7ffe0100,0x7ffe0101})
        self.assertTrue(all(row['mesh']['indices'] for row in instances))
        with tempfile.TemporaryDirectory() as name:
            manifest=Path(name)/'world.json';project.save(manifest)
            restored=WorldProject.load(manifest)
            self.assertEqual(restored.patches,project.patches)
            destination=Path(name)/'cell_1.dat'
            export_archive(folder/'cell_1.dat',destination,restored.patches,{did:digest(restored.originals[did]) for did in restored.patches})
            with Dat(destination) as cells:
                check=DungeonBuilder(cells,assets.portal)
                self.assertEqual(check.validate(0x7ffe),[])
                for did,data in restored.patches.items():self.assertEqual(cells.read(did),data)
                native=TerrainEditor(cells,assets.portal)
                self.assertEqual(native.mesh(0x74baffff)['vertices'],terrain.mesh(0x74baffff)['vertices'])
        project.restore(before)
        self.assertEqual(terrain.mesh(0x74baffff)['vertices'],mesh['vertices'])

    def test_custom_mesh_and_paired_entrances_export_both_archives(self):
        from world_room import build_room
        from world_entrance import make_pair,endpoint
        from world_bundle import export_bundle
        folder=Path("C:/Asheron's Call 2")
        if not (folder/'cell_1.dat').exists():self.skipTest('Installed native assets unavailable')
        assets=Assets(folder);self.addCleanup(assets.close)
        project=WorldProject(folder)
        result=build_room(assets.cells,assets.portal,width=40,depth=40,height=24,doors=(),target_block=0x7ffd)
        project.apply_bundle({key:result[key] for key in ('cell_1.dat','portal.dat')})
        project.set_entrances([make_pair([],endpoint(0x74ba0001,[10,10,5]),endpoint(result['cell'],[0,0,0]),endpoint(result['cell'],[10,0,0]),endpoint(0x74ba0001,[25,10,5]))])
        assets.cells=OverlayDat(assets.cells,project);assets.portal=OverlayDat(assets.portal,project,'portal.dat')
        instances,warnings=assets.scene(result['cell'],0,[0,0,0],0,1,[],{})
        self.assertTrue(instances);self.assertFalse(warnings)
        from editor_locations import resolve_location
        anchor=resolve_location(folder,result["cell"],cells=assets.cells,portal=assets.portal,mesh_loader=assets.mesh)
        self.assertTrue(all(abs(value)<20 for value in anchor[:2]))
        self.assertAlmostEqual(anchor[2],0,places=3)
        self.assertEqual(DungeonBuilder(assets.cells,assets.portal).validate(0x7ffd),[])
        with tempfile.TemporaryDirectory() as name:
            dest=Path(name)/'bundle'
            report=export_bundle(project,dest,'pyrealium-world-v2\n')
            self.assertEqual(set(report['archives']),{'cell_1.dat','portal.dat'})
            self.assertEqual(len((dest/'world.routes.tsv').read_text(encoding='utf-8').splitlines()),3)
            self.assertEqual(len((dest/'world.tsv').read_text(encoding='utf-8').splitlines()),3)
            loaded=WorldProject.load(dest/'world.world.json')
            self.assertEqual(loaded.signature(),project.signature())
            with Dat(dest/'cell_1.dat') as cells,Dat(dest/'portal.dat') as portal:
                self.assertEqual(DungeonBuilder(cells,portal).validate(0x7ffd),[])
                self.assertEqual(portal.read(result['mesh']),project.portal_patches[result['mesh']])

    def test_reexport_content_with_identical_entrances_does_not_duplicate_objects(self):
        from world_entrance import make_pair,endpoint
        from world_bundle import export_bundle
        folder=Path("C:/Asheron's Call 2")
        if not (folder/'cell_1.dat').exists():self.skipTest('Installed native assets unavailable')
        project=WorldProject(folder)
        project.set_entrances([make_pair([],endpoint(0x74ba0001,[10,10,0]),endpoint(0x02990109,[40,250,-44]),endpoint(0x02990109,[50,250,-44]),endpoint(0x74ba0001,[25,10,0]))])
        with tempfile.TemporaryDirectory() as name:
            first=Path(name)/'first';second=Path(name)/'second'
            export_bundle(project,first,'pyrealium-world-v2\n')
            content=(first/'world.tsv').read_text(encoding='utf-8')
            export_bundle(project,second,content)
            self.assertEqual((second/'world.tsv').read_text(encoding='utf-8'),content)
