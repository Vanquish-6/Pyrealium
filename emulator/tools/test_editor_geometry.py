"""Headless brush gesture tests, including native bytes through the project overlay."""
from concurrent.futures import Future
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from editor_geometry import GeometryTools
from preview_view import Preview3D
from world_project import WorldProject, OverlayDat
from world_terrain import TerrainEditor, decode_block
from test_world_terrain import Archive, block, definitions, region, types


class Variable:
    def __init__(self, value=None): self.value=value
    def get(self): return self.value
    def set(self, value): self.value=value


class DeferredExecutor:
    """A controllable worker: no sleeps, threads, GUI or native DAT writes."""
    def __init__(self): self.jobs=[]
    def submit(self, function, *args):
        future=Future()
        self.jobs.append((future,function,args))
        return future
    def complete(self):
        future,function,args=self.jobs.pop(0)
        try: future.set_result(function(*args))
        except Exception as error: future.set_exception(error)


class BrushIntegrationTests(unittest.TestCase):
    def setUp(self):
        tool=self.tool=GeometryTools.__new__(GeometryTools)
        tool.project=WorldProject(Path.cwd())
        tool.future=tool.kind=tool.terrain_engine=None
        tool.stroke_before=tool.stroke_after=tool.next_point=None
        tool.stroke_ended=False
        tool.stroke_poll=False
        tool.pick_height=False
        self.now=0.
        self.clock=patch("editor_geometry.time.monotonic",lambda:self.now)
        self.clock.start()
        self.addCleanup(self.clock.stop)
        for name,value in (("radius","15"),("strength","10"),("brush_mode","Raise"),
                           ("flatten_height","0"),("terrain_type","0: Dirt")):
            setattr(tool,name,Variable(value))
        self.executor=DeferredExecutor()
        view=SimpleNamespace(brush_preview=None,redraw=lambda:None,tool=Variable("terrain"))
        tool.editor=SimpleNamespace(status=Variable(),view3d=view,history=[],redo_history=["redo"],
            active_scene_cell=0x0a0a0001,game=Variable(str(Path.cwd())),executor=self.executor,
            root=SimpleNamespace(after=lambda *args:None),closing=False,
            outdoor_grid={"cell_side":10.,"cells":2,"block_side":20.})
        self.refreshes=0
        def changed(): self.refreshes+=1
        tool.changed=changed
        self.records={bx<<24|by<<16|65535:block(bx,by) for bx in range(9,13) for by in range(9,13)}
        base=Archive(self.records)
        base.path=Path("never-written.dat")
        self.overlay=OverlayDat(base,tool.project)
        portal=Archive({0x0f000000:region(),0x36000000:definitions(),0x12000000:types()})
        self.engine=TerrainEditor(self.overlay,portal)
        tool.terrain=lambda folder:self.engine

    def complete(self):
        self.now+=.1
        self.executor.complete()
        self.tool.finish_job()

    def test_hover_is_read_only_and_rejects_nonfinite_preview(self):
        self.assertTrue(self.tool.stroke([10,10,100],"hover"))
        self.assertEqual(self.tool.editor.view3d.brush_preview,{"center":[10,10,100],"radius":15})
        self.assertFalse(self.tool.project.patches)
        self.assertFalse(self.executor.jobs)
        for radius in ("nan","inf","-1","invalid"):
            self.tool.radius.set(radius)
            self.tool.stroke([10,10,100],"hover")
            self.assertIsNone(self.tool.editor.view3d.brush_preview)

    def test_drag_coalesces_pending_points_and_records_one_undo(self):
        self.assertTrue(self.tool.stroke([10,10,100],"begin"))
        self.tool.stroke([14,10,100],"preview")
        self.tool.stroke([16,10,100],"preview")
        self.tool.stroke(None,"commit")
        self.complete()
        self.assertEqual(len(self.executor.jobs),1)
        self.assertFalse(self.tool.editor.history)
        self.complete()
        history=self.tool.editor.history
        self.assertEqual(len(history),1)
        self.assertEqual(history[0][0],"geometry")
        self.assertFalse(self.tool.editor.redo_history)
        for did,before in history[0][1].items(): self.assertEqual(before,self.records[did])
        for did,after in history[0][2].items():
            self.assertEqual(after,self.overlay.read(did))
            self.assertNotEqual(after,self.records[did])
        self.tool.project.restore(history[0][1])
        self.assertFalse(self.tool.project.patches)
        self.tool.project.restore(history[0][2])
        self.assertEqual(self.tool.project.patches,history[0][2])

    def test_pending_cancel_discards_result_and_queued_sample(self):
        self.tool.stroke([10,10,100],"begin")
        self.complete()
        self.assertTrue(self.tool.project.patches)
        self.tool.stroke([14,10,100],"preview")
        self.tool.stroke([14,10,100],"preview")
        self.tool.stroke(None,"cancel")
        self.assertFalse(self.tool.stroke([16,10,100],"preview"))
        self.assertFalse(self.tool.stroke(None,"commit"))
        self.complete()
        self.assertFalse(self.tool.project.patches)
        self.assertFalse(self.tool.editor.history)
        self.assertEqual(self.tool.editor.redo_history,["redo"])
        self.assertFalse(self.executor.jobs)
        self.assertIsNone(self.tool.stroke_before)
        self.assertIsNone(self.tool.next_point)
        self.assertFalse(self.tool.project.dirty)
        self.assertTrue(self.tool.stroke([10,10,100],"begin"))

    def test_invalid_settings_reject_begin_without_worker_or_history(self):
        for name,value in (("radius","nan"),("radius","0"),("radius","41"),
                           ("strength","-1"),("strength","inf"),("strength","1001"),
                           ("brush_mode","Unknown")):
            with self.subTest(name=name,value=value):
                variable=getattr(self.tool,name)
                old=variable.get()
                variable.set(value)
                self.assertFalse(self.tool.stroke([10,10,100],"begin"))
                self.assertFalse(self.executor.jobs)
                self.assertIsNone(self.tool.stroke_before)
                variable.set(old)
        self.tool.brush_mode.set("Flatten")
        self.tool.flatten_height.set("nan")
        self.assertFalse(self.tool.stroke([10,10,100],"begin"))
        self.assertFalse(self.tool.project.patches)
        self.assertFalse(self.tool.editor.history)

    def test_noop_and_net_zero_preserve_redo_history(self):
        self.tool.strength.set("0.001")
        self.tool.stroke([10,10,100],"begin")
        self.tool.stroke(None,"commit")
        self.complete()
        self.assertFalse(self.tool.editor.history)
        self.assertEqual(self.tool.editor.redo_history,["redo"])
        self.tool.strength.set("10")
        self.tool.stroke([10,10,100],"begin")
        self.complete()
        self.assertFalse(self.tool.stroke([10,10,100],"begin"))
        self.tool.brush_mode.set("Lower")
        self.tool.stroke([10,10,100],"preview")
        self.tool.stroke(None,"commit")
        self.complete()
        self.assertFalse(self.tool.project.patches)
        self.assertFalse(self.tool.editor.history)
        self.assertEqual(self.tool.editor.redo_history,["redo"])

    def test_worker_error_rolls_back_and_drops_pending_samples(self):
        self.tool.stroke([10,10,100],"begin")
        self.complete()
        self.tool.stroke([14,10,100],"preview")
        self.tool.stroke([16,10,100],"preview")
        future,_,_=self.executor.jobs.pop(0)
        future.set_exception(ValueError("bad native payload"))
        with patch("editor_geometry.messagebox.showerror"):
            self.tool.finish_job()
        self.assertFalse(self.tool.project.patches)
        self.assertIsNone(self.tool.next_point)
        self.assertIsNone(self.tool.stroke_before)
        self.assertFalse(self.tool.stroke_ended)
        self.assertTrue(self.tool.stroke([10,10,100],"begin"))
        self.tool.stroke(None,"commit")
        self.complete()
        self.assertEqual(len(self.tool.editor.history),1)

    def test_paint_native_overlay_keeps_heights_and_other_packed_bits(self):
        self.tool.brush_mode.set("Paint")
        self.tool.terrain_type.set("1: GrassLand")
        self.tool.stroke([10,10,100],"begin")
        self.tool.stroke(None,"commit")
        self.complete()
        self.assertTrue(self.tool.project.patches)
        for did,after in self.tool.project.patches.items():
            before=decode_block(self.records[did],self.engine.definitions)
            after=decode_block(after,self.engine.definitions)
            self.assertEqual(before["heights"],after["heights"])
            for old,new in zip(before["infos"],after["infos"]):
                self.assertEqual(old&~127,new&~127)
        self.assertEqual(self.records,self.overlay.base.records)

    def test_pick_height_reads_ground_without_starting_a_stroke(self):
        self.tool.pick_height=True
        self.assertFalse(self.tool.stroke([10,10,123.5],"begin"))
        self.assertEqual(self.tool.flatten_height.get(),"123.5")
        self.assertEqual(self.tool.brush_mode.get(),"Flatten")
        self.assertFalse(self.tool.pick_height)
        self.assertFalse(self.executor.jobs)
        self.assertFalse(self.tool.project.patches)
        self.assertFalse(self.tool.stroke([10,10,140],"sample"))
        self.assertEqual(self.tool.flatten_height.get(),"140")

    def test_mode_controls_show_blend_and_only_relevant_fields(self):
        tool=self.tool
        tool.blend=Variable("50")
        class Widget:
            def __init__(self):self.options={}
            def configure(self,**options):self.options.update(options)
        for name in ("strength_label","strength_widget","flatten_widget","pick_height_button","palette_widget"):
            setattr(tool,name,Widget())
        tool.brush_mode.set("Flatten")
        tool.terrain_mode_changed()
        self.assertEqual(tool.strength_widget.options["textvariable"],tool.blend)
        self.assertEqual(tool.strength_widget.options["to"],100)
        self.assertEqual(tool.flatten_widget.options["state"],"normal")
        self.assertEqual(tool.palette_widget.options["state"],"disabled")
        tool.stroke([10,10,100],"begin")
        self.complete()
        value=decode_block(tool.project.patches[0x0a0affff],self.engine.definitions)
        self.assertEqual(value["heights"][4],50)
        tool.stroke(None,"commit")
        tool.brush_mode.set("Paint")
        tool.terrain_mode_changed()
        self.assertEqual(tool.strength_widget.options["state"],"disabled")
        self.assertEqual(tool.flatten_widget.options["state"],"disabled")
        self.assertEqual(tool.palette_widget.options["state"],"readonly")

    def test_high_frequency_motion_is_bounded_and_commit_keeps_endpoint(self):
        callbacks=[]
        self.tool.editor.root.after=lambda delay,callback:callbacks.append((delay,callback))
        self.tool.stroke([10,10,100],"begin")
        # Complete the native work immediately, before the next allowed sample.
        self.executor.complete()
        self.tool.finish_job()
        for index in range(100):self.tool.stroke([15+index*.01,10,100],"preview")
        self.assertFalse(self.executor.jobs)
        self.assertEqual(sum(callback==self.tool.flush_stroke for _,callback in callbacks),1)
        self.assertEqual(self.tool.next_point,[15.99,10,100])
        self.tool.stroke(None,"commit")
        self.assertFalse(self.tool.editor.history)
        self.now=.08
        self.tool.flush_stroke()
        self.assertEqual(len(self.executor.jobs),1)
        self.complete()
        self.assertEqual(len(self.tool.editor.history),1)
        self.assertIsNone(self.tool.next_point)

    def test_stationary_motion_does_not_repeatedly_sculpt(self):
        self.tool.stroke([10,10,100],"begin")
        self.complete()
        signature=self.tool.project.signature()
        for index in range(200):
            self.now+=.1
            self.tool.stroke([10.01,10.01,100],"preview")
        self.assertFalse(self.executor.jobs)
        self.assertEqual(signature,self.tool.project.signature())
        self.tool.stroke(None,"commit")
        self.assertEqual(len(self.tool.editor.history),1)

    def test_radius_presets_and_shortcuts_obey_native_world_limit(self):
        self.tool.set_radius(10000)
        self.assertEqual(float(self.tool.radius.get()),40)
        self.assertEqual(self.tool.resize_brush(.8),"break")
        self.assertEqual(float(self.tool.radius.get()),32)
        self.tool.set_radius(-10)
        self.assertEqual(float(self.tool.radius.get()),.5)
        self.tool.editor.view3d.tool.set("explore")
        self.assertIsNone(self.tool.resize_brush(2))
        self.assertEqual(float(self.tool.radius.get()),.5)


class PreviewTerrainHookTests(unittest.TestCase):
    def test_view_forwards_only_terrain_hit_gesture_phases(self):
        events=[]
        point=[10,10,100]
        calls=[]
        def hit(event,roles,terrain_only=False):
            calls.append((roles,terrain_only))
            return (0,None,None,point)
        view=SimpleNamespace(tool=Variable("terrain"),surface_hit=hit,
            on_surface=lambda point,phase:events.append((point,phase)) or True,
            terrain_dragging=False,transform_drag=None,drag=None,placement_drag=False,
            canvas=SimpleNamespace(focus_set=lambda:None))
        event=SimpleNamespace(state=0,x=1,y=2)
        Preview3D.hover_placement(view,event)
        Preview3D.orbit_start(view,event)
        self.assertTrue(view.terrain_dragging)
        Preview3D.orbit_drag(view,event)
        Preview3D.end_drag(view)
        self.assertFalse(view.terrain_dragging)
        self.assertEqual(events,[(point,"hover"),(point,"begin"),(point,"preview"),(None,"commit")])
        self.assertTrue(all(roles=={"world"} and terrain_only for roles,terrain_only in calls))


if __name__=="__main__": unittest.main()
