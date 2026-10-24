"""Direct manipulation math: axis picking, planar movement and group pivots."""
import math
import unittest

from editor_gizmo import descriptor, project, hit_axis, drag_options


class GizmoTests(unittest.TestCase):
    def setUp(self):
        v = math.sqrt(.5)
        self.camera = ((0, -10, 10), (0, v, -v), (1, 0, 0), (0, v, v))
        self.gizmo = descriptor([dict(selected=True, object_id=1, position=(0, 0, 0))], "move", self.camera, 800, 600)

    def test_axis_hit_and_drag_follow_projected_axis(self):
        end = project(self.gizmo["ends"][0], self.camera, 800, 600)
        self.assertEqual(hit_axis(*end, self.gizmo, self.camera, 800, 600), 0)
        result = drag_options("move", (400, 300), end, self.gizmo, self.camera, 800, 600, axis=0)
        self.assertAlmostEqual(result["translation"][0], self.gizmo["length"])
        self.assertEqual(result["translation"][1:], (0, 0))

    def test_planar_move_preserves_height(self):
        result = drag_options("move", (400, 300), (420, 310), self.gizmo, self.camera, 800, 600)
        self.assertGreater(result["translation"][0], 0)
        self.assertAlmostEqual(result["translation"][2], 0)

    def test_group_pivot_counts_each_object_once(self):
        objects = [dict(selected=True, object_id=1, position=(0, 0, 0))]*3 + [dict(selected=True, object_id=2, position=(10, 0, 0))]
        result = descriptor(objects, "rotate", self.camera, 800, 600)
        self.assertEqual(result["center"], (5, 0, 0))

    def test_rotation_and_scale_are_relative_to_drag_start(self):
        rotation = drag_options("rotate", (100, 100), (200, 100), self.gizmo, self.camera, 800, 600)
        scale = drag_options("scale", (100, 100), (100, 50), self.gizmo, self.camera, 800, 600)
        self.assertEqual(rotation["degrees"], 65)
        self.assertAlmostEqual(scale["scale"], math.exp(.6))
