"""Location inventory and geometry navigation checks; isolated, read-only data."""
from pathlib import Path
import math
import tempfile
import unittest
from unittest.mock import patch

from editor_locations import build_locations, floor_anchor, location_kind, location_links, resolve_location, room_below


class FakeDat:
    def __init__(self, path):
        self.path = Path(path)

    def __enter__(self):
        return self

    def __exit__(self, *unused):
        pass

    def index(self):
        if self.path.name == "cell_1.dat":
            return {0x1234ffff: (1, 100, 0), 0x02990109: (2, 100, 0), 0x02990110: (3, 100, 0)}
        return {0x01000001: (4, 100, 0)}

    def read(self, did):
        return b"unused"


class LocationTests(unittest.TestCase):
    def test_inventory_named_anchors_and_missing_records(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder)
            (path / "world.tsv").write_text(
                "pyrealium-world-v2\n"
                "8140000000000001\t47000001\t1f000001\t12340010\t10\t20\t30\t1\t0\t0\t0\n"
                "8140000000000002\t47000001\t1f000001\t99990100\t1\t2\t3\t1\t0\t0\t0\n", encoding="utf-8")
            (path / "portal-routes.tsv").write_text(
                "pyrealium-portal-routes-v1\n"
                "8140000000010001\t47000001\t12340010\t1\t2\t3\t02990109\t4\t5\t6\t0\t1\n", encoding="utf-8")
            with patch("editor_locations.Dat", FakeDat):
                rows = build_locations(path, path, [("47000001", "1f000001", "Test portal")])
            indexed = {row["cell"]: row for row in rows}
            self.assertEqual(indexed[0x12340010]["name"], "Test portal")
            self.assertEqual(indexed[0x12340010]["position"], [10, 20, 30])
            self.assertEqual(indexed[0x02990109]["position"], [4, 5, 6])
            self.assertIsNone(indexed[0x02990110]["position"])
            self.assertIn(0x12340001, indexed)
            self.assertNotIn(0x99990100, indexed)

    def test_floor_anchor_avoids_ceiling_and_walls(self):
        # Same footprint on floor and ceiling, plus a vertical wall.
        vertices = [[0, 0, 2], [6, 0, 2], [0, 6, 2],
                    [0, 0, 12], [6, 0, 12], [0, 6, 12]]
        indices = [3, 4, 5, 0, 1, 3, 0, 1, 2]
        self.assertEqual(floor_anchor(vertices, indices, lower_half=True), [2, 2, 2])

    def test_floor_anchor_rejects_void(self):
        with self.assertRaisesRegex(ValueError, "no render triangles"):
            floor_anchor([[0, 0, 0]], [])
        with self.assertRaisesRegex(ValueError, "No floor-like"):
            floor_anchor([[0, 0, 0], [1, 0, 0], [0, 0, 1]], [0, 1, 2])

    def test_room_anchor_uses_native_rotation_and_translation(self):
        room = dict(cell=0x02990109, mesh=0x01000001, position=[10, 20, -30],
                    quaternion=[math.sqrt(.5), 0, 0, math.sqrt(.5)])
        mesh = dict(vertices=[[0, 0, 0], [6, 0, 0], [0, 6, 0]], indices=[0, 1, 2])
        with patch("editor_locations.Dat", FakeDat), patch("preview_cell.decode_cell", return_value=room), patch("editor_locations._room_mesh", return_value=mesh):
            point = resolve_location(Path("unused"), 0x02990109)
        for actual, expected in zip(point, [8, 22, -30]):
            self.assertAlmostEqual(actual, expected)

    def test_room_floor_band_beats_overhead_center_platform(self):
        vertices = [[0, 0, 0], [6, 0, 0], [0, 6, 0],
                    [1, 1, 18], [5, 1, 18], [1, 5, 18],
                    [0, 0, 60], [6, 0, 60], [0, 6, 60]]
        result = floor_anchor(vertices, [3, 4, 5, 0, 1, 2, 6, 7, 8],
                              target_xy=[2, 2], lower_half=True, target_z=0)
        self.assertEqual(result, [2, 2, 0])

    def test_outdoor_navigation_respects_requested_cell_and_holes(self):
        # The requested cell is the second Y cell. A missing triangle there
        # must not silently move its anchor into a neighboring cell.
        definitions = dict(cell_side=10, cells=2)
        mesh = dict(vertices=[[0, 0, 2], [10, 0, 2], [0, 10, 2],
                              [0, 10, 3], [10, 10, 3], [0, 20, 3]],
                    indices=[0, 1, 2, 3, 4, 5])
        class TerrainDat(FakeDat):
            def index(self):
                return {0x1234ffff: (1, 100, 0), 0x0f000000: (2, 100, 0), 0x36000001: (3, 100, 0)}
        with patch("editor_locations.Dat", TerrainDat), patch("preview_terrain.region_references", return_value={"land_defs": 0x36000001}), patch("preview_terrain.landscape_defs", return_value=definitions), patch("preview_terrain.decode_terrain", return_value=mesh):
            point = resolve_location(Path("unused"), 0x12340002)
            self.assertEqual(point[2], 3)
            self.assertTrue(10 <= point[1] < 20)
            mesh["indices"] = [0, 1, 2]
            with self.assertRaisesRegex(ValueError, "no render triangles"):
                resolve_location(Path("unused"), 0x12340002)

    def test_reserved_identifiers_rejected(self):
        for cell in (0, 0x12340000, 0x12340065, 0x1234ffff, -1, 0x100000001):
            with self.assertRaises(ValueError):
                location_kind(cell)

    def test_outdoor_links_only_existing_cardinal_blocks(self):
        class NeighborDat(FakeDat):
            def index(self):
                return {0x1234ffff: (1, 100, 0), 0x1235ffff: (2, 100, 0), 0x1134ffff: (3, 100, 0)}
        with patch("editor_locations.Dat", NeighborDat):
            rows = location_links(Path("unused"), 0x12340010)
        self.assertEqual([r["cell"] for r in rows], [0x12350010, 0x11340010])
        self.assertTrue(rows[0]["name"].startswith("North"))
        self.assertTrue(rows[1]["name"].startswith("West"))
        self.assertTrue(all(r["position"] is None for r in rows))

    def test_interior_links_prioritize_actual_doorways_and_deduplicate(self):
        room = dict(cell=0x02990109, portals=[{"other_cell": 0x02990110}, {"other_cell": 0}, {"other_cell": 0x99990100}],
                    visible_cells=[0x02990110, 0x02990109], shared_cells=[0x02990110])
        with patch("editor_locations.Dat", FakeDat), patch("preview_cell.decode_cell", return_value=room):
            rows = location_links(Path("unused"), 0x02990109)
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["cell"], 0x02990110)
        self.assertEqual(rows[0]["connection"], "Doorway")

    def test_room_below_follows_adjacent_visible_floor_polygons(self):
        first = dict(role="world", cell=0x02990109,
                     vertices=[[0, 0, 0], [10, 0, 0], [0, 10, 0]], indices=[0, 1, 2])
        second = dict(role="world", cell=0x02990110,
                      vertices=[[10, 0, 2], [20, 0, 2], [10, 10, 2]], indices=[0, 1, 2])
        self.assertEqual(room_below([2, 2, 5], [first, second]), first["cell"])
        self.assertEqual(room_below([12, 2, 5], [first, second]), second["cell"])
        self.assertIsNone(room_below([30, 30, 5], [first, second]))
        self.assertIsNone(room_below([2, 2, 30], [first, second]))

    def test_room_below_rejects_overhead_ceilings_walls_and_object_meshes(self):
        floor = dict(role="world", cell=0x02990109,
                     vertices=[[0, 0, 0], [10, 0, 0], [0, 10, 0]], indices=[0, 1, 2])
        ceiling = dict(role="world", cell=0x02990110,
                       vertices=[[0, 0, 10], [10, 0, 10], [0, 10, 10]], indices=[0, 1, 2])
        wall = dict(role="world", cell=0x02990111,
                    vertices=[[0, 0, 0], [10, 0, 0], [0, 0, 10]], indices=[0, 1, 2])
        object_mesh = dict(floor, role="saved", cell=0x02990112)
        self.assertEqual(room_below([2, 2, 5], [ceiling, wall, object_mesh, floor]), floor["cell"])
        self.assertIsNone(room_below([2, 2, 5], [ceiling, wall, object_mesh]))
        self.assertIsNone(room_below([2, 2, -1], [floor]))
        self.assertEqual(room_below([2, 2, -.2], [floor]), floor["cell"])

    def test_room_below_chooses_nearest_sloping_floor_and_handles_degeneracy(self):
        low = dict(role="world", cell=0x02990109,
                   vertices=[[0, 0, 0], [10, 0, 0], [0, 10, 0]], indices=[0, 1, 2])
        ramp = dict(role="world", cell=0x02990110,
                    vertices=[[0, 0, 2], [10, 0, 4], [0, 10, 2]], indices=[0, 1, 2])
        degenerate = dict(role="world", cell=0x02990111,
                          vertices=[[0, 0, 0], [0, 0, 0], [0, 0, 0]], indices=[0, 1, 2])
        self.assertEqual(room_below([2, 2, 5], [low, ramp, degenerate]), ramp["cell"])
        self.assertIsNone(room_below([float("nan"), 2, 5], [low]))


if __name__ == "__main__":
    unittest.main()
