"""Picking/camera and native input bridge regressions without requiring GL."""
from collections import deque
import ctypes
from types import SimpleNamespace
import tkinter as tk
import unittest
from unittest.mock import MagicMock

from preview_gl import GLRenderer, material_rgba, sample_alpha, surface_hit_visible, material_draw_commands
from preview_view import Preview3D


class NativeInputTests(unittest.TestCase):
    def setUp(self):
        self.renderer = GLRenderer.__new__(GLRenderer)
        self.renderer.input_events = deque(maxlen=128)
        self.renderer.u = MagicMock()
        self.renderer.u.GetKeyState.return_value = 0

    def test_mouse_capture_and_signed_drag_coordinates(self):
        r = self.renderer
        r._queue_input(123, 0x201, 1, (20 << 16) | 15)
        r.u.SetCapture.assert_called_once_with(123)
        r._queue_input(123, 0x200, 1, (20 << 16) | 0xfffe)
        r._queue_input(123, 0x202, 0, (20 << 16) | 0xfffe)
        events = r.consume_input()
        self.assertEqual(events[0][:4], ("<ButtonPress-1>", 15, 20, 0x100))
        self.assertEqual(events[1][:3], ("<Motion>", -2, 20))
        self.assertEqual(events[2][0], "<ButtonRelease-1>")
        r.u.ReleaseCapture.assert_called_once()
        self.assertEqual(r.consume_input(), ())

    def test_motion_coalesces_without_losing_button_events(self):
        r = self.renderer
        r._queue_input(123, 0x201, 1, 0)
        for x in range(1000):
            r._queue_input(123, 0x200, 1, x)
        self.assertEqual(len(r.input_events), 2)
        self.assertEqual(r.consume_input()[1][1], 999)

    def test_shift_keyboard_modifier(self):
        r = self.renderer
        r.u.GetKeyState.side_effect = lambda vk: -1 if vk == 0x10 else 0
        r._queue_input(123, 0x100, 0x25, 0)
        self.assertEqual(r.consume_input(), (("<KeyPress>", 0, 0, 1, 0, "Left"),))

    def test_native_key_release_and_focus_loss_are_forwarded(self):
        r = self.renderer
        r._queue_input(123, 0x100, ord("W"), 0)
        r._queue_input(123, 0x101, ord("W"), 0)
        r._queue_input(123, 0x8, 0, 0)
        self.assertEqual([event[0] for event in r.consume_input()], ["<KeyPress>", "<KeyRelease>", "<FocusOut>"])


class SurfaceAlphaTests(unittest.TestCase):
    def setUp(self):
        self.item = dict(indices=[0, 1, 2], uvs=[(.25, .25)] * 3,
            groups=[dict(start=0, count=3, material=0)],
            material_diffuse={0: [.5, .75, 1., 1.]},
            material_states={0: dict(texture_alpha=True, alpha_sources=["texture", "diffuse"],
                source_blend=5, destination_blend=6, alpha_test=False)},
            material_images={0: dict(width=2, height=2, alpha=bytes([0, 255, 255, 255]))})

    def test_native_float_rgba_is_not_divided_by_255(self):
        self.assertEqual(material_rgba(self.item, 0), (.5, .75, 1., 1.))

    def test_fallback_color_is_bytes(self):
        self.assertEqual(material_rgba(dict(material_colors={0: [255, 0, 255]}), 0), (1., 0., 1., 1.))

    def test_transparent_foliage_texel_does_not_intercept_pick(self):
        self.assertFalse(surface_hit_visible(self.item, 0, .2, .3))
        self.item["uvs"] = [(.75, .25)] * 3
        self.assertTrue(surface_hit_visible(self.item, 0, .2, .3))

    def test_gl_linear_repeat_alpha_matches_pixel_centers(self):
        image = self.item["material_images"][0]
        self.assertEqual(sample_alpha(image, (.25, .25)), 0.)
        self.assertEqual(sample_alpha(image, (1.25, -.75)), 0.)
        self.assertEqual(sample_alpha(image, (.5, .25)), .5)

    def test_alpha_test_uses_interpolated_texel_and_native_reference(self):
        self.item["uvs"] = [(.5, .25)] * 3
        self.item["material_states"][0].update(alpha_test=True, alpha_reference=.6)
        self.assertFalse(surface_hit_visible(self.item, 0, .2, .3))
        self.item["material_states"][0]["alpha_reference"] = .4
        self.assertTrue(surface_hit_visible(self.item, 0, .2, .3))

    def test_unknown_shader_does_not_prove_pixel_invisible(self):
        self.item["material_states"][0]["alpha_sources"] = ["unknown"]
        self.assertTrue(surface_hit_visible(self.item, 0, .2, .3))

    def test_mixed_material_groups_sort_opaque_then_far_to_near_blends(self):
        blended = dict(source_blend=5, destination_blend=6, depth_write=True)
        near = dict(vertices=[(0, 0, 1)] * 3, indices=[0, 1, 2, 0, 1, 2],
            groups=[dict(start=0, count=3, material=0), dict(start=3, count=3, material=1)],
            material_states={1: blended})
        far = dict(vertices=[(0, 0, 10)] * 3, indices=[0, 1, 2],
            groups=[dict(start=0, count=3, material=2)], material_states={2: blended})
        commands = material_draw_commands([near, far], (0, 0, 0))
        self.assertEqual([group["material"] for item, group in commands], [0, 2, 1])
        self.assertTrue(blended["depth_write"])
        # Moving the camera reverses the blended order; cached static centroids
        # retain the same geometry while distances are recomputed each frame.
        commands = material_draw_commands([near, far], (0, 0, 20))
        self.assertEqual([group["material"] for item, group in commands], [0, 1, 2])


