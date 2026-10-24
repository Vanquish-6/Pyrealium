import copy
import json
import math
import unittest

from world_entrance import endpoint, make_pair, pair_rows, render_routes, validate_pair, FIRST_ID


class EntranceTests(unittest.TestCase):
    def setUp(self):
        self.outside = endpoint(0x74ba0007, [8, 12, 2], 64)
        self.inside = endpoint(0x02990109, [50, 220, -44], 128)
        self.landing = endpoint(0x0299010a, [10, 20, -40], 191)
        self.home = endpoint(0x74ba0008, [35, 50, 2], 0)
        self.pair = make_pair([], self.outside, self.inside, self.landing, self.home, min_level=5)

    def test_pair_has_distinct_objects_and_explicit_arrival_positions(self):
        self.assertEqual(self.pair["entrance"]["id"], FIRST_ID)
        self.assertEqual(self.pair["return"]["id"], FIRST_ID+1)
        self.assertEqual(self.pair["entrance"]["destination"],self.landing)
        self.assertEqual(self.pair["return"]["destination"],self.home)
        self.assertEqual(self.pair["return"]["min_level"],1)
        self.assertEqual(validate_pair(json.loads(json.dumps(self.pair))),self.pair)

    def test_ids_deterministically_skip_existing_objects(self):
        pair = make_pair([FIRST_ID, FIRST_ID+2], self.outside,self.inside,self.landing,self.home)
        self.assertEqual([pair[key]["id"] for key in ("entrance","return")],[FIRST_ID+1,FIRST_ID+3])

    def test_rows_match_visual_source_and_destination_facing(self):
        objects,routes = pair_rows(self.pair)
        self.assertEqual([len(row.split("\t")) for row in objects],[12,12])
        self.assertEqual([len(row.split("\t")) for row in routes],[12,12])
        obj,route = objects[0].split("\t"),routes[0].split("\t")
        self.assertEqual(obj[:3],[f"{FIRST_ID:016x}","47001454","1f000d09"])
        self.assertEqual(obj[3:7],route[2:6])
        self.assertEqual(route[6:],["0299010a","10","20","-40","191","5"])
        self.assertAlmostEqual(float(obj[7]), math.cos(64*math.pi/255))
        self.assertAlmostEqual(float(obj[10]),-math.sin(64*math.pi/255))
        self.assertEqual(len(pair_rows(self.pair,2)[0][0].split("\t")),11)

    def test_new_dungeon_cell_needs_no_installed_dat(self):
        point = endpoint(0x9abc0100,[0,0,0],255)
        make_pair([],self.outside,point,point,self.home)

    def test_reserved_cell_nonfinite_and_out_of_range_input_rejected(self):
        for cell in [0,0x74baffff,0x74bafffe,0x74ba0041,0x74ba00ff,0x74bafffc,0x100000000]:
            with self.assertRaises(ValueError): endpoint(cell,[0,0,0])
        for value in [float("nan"),float("inf"),999999.9999,True]:
            with self.assertRaises(ValueError): endpoint(0x02990100,[value,0,0])
        for heading in [-1,256,1.5,True]:
            with self.assertRaises(ValueError): endpoint(0x02990100,[0,0,0],heading)

    def test_metadata_tampering_and_duplicate_ids_rejected(self):
        pair=copy.deepcopy(self.pair)
        pair["return"]["id"] = pair["entrance"]["id"]
        with self.assertRaises(ValueError): validate_pair(pair)
        pair=copy.deepcopy(self.pair)
        pair["return"]["min_level"] = 2
        with self.assertRaises(ValueError): validate_pair(pair)
        with self.assertRaises(ValueError): render_routes([self.pair,self.pair])

    def test_export_is_deterministic_and_bounded(self):
        result=render_routes([self.pair])
        self.assertEqual(result,render_routes([self.pair]))
        self.assertTrue(result.startswith("pyrealium-portal-routes-v1\n"))
        self.assertEqual(len(result.splitlines()),3)
        with self.assertRaises(ValueError): render_routes([self.pair]*513)


if __name__ == "__main__":
    unittest.main()
