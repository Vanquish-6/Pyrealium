"""Tk workflow integration using isolated files and mocked DAT reads."""
import os
import tkinter as tk
import unittest
from concurrent.futures import Future
from unittest.mock import patch, MagicMock

from content_editor import Editor
from editor_document import read_document, rows_from_text
import test_editor_document as fixtures

REAL_REFRESH = Editor.refresh_scene


class EditorWorkflowTests(unittest.TestCase):
    def setUp(self):
        fixtures.DocumentTests.setUp(self)
        self.args.output = self.path
        self.args.game_data = self.folder
        self.env = patch.dict(os.environ, {"LOCALAPPDATA": str(self.folder)})
        self.env.start()
        self.addCleanup(self.env.stop)
        self.refresh = patch.object(Editor, "refresh_scene")
        self.refresh.start()
        self.addCleanup(self.refresh.stop)
        self.root = tk.Tk()
        self.root.withdraw()
        self.editor = Editor(self.root, self.args)
        self.addCleanup(self.cleanup_editor)
        self.dialog = patch("content_editor.messagebox.showerror", side_effect=AssertionError("Unexpected error dialog"))
        self.dialog.start()
        self.addCleanup(self.dialog.stop)

    def cleanup_editor(self):
        self.editor.closing = True
        self.editor.view3d.close()
        self.editor.executor.shutdown(wait=True)
        for callback in self.root.tk.call("after", "info"):
            self.root.after_cancel(callback)
        self.root.destroy()

    def select(self):
        self.editor.objects.selection_set(self.row[:16])
        self.editor.choose_position(None)

    def test_edit_save_undo_redo_workflow(self):
        self.select()
        self.assertEqual(self.editor.fields["Heading"].get(), "64")
        self.editor.fields["X"].set("45")
        self.editor.save()
        rows = rows_from_text(read_document(self.path))
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0][0], self.row[:16])
        self.assertEqual(rows[0][4], "45")
        self.assertEqual(rows[0][7:11], self.row.split("\t")[7:11])
        self.editor.undo()
        self.assertEqual(read_document(self.path), self.original)
        self.editor.redo()
        self.assertEqual(rows_from_text(read_document(self.path))[0][4], "45")

    def test_duplicate_then_delete(self):
        self.select()
        self.editor.new_placement()
        self.assertIsNone(self.editor.selected_id)
        self.editor.save()
        self.assertEqual(len(rows_from_text(read_document(self.path))), 2)
        with patch("content_editor.messagebox.askyesno", return_value=True):
            self.editor.delete_selected()
        self.assertEqual(len(rows_from_text(read_document(self.path))), 1)
        self.editor.undo()
        self.assertEqual(len(rows_from_text(read_document(self.path))), 2)

    def test_search_names_and_cell_filter(self):
        self.editor.placement_search.set("Test creature")
        self.assertEqual(len(self.editor.objects.get_children()), 1)
        self.editor.placement_search.set("no matches")
        self.assertEqual(len(self.editor.objects.get_children()), 0)
        self.editor.placement_search.set("")
        self.editor.cell_only.set(True)
        self.editor.fields["Cell"].set("02990110")
        self.assertEqual(len(self.editor.objects.get_children()), 0)

    def test_cancel_reload_keeps_draft(self):
        self.select()
        self.editor.fields["X"].set("45")
        with patch("content_editor.messagebox.askyesno", return_value=False):
            self.editor.reload()
        self.assertEqual(self.editor.fields["X"].get(), "45")
        self.assertEqual(self.editor.selected_id, int(self.row[:16], 16))

    def test_world_load_without_selected_entity(self):
        self.editor.fields["Entity"].set("")
        self.editor.explore_cell = int(self.editor.fields["Cell"].get(), 16)
        REAL_REFRESH(self.editor)
        self.assertIsNotNone(self.editor.wanted)
        self.assertEqual(self.editor.wanted[0][2], 0)

    def test_startup_does_not_load_an_unrequested_dungeon(self):
        REAL_REFRESH(self.editor)
        self.assertIsNone(self.editor.wanted)

    def test_outdoor_arrival_keeps_eye_above_nearby_steep_ground(self):
        self.editor.frame_arrival([dict(terrain=True, mesh=dict(vertices=[(0, 0, 500), (30, 0, 600)]), position=(0, 0, 0))], (0, 0, 490))
        eye = self.editor.view3d.camera()[0]
        self.assertGreaterEqual(eye[2], 625)

    def test_world_map_is_initial_workspace_and_inspector_is_hidden(self):
        self.assertEqual(self.editor.workspace.select(), str(self.editor.atlas))
        self.assertFalse(self.editor.inspector_visible)

    def test_outdoor_cross_cell_keeps_reference_frame(self):
        editor = self.editor
        editor.view3d.set_tool("place")
        editor.outdoor_grid = {"block_side": 160, "cell_side": 20, "cells": 8}
        editor.active_scene_cell = 0x74ba0001
        editor.scene_ready = (editor.game.get(), editor.active_scene_cell, 0, (0, None, 0))
        editor.set_fields(dict(Cell="75ba0002", X="10", Y="15", Z="20"))
        with patch.object(editor.view3d, "update_placement") as moved:
            REAL_REFRESH(editor)
        self.assertEqual(editor.wanted[0][1], 0x74ba0001)
        self.assertEqual(editor.wanted[1], [170, 15, 20])
        moved.assert_called_once_with([170, 15, 20], 0, None)

    def test_scene_hit_maps_back_to_canonical_outdoor_cell(self):
        editor = self.editor
        editor.outdoor_grid = {"block_side": 160, "cell_side": 20, "cells": 8}
        editor.active_scene_cell = 0x74ba0001
        editor.pick_3d([170, 15, 20])
        self.assertEqual(editor.fields["Cell"].get(), "75ba0001")
        self.assertEqual(editor.fields["X"].get(), "10")
        self.assertEqual(editor.fields["Y"].get(), "15")

    def test_area_navigation_preserves_authored_placement(self):
        editor = self.editor
        self.select()
        before = editor.field_snapshot()
        future = Future()
        future.set_result([10, 20, 30])
        editor.navigation_future = future
        editor.navigation_loading = (editor.game.get(), 0x74ba0010, None)
        editor.navigation_wanted = editor.navigation_loading
        editor.finish_navigation()
        self.assertEqual(editor.field_snapshot(), before)
        self.assertEqual(editor.explore_cell, 0x74ba0010)
        self.assertEqual(editor.view3d.tool.get(), "explore")

    def test_camera_streaming_preserves_placement_and_translates_view(self):
        editor = self.editor
        before = editor.field_snapshot()
        editor.outdoor_grid = {"block_side": 160, "cell_side": 20, "cells": 8}
        editor.active_scene_cell = 0x74ba0001
        editor.view3d.focus = (165, 15, 30)
        with patch.object(editor.view3d, "get_navigation_point", return_value=(170, 15, 30)):
            editor.stream_camera_area()
        self.assertEqual(editor.field_snapshot(), before)
        self.assertEqual(editor.explore_cell, 0x75ba0001)
        self.assertEqual(editor.stream_camera[1], (5, 15, 30))
        REAL_REFRESH(editor)
        self.assertEqual(editor.wanted[0][1], 0x75ba0001)
        self.assertEqual(editor.wanted[0][2], 0)

    def test_keep_adding_allocates_distinct_objects(self):
        self.select()
        self.editor.new_placement()
        self.editor.keep_adding.set(True)
        self.editor.save()
        self.assertIsNone(self.editor.selected_id)
        self.editor.fields["X"].set("5")
        self.editor.save()
        rows = rows_from_text(read_document(self.path))
        self.assertEqual(len(rows), 3)
        self.assertEqual(len({row[0] for row in rows}), 3)

    def test_exploring_linked_room_streams_without_moving_object(self):
        editor = self.editor
        before = editor.field_snapshot()
        editor.active_scene_cell = 0x02990109
        editor.nearby_locations = [dict(cell=0x0299011b)]
        with patch("editor_locations.room_below", return_value=0x0299011b):
            editor.stream_camera_area()
        self.assertEqual(editor.field_snapshot(), before)
        self.assertEqual(editor.explore_cell, 0x0299011b)
        self.assertEqual(editor.stream_camera[0], 0x0299011b)

    def seed_group(self):
        second = self.row.split("\t")
        second[0], second[4] = "8140000000020001", "3"
        self.path.write_bytes((self.original + "\t".join(second) + "\n").encode("utf-8"))
        self.editor.reload(force=True)
        self.editor.select_many({0x8140000000020000, 0x8140000000020001})

    def test_multi_selection_disables_single_object_fields(self):
        self.seed_group()
        self.assertEqual(len(self.editor.group_ids), 2)
        self.assertEqual(self.editor.view3d.tool.get(), "select")
        self.assertEqual(str(self.editor.transform_entries["Entity"].cget("state")), "disabled")
        self.assertEqual(self.editor.save_button.cget("text"), "Arrange selected objects")

    def test_group_arrangement_commits_one_undoable_edit(self):
        self.seed_group()
        before = read_document(self.path)
        with patch.object(tk.Toplevel, "grab_set"):
            self.editor.open_arrange()
        dialog = self.editor.arrange_dialog
        self.assertIsNotNone(dialog)
        dialog.fields["Offset X"].set("5")
        dialog.preview()
        self.assertEqual(read_document(self.path), before)
        with patch("editor_arrange.Dat", MagicMock()):
            dialog.apply()
        self.assertIsNone(self.editor.arrange_dialog)
        rows = rows_from_text(read_document(self.path))
        self.assertEqual([r[4] for r in rows], ["6", "8"])
        self.assertEqual(len(self.editor.history), 1)
        self.editor.undo()
        self.assertEqual(read_document(self.path), before)

    def test_group_delete_undo_restores_entire_selection(self):
        self.seed_group()
        before = read_document(self.path)
        with patch("content_editor.messagebox.askyesno", return_value=True):
            self.editor.delete_selected()
        self.assertEqual(rows_from_text(read_document(self.path)), [])
        self.editor.undo()
        self.assertEqual(read_document(self.path), before)

    def test_direct_drag_commits_once_and_undo_restores_object(self):
        self.select()
        editor = self.editor
        editor.view3d.set_tool("move")
        self.assertTrue(editor.object_tools.transform("move", {}, "begin"))
        editor.object_tools.transform("move", dict(translation=(5, 0, 0)), "preview")
        self.assertEqual(read_document(self.path), self.original)
        with patch("editor_arrange.Dat", MagicMock()):
            editor.object_tools.transform("move", {}, "commit")
        self.assertEqual(rows_from_text(read_document(self.path))[0][4], "6")
        self.assertEqual(len(editor.history), 1)
        editor.undo()
        self.assertEqual(read_document(self.path), self.original)

    def test_cancel_drag_restores_fields_and_leaves_file_untouched(self):
        self.select()
        editor = self.editor
        before = editor.field_snapshot()
        editor.object_tools.transform("rotate", {}, "begin")
        editor.object_tools.transform("rotate", dict(degrees=90), "preview")
        editor.object_tools.transform("rotate", {}, "cancel")
        self.assertEqual(editor.field_snapshot(), before)
        self.assertEqual(read_document(self.path), self.original)
        self.assertEqual(editor.history, [])