class ViewportTests(unittest.TestCase):
    def setUp(self):
        self.root = tk.Tk()
        self.root.withdraw()
        self.picked = []
        self.view = Preview3D(self.root, on_pick=self.picked.append)
        self.view.canvas.winfo_width = lambda: 800
        self.view.canvas.winfo_height = lambda: 600
        self.view.focus = (0, 0, 0)
        self.view.instances = [dict(vertices=[(-10, -10, 0), (10, -10, 0), (0, 10, 0)],
                                    indices=[0, 1, 2], triangle_colors=[], role="world", cell=0x02990109)]
        self.addCleanup(self.cleanup)

    def cleanup(self):
        self.view.close()
        for callback in self.root.tk.call("after", "info"):
            self.root.after_cancel(callback)
        self.root.destroy()

    def test_click_in_place_tool_hits_surface_and_retains_cell(self):
        self.view.orbit_start(SimpleNamespace(x=400, y=300, state=0))
        self.assertEqual(len(self.picked), 1)
        for value in self.picked[0]:
            self.assertAlmostEqual(value, 0, places=7)
        self.assertEqual(self.view.picked_cell, 0x02990109)
        self.assertTrue(self.view.placement_drag)

    def test_place_drag_repeats_picking_and_release_ends_it(self):
        self.view.orbit_start(SimpleNamespace(x=400, y=300, state=0))
        self.view.orbit_drag(SimpleNamespace(x=410, y=300, state=0x100))
        self.assertEqual(len(self.picked), 2)
        self.assertNotEqual(self.picked[0], self.picked[1])
        self.view.end_drag()
        self.assertFalse(self.view.placement_drag)

    def test_click_placement_calls_stamp_only_after_a_surface_hit(self):
        stamps = []
        self.view.on_stamp = lambda: stamps.append(True)
        self.view.placement_enabled = True
        self.view.orbit_start(SimpleNamespace(x=400, y=300, state=0))
        self.assertEqual(stamps, [True])
        self.view.instances = []
        self.view.orbit_start(SimpleNamespace(x=400, y=300, state=0))
        self.assertEqual(stamps, [True])

    def test_move_drag_has_one_begin_preview_commit_sequence(self):
        actions = []
        self.view.on_transform = lambda tool, options, phase: actions.append((tool, phase)) or True
        self.view.instances[0].update(position=(0,0,0), selected=True, object_id=1)
        self.view.set_tool("move")
        self.view.orbit_start(SimpleNamespace(x=400,y=300,state=0))
        self.view.orbit_drag(SimpleNamespace(x=430,y=310,state=0x100))
        self.view.end_drag()
        self.assertEqual(actions, [("move","begin"),("move","preview"),("move","commit")])

    def test_escape_cancels_an_active_rotation_drag(self):
        actions = []
        self.view.on_transform = lambda tool, options, phase: actions.append(phase) or True
        self.view.instances[0].update(position=(0,0,0), selected=True, object_id=1)
        self.view.set_tool("rotate")
        self.view.orbit_start(SimpleNamespace(x=400,y=300,state=0))
        self.view.orbit_drag(SimpleNamespace(x=430,y=300,state=0x100))
        self.view.key_input("Escape")
        self.assertEqual(actions, ["begin","preview","cancel"])

    def test_alt_drag_orbits_instead_of_moving_placement(self):
        self.view.orbit_start(SimpleNamespace(x=400, y=300, state=8))
        prior = self.view.azimuth
        self.view.orbit_drag(SimpleNamespace(x=410, y=300, state=8))
        self.assertNotEqual(self.view.azimuth, prior)
        self.assertEqual(self.picked, [])

    def test_fine_nudge_uses_scene_space_and_current_anchor(self):
        self.view.placement = (10, 20, 30)
        self.view.picked_cell = 0x12340100
        self.view.nudge(SimpleNamespace(state=1), (1, 0, 0))
        self.assertEqual(self.picked, [(10.1, 20, 30)])
        self.assertIsNone(self.view.picked_cell)

    def test_frame_object_uses_actual_bounds(self):
        self.view.instances.append(dict(role="preview", vertices=[(1, 2, 3), (3, 4, 5)]))
        self.view.focus_placement()
        self.assertEqual(self.view.focus, (2, 3, 4))
        self.assertLess(self.view.distance, 10)

    def test_explore_flight_moves_camera_without_editing_placement(self):
        self.view.set_tool("explore")
        self.view.placement = (10, 20, 30)
        original_focus = self.view.focus
        original_anchor = self.view.placement
        moved = []
        self.view.on_camera_move = moved.append
        self.view.explore_key(SimpleNamespace(state=0), (0, 1, 0))
        self.assertNotEqual(self.view.focus, original_focus)
        self.assertEqual(self.view.placement, original_anchor)
        self.assertEqual(self.picked, [])
        self.assertEqual(moved, [self.view.focus])

    def test_explore_arrows_never_nudge_the_selected_object(self):
        self.view.set_tool("explore")
        self.view.nudge(SimpleNamespace(state=0), (1, 0, 0))
        self.assertEqual(self.picked, [])
        self.assertNotEqual(self.view.focus, (0, 0, 0))

    def test_explore_mouse_look_keeps_camera_eye_stationary(self):
        self.view.set_tool("explore")
        prior = self.view.get_navigation_point()
        self.view.orbit_start(SimpleNamespace(x=400, y=300, state=0))
        self.view.orbit_drag(SimpleNamespace(x=425, y=305, state=0))
        for a, b in zip(prior, self.view.get_navigation_point()):
            self.assertAlmostEqual(a, b, places=7)
        self.assertEqual(self.picked, [])

    def test_select_tool_hits_authored_object_without_moving_it(self):
        selected = []
        self.view.on_select = lambda identity, additive: selected.append((identity, additive))
        self.view.instances[0].update(role="saved", object_id=123, did=1)
        self.view.set_tool("select")
        self.view.orbit_start(SimpleNamespace(x=400, y=300, state=4))
        self.assertEqual(selected, [(123, True)])
        self.assertEqual(self.picked, [])

    def test_native_keys_move_directly_without_tk_key_focus(self):
        self.view.set_tool("explore")
        self.view.after_cancel(self.view.paint_poll_pending)
        self.view.paint_poll_pending = None
        before = self.view.focus
        self.view.gpu = MagicMock()
        self.view.gpu.consume_repaint.return_value = False
        self.view.gpu.consume_input.return_value = [("<KeyPress>", 0, 0, 0, 0, "w"), ("<KeyRelease>", 0, 0, 0, 0, "w")]
        self.view.poll_native_paint()
        self.assertNotEqual(before, self.view.focus)
        self.assertEqual(self.view.held_keys, {})
        self.assertEqual(self.picked, [])
        self.view.gpu = None

    def test_opposite_held_keys_do_not_drift_upward(self):
        self.view.set_tool("explore")
        self.view.held_keys = {"w": ((0, 1, 0), 0), "s": ((0, -1, 0), 0)}
        before = self.view.focus
        self.view.advance_travel(.1)
        self.assertEqual(self.view.focus, before)

    def test_selection_does_not_reach_through_nearer_world_geometry(self):
        selected = []
        self.view.on_select = lambda identity, additive: selected.append(identity)
        target = dict(self.view.instances[0], role="saved", object_id=123, did=1)
        target["vertices"] = [(x, y, z-2) for x, y, z in target["vertices"]]
        self.view.instances.append(target)
        self.view.set_tool("select")
        self.view.orbit_start(SimpleNamespace(x=400, y=300, state=0))
        self.assertEqual(selected, [])

    def test_arrangement_preview_cancel_restores_exact_instance_state(self):
        mesh = dict(vertices=[(0, 0, 0), (1, 0, 0), (0, 1, 0)], indices=[0, 1, 2], did=1)
        self.view.set_scene([dict(mesh=mesh, position=(1, 2, 3), role="saved", object_id=123)], (1, 2, 3))
        source = [f"{123:016x}", "47000001", "1f000001", "02990109", "1", "2", "3", "1", "0", "0", "0"]
        target = list(source)
        target[4:7] = ["11", "12", "13"]
        before = dict(self.view.instances[0])
        self.view.preview_arrangement([target], [source], 0x02990109)
        self.assertEqual(self.view.instances[0]["position"], (11, 12, 13))
        self.view.clear_arrangement()
        self.assertEqual(self.view.instances[0], before)

    def test_duplicate_preview_can_be_canceled_without_extra_instances(self):
        mesh = dict(vertices=[(0, 0, 0), (1, 0, 0), (0, 1, 0)], indices=[0, 1, 2], did=1)
        self.view.set_scene([dict(mesh=mesh, position=(1, 2, 3), role="saved", object_id=123)], (1, 2, 3))
        source = [f"{123:016x}", "47000001", "1f000001", "02990109", "1", "2", "3", "1", "0", "0", "0"]
        self.view.preview_arrangement([source, source], [source], 0x02990109, duplicate=True)
        self.assertEqual(len(self.view.instances), 3)
        self.view.clear_arrangement()
        self.assertEqual(len(self.view.instances), 1)


if __name__ == "__main__":
    unittest.main()
