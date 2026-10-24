"""Read-only native AC2 outdoor height fields and surface-table references.

Evidence: LandscapeDefs::UnPack 00629d60, CLandBlockData::StreamPack
0048f310, CachePackValues 0048f980, ConstructVertices 0048ca50,
ConstructPolygons 0048cca0, CTerrainDesc::UnPack 00522000,
TerrainType::unpack 006c6460, CSurfaceDesc::StreamPack 00522380,
LScapeSurfaceType::StreamPack 006c5910, MaterialGroup::StreamPack 006c45a0.
Geometry, holes and diagonal cuts follow native data. Native multi-corner
material blending, roads, water and procedural scene/frill placement are separate.
"""
import math
import struct
import colorsys


class TerrainFormatError(ValueError):
    pass


class Reader:
    def __init__(self, data):
        if not 4 <= len(data) <= 4 * 1024 * 1024:
            raise TerrainFormatError("Terrain record exceeds preview bounds")
        self.data, self.pos = data, 0

    def take(self, fmt):
        size = struct.calcsize("<" + fmt)
        if self.pos + size > len(self.data):
            raise TerrainFormatError(f"Truncated terrain data at {self.pos:#x}")
        result = struct.unpack_from("<" + fmt, self.data, self.pos)
        self.pos += size
        return result[0] if len(result) == 1 else result

    def u32(self):
        return self.take("I")

    def floating(self):
        result = self.take("f")
        if not math.isfinite(result):
            raise TerrainFormatError("Non-finite terrain value")
        return result

    def count(self, limit=1024):
        value = self.u32()
        if value > limit:
            raise TerrainFormatError("Terrain array count exceeds preview bounds")
        return value

    def align(self):
        self.pos = (self.pos + 3) & ~3
        if self.pos > len(self.data):
            raise TerrainFormatError("Missing terrain alignment bytes")

    def string(self):
        size = self.take("H")
        if size == 65535:
            size = self.u32()
        if size > 65536 or size > len(self.data) - self.pos:
            raise TerrainFormatError("Terrain label exceeds record bounds")
        self.pos += size
        self.align()

    def end(self):
        if self.pos != len(self.data):
            raise TerrainFormatError("Unsupported trailing terrain fields")


def landscape_defs(data):
    reader = Reader(data)
    did = reader.u32()
    if did >> 24 != 0x36:
        raise TerrainFormatError("Unsupported landscape definition identity")
    side, cells, pixels = reader.floating(), reader.u32(), reader.u32()
    maximum_height, sky, road_width = (reader.floating() for _ in range(3))
    terrain_bits, scene_bits, encounter_bits, color_bits, road_bits, replace_bits = (reader.u32() for _ in range(6))
    heights = [reader.floating() for _ in range(256)]
    reader.end()
    if not 0 < side <= 1000 or not 1 <= cells <= 32 or not 1 <= pixels <= 16:
        raise TerrainFormatError("Landscape dimensions outside preview bounds")
    if any(abs(v) > 1_000_000 for v in heights):
        raise TerrainFormatError("Invalid native height mapping")
    if terrain_bits + scene_bits + encounter_bits + color_bits + road_bits + replace_bits > 32:
        raise TerrainFormatError("Invalid landscape cell-info fields")
    return dict(did=did, cell_side=side, cells=cells, pixels=pixels,
                block_side=side*cells, vertex_side=cells*pixels+1,
                heights=heights, terrain_bits=terrain_bits,
                scene_bits=scene_bits, encounter_bits=encounter_bits,
                color_bits=color_bits, road_bits=road_bits, replace_bits=replace_bits)


def terrain_types(data):
    reader = Reader(data)
    did, version, has_default = reader.u32(), reader.u32(), reader.u32()
    if did >> 24 != 0x12 or version != 0 or has_default not in (0, 1):
        raise TerrainFormatError("Unsupported terrain-description header")

    def record():
        reader.string()
        surface, index, edge, default = reader.u32(), reader.u32(), reader.floating(), reader.u32()
        if default not in (0, 1):
            raise TerrainFormatError("Invalid default terrain scene flag")
        if default:
            reader.take("II")
        scenes = reader.count()
        for _ in range(scenes):
            reader.take("II")
        return dict(surface=surface, index=index, edge=edge)

    default = record() if has_default else None
    records = {}
    for _ in range(reader.count()):
        value = record()
        if value["index"] in records:
            raise TerrainFormatError("Duplicate terrain type")
        records[value["index"]] = value
    reader.align()
    reader.end()
    return records, default


def surface_types(data):
    reader = Reader(data)
    version, did = reader.u32(), reader.u32()
    if did >> 24 != 0x13 or version != 0:
        raise TerrainFormatError("Unsupported surface-description header")
    far_texture, far_scale, water_material, cloud_texture, cloud_scale, scroll_u, scroll_v = reader.take("IfIIfff")

    def material_array():
        result = []
        for _ in range(reader.count()):
            material, frequency, tiling = reader.u32(), reader.floating(), reader.floating()
            if frequency < 0 or not 0 < tiling <= 100000:
                raise TerrainFormatError("Invalid native material frequency or tiling")
            result.append(dict(material=material, frequency=frequency, tiling=tiling))
        frequency = reader.floating()
        return result, frequency

    def group():
        low, high, color_distance, color_interpolation = (reader.floating() for _ in range(4))
        edge = reader.u32()
        reader.align()
        physics = reader.u32()
        colors = []
        for _ in range(reader.count()):
            far, near = reader.u32(), reader.u32()
            # HSV variation and powers: ten serialized four-byte fields.
            variation = reader.take("10I")
            colors.append(dict(far=far, near=near, variation=variation))
        color_frequency = reader.floating()
        base, base_frequency = material_array()
        alpha = []
        for _ in range(4):
            # Alpha descriptors share the same three-word native stream size.
            values = [reader.take("IIf") for _ in range(reader.count())]
            alpha.append((values, reader.floating()))
        detail, detail_frequency = material_array()
        bump, bump_frequency = material_array()
        return dict(low=low, high=high, colors=colors, base=base,
                    base_frequency=base_frequency, detail=detail,
                    detail_frequency=detail_frequency, bump=bump,
                    physics=physics, alpha=alpha, color_frequency=color_frequency)

    def surfaces():
        result = {}
        for _ in range(reader.count()):
            index = reader.u32()
            if index in result:
                raise TerrainFormatError("Duplicate surface index")
            groups = [group() for _ in range(reader.count())]
            empty_road = reader.u32()
            result[index] = groups
        return result

    values, roads = surfaces(), surfaces()
    reader.end()
    return dict(surfaces=values, roads=roads, far_texture=far_texture,
                far_scale=far_scale, water_material=water_material)


def region_references(data):
    reader = Reader(data)
    did, number = reader.u32(), reader.u32()
    if did >> 24 != 0x0f:
        raise TerrainFormatError("Unsupported region descriptor")
    reader.string()
    map_id, map_scale, auto_map, auto_size, land_defs, length, width = reader.take("IfIIIII")
    result = dict(land_defs=land_defs, map_surface=map_id, map_scale=map_scale,
                  auto_map_surface=auto_map, auto_map_size=auto_size,
                  map_width=length, map_height=width)
    names = ("sound", "scene", "terrain", "surface", "encounters", "detailmap", "sky", "water", "fog", "properties", "worldmap")
    for name in names:
        count = reader.count(256)
        identifiers = [reader.u32() for _ in range(count)]
        regions = [reader.u32() for _ in range(count)]
        reader.align()
        result[name] = list(zip(identifiers, regions))
    reader.end()
    return result


def decode_terrain(data, definitions):
    reader = Reader(data)
    did, lbi, pathmap, count = reader.u32(), reader.u32(), reader.u32(), reader.u32()
    side = definitions["vertex_side"]
    if did & 65535 != 65535 or count != side*side or count > 16641:
        raise TerrainFormatError("Unsupported land-block height count or identity")
    height_values = reader.take(f"{count}B")
    # PSmartArray<byte> does not align its payload; the following u32 array
    # count is intentionally unaligned. Only the outer block stream aligns.
    if reader.count(16641) != count:
        raise TerrainFormatError("Terrain cell-info dimensions do not match heights")
    infos = list(reader.take(f"{count}I"))
    reader.align()
    reader.end()
    step = definitions["block_side"] / (side-1)
    vertices = [[x*step, y*step, definitions["heights"][height_values[x*side+y]]]
                for x in range(side) for y in range(side)]
    terrain_mask = (1 << definitions["terrain_bits"]) - 1
    types = [value & terrain_mask for value in infos]
    indices, corners = [], []
    blockx, blocky = (did >> 24) & 255, (did >> 16) & 255
    for x in range(side-1):
        gx = blockx*(side-1)+x
        for y in range(side-1):
            a, b, c, d = x*side+y, (x+1)*side+y, x*side+y+1, (x+1)*side+y+1
            if any(types[i] == 30 for i in (a,b,c,d)):
                continue
            gy = blocky*(side-1)+y
            random = ((gy*gx*gx*0x39557fcf-0x4a6506db)*gy-gx*0x6b3c5f11-0x51dd9315) & 0xffffffff
            triangles = ((a,b,d),(a,d,c)) if random >= 0x80000000 else ((a,b,c),(d,c,b))
            for triangle in triangles:
                indices.extend(triangle)
                corners.append((x,y))
    normals = [[0.0,0.0,0.0] for _ in vertices]
    for offset in range(0,len(indices),3):
        a,b,c = [vertices[i] for i in indices[offset:offset+3]]
        u,v = [b[i]-a[i] for i in range(3)],[c[i]-a[i] for i in range(3)]
        n = [u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]]
        for index in indices[offset:offset+3]:
            for axis in range(3):normals[index][axis] += n[axis]
    for normal in normals:
        length = math.sqrt(sum(v*v for v in normal))
        normal[:] = [v/length for v in normal] if length else [0,0,1]
    return dict(did=did,lbi=lbi,pathmap=pathmap,vertices=vertices,indices=indices,
                normals=normals,uvs=[[p[0]/20,p[1]/20] for p in vertices],
                colors=[],skin=[],materials=[],groups=[],terrain_types=types,
                terrain_info=infos,triangle_corners=corners,bounds=[
                    [0,0,min(p[2] for p in vertices)],
                    [definitions["block_side"],definitions["block_side"],max(p[2] for p in vertices)]])


def native_random(x, y, seed, total):
    """MaterialGroup::GetRandom 006c44d0's wrapping unsigned hash."""
    value = (seed*0x62433d7d-(x*y*y*seed*seed*0x2bd38901+0x6b3c5f11)*x-y*0x4a6506db) & 0xffffffff
    return value / 4294967296 * total


def choose_material(values, total, x, y, seed):
    if not values:
        return None
    target = native_random(x,y,seed,total)
    cumulative = 0.0
    for entry in values[:-1]:
        cumulative += entry["frequency"]
        if target < cumulative:
            return entry
    return values[-1]


def vertex_color(group, x, y):
    """Native deterministic color selection and HSV variation (006c4970/36d0)."""
    rows = group["colors"]
    if not rows:
        return 0xffffffff,0xffffffff
    as_float = lambda value: struct.unpack("<f",struct.pack("<I",value))[0]
    target = native_random(x,y,0x5dd39fd,group["color_frequency"])
    cumulative,selected = 0.0,rows[-1]
    for row in rows[:-1]:
        cumulative += as_float(row["variation"][-1])
        if target < cumulative:
            selected = row
            break
    var = selected["variation"]
    shifts = []
    for index,(coefficient,seed) in enumerate(((-0x450751a7,0x86f2a7),(-0x2ef80027,0x152127),(-0x6f4aaf73,0x2473))):
        random = ((x*x*y*y*coefficient+0x62433d7d)*seed-x*0x6b3c5f11-y*0x4a6506db)&0xffffffff
        value = random/4294967296
        low,high,power = as_float(var[index*3]),as_float(var[index*3+1]),var[index*3+2]
        if not all(math.isfinite(v) for v in (low,high)) or power > 64:
            raise TerrainFormatError("Unsupported terrain color variation")
        shifts.append((abs(2*value-1)**max(1,power))*(low if value<.5 else high))
    def shifted(packed,far=False):
        red,green,blue = ((packed>>shift)&255 for shift in (16,8,0))
        hue,saturation,value = colorsys.rgb_to_hsv(red/255,green/255,blue/255)
        if far:
            value += shifts[2]*.5
        else:
            hue += shifts[0]
            saturation += shifts[1]
            value += shifts[2]
        channels = colorsys.hsv_to_rgb(hue%1,max(0,min(1,saturation)),max(0,min(1,value)))
        rgb = [max(0,min(255,int(channel*255))) for channel in channels]
        return (packed&0xff000000)|(rgb[0]<<16)|(rgb[1]<<8)|rgb[2]
    return shifted(selected["near"]),shifted(selected["far"],True)


def apply_surfaces(mesh, definitions, terrain, default, surfaces):
    """Keep native base/detail choices and UV tiling, without inventing blends.

    Native terrain interpolates distinct SurfaceInfo values across all corners.
    A single triangle material uses its first corner for now; complete corner
    alpha/road/detail-shader composition remains explicit metadata for callers.
    """
    side = definitions["vertex_side"]
    blockx,blocky = (mesh["did"]>>24)&255,(mesh["did"]>>16)&255
    corner_values = []
    for index, kind in enumerate(mesh["terrain_types"]):
        info = terrain.get(kind,default)
        groups = surfaces["surfaces"].get(info["surface"],[]) if info else []
        normal = mesh["normals"][index]
        pitch = normal[0]*normal[0]+normal[1]*normal[1]
        group = next((value for value in groups if value["low"]**2 <= pitch <= value["high"]**2),None)
        x,y = blockx*(side-1)+index//side,blocky*(side-1)+index%side
        if group:
            base = choose_material(group["base"],group["base_frequency"],x,y,0x3c47f7)
            detail = choose_material(group["detail"],group["detail_frequency"],x,y,0x90083d)
            color,far = vertex_color(group,x,y)
            corner_values.append(dict(base=base,detail=detail,color=color,far_color=far,
                                      surface=info["surface"],terrain=kind))
        else:
            corner_values.append(dict(base=None,detail=None,color=0xffffffff,far_color=0xffffffff,
                                      surface=info["surface"] if info else 0,terrain=kind))
    old_vertices,old_normals,old_indices = mesh["vertices"],mesh["normals"],mesh["indices"]
    vertices,normals,uvs,detail_uvs,indices,colors,groups,materials,vertex_surfaces = [],[],[],[],[],[],[],[],[]
    material_lookup, material_metadata = {},{}
    for offset in range(0,len(old_indices),3):
        corners = old_indices[offset:offset+3]
        info = corner_values[corners[0]]
        base,detail = info["base"],info["detail"]
        key = (base["material"] if base else 0,base["tiling"] if base else 1,
               detail["material"] if detail else 0,detail["tiling"] if detail else 1)
        if key not in material_lookup:
            material_lookup[key] = len(materials)
            materials.append(key[0])
            material_metadata[material_lookup[key]] = dict(base=base,detail=detail)
        material = material_lookup[key]
        start = len(indices)
        for index in corners:
            indices.append(len(vertices))
            point = old_vertices[index]
            vertices.append(point)
            normals.append(old_normals[index])
            uvs.append([point[0]/key[1],point[1]/key[1]])
            detail_uvs.append([point[0]/key[3],point[1]/key[3]])
            colors.append(corner_values[index]["color"])
            vertex_surfaces.append(corner_values[index])
        if groups and groups[-1]["material"] == material:
            groups[-1]["count"] += 3
        else:
            groups.append(dict(start=start,count=3,material=material))
    mesh.update(vertices=vertices,normals=normals,uvs=uvs,indices=indices,
                colors=colors,groups=groups,materials=materials,
                detail_uvs=detail_uvs,vertex_surfaces=vertex_surfaces,
                terrain_materials=material_metadata,terrain_corners=corner_values,
                scope="Native height/cuts/holes and base/detail material references; corner blending, roads and water pending")
    return mesh


def block_placements(data):
    """CLandBlockInfo header and bounded EntityDesc static geometry prefix."""
    reader = Reader(data)
    flags,did = reader.u32(),reader.u32()
    if flags & ~1 or did & 65535 != 65534:
        raise TerrainFormatError("Unsupported outdoor block-info header")
    if flags & 1:
        reader.u32()
    cells,lighting,entity_group = reader.u32(),reader.u32(),reader.u32()
    rows,warnings = [],[]
    for index in range(reader.count(4096)):
        fields = reader.u32()
        if fields & ~0x1ff:
            warnings.append("Unsupported outdoor static entity flags; later props omitted")
            break
        identity = reader.u32() if fields&1 else 0
        kind = reader.u32() if fields&2 else 0
        runtime = reader.take("Q") if fields&4 else 0
        definition = reader.u32() if fields&8 else 0
        position,quaternion = [0,0,0],[1,0,0,0]
        if fields&0x10:
            position,quaternion,matrix = reader.take("3f"),reader.take("4f"),reader.take("9f")
        scale = reader.take("3f") if fields&0x20 else [1,1,1]
        if any(not math.isfinite(v) or abs(v)>1_000_000 for v in (*position,*quaternion,*scale)):
            raise TerrainFormatError("Invalid outdoor entity transform")
        if abs(sum(v*v for v in quaternion)-1)>.01 or any(v<=0 or v>1000 for v in scale):
            raise TerrainFormatError("Invalid outdoor entity orientation/scale")
        rows.append(dict(definition=definition,position=list(position),quaternion=list(quaternion),
                         scale=list(scale),identity=identity,runtime_id=runtime,type=kind))
        if fields&0x80:
            warnings.append("Outdoor entity property metadata omitted; later props unavailable")
            break
        if fields&0x40:
            reader.u32()
        if fields&0x100:
            warnings.append("Outdoor authoring labels omitted; later props unavailable")
            break
    return rows,warnings


def placement_from_point(cell, point, definitions):
    """Convert a ray-hit in the displayed block frame to canonical cell/XYZ."""
    if not 1 <= (cell & 65535) <= definitions["cells"]**2:
        raise TerrainFormatError("Ray-pick reference is not an outdoor cell")
    if len(point) != 3 or any(not math.isfinite(value) for value in point):
        raise TerrainFormatError("Invalid outdoor ray-pick position")
    side, count, block = definitions["cell_side"], definitions["cells"], definitions["block_side"]
    dx,dy = math.floor(point[0]/block),math.floor(point[1]/block)
    bx,by = (cell>>24)+dx,((cell>>16)&255)+dy
    if not 0 <= bx < 255 or not 0 <= by < 255:
        raise TerrainFormatError("Placement lies outside the supported world blocks")
    x,y = point[0]-dx*block,point[1]-dy*block
    low = 1 + min(count-1,int(x/side))*count + min(count-1,int(y/side))
    return (bx<<24)|(by<<16)|low, [x,y,point[2]]


def terrain_scene(cells, portal, cell, neighbors=1, region=0x0f000000):
    """Return instances, static placements and loaded outdoor cells in block space.

    Target block origin is (0,0,0). Adjacent blocks are offset by native block_side
    so player Placement coordinates and terrain ray picks share one frame.
    """
    if not 1 <= (cell&65535) <= 64 or not 0 <= neighbors <= 1:
        raise TerrainFormatError("Outdoor preview requires cell1..64 and at most one neighbor ring")
    refs = region_references(portal.read(region))
    definitions = landscape_defs(portal.read(refs["land_defs"]))
    if len(refs["terrain"]) != 1 or len(refs["surface"]) != 1:
        raise TerrainFormatError("Region-specific terrain/surface lookup needs native region-map support")
    terrain,default = terrain_types(portal.read(refs["terrain"][0][0]))
    surfaces = surface_types(portal.read(refs["surface"][0][0]))
    instances,placements,warnings,loaded = [],[],[],set()
    blockx,blocky = (cell>>24)&255,(cell>>16)&255
    offsets = [(x,y) for x in range(-neighbors,neighbors+1) for y in range(-neighbors,neighbors+1)]
    offsets.sort(key=lambda value:value[0]*value[0]+value[1]*value[1])
    for dx,dy in offsets:
        bx,by = blockx+dx,blocky+dy
        if not 0 <= bx < 255 or not 0 <= by < 255:
            continue
        block = bx<<24 | by<<16
        try:
            mesh = apply_surfaces(decode_terrain(cells.read(block|65535),definitions),definitions,
                                  terrain,default,surfaces)
            position = [dx*definitions["block_side"],dy*definitions["block_side"],0]
            instances.append(dict(mesh=mesh,position=position,quaternion=[1,0,0,0],scale=[1,1,1],role="world"))
            loaded.update(block|low for low in range(1,definitions["cells"]**2+1))
            if mesh["lbi"]:
                try:
                    rows,notes = block_placements(cells.read(mesh["lbi"]))
                    for row in rows:
                        row["position"] = [a+b for a,b in zip(row["position"],position)]
                    placements.extend(rows)
                    warnings.extend(notes)
                except (KeyError,ValueError,OSError) as error:
                    warnings.append(f"Outdoor block info {mesh['lbi']:08x}: {error}")
        except (KeyError,ValueError,OSError) as error:
            if dx==dy==0:
                raise
            warnings.append(f"Neighbor terrain {block|65535:08x}: {error}")
    warnings.append("Outdoor geometry uses native height-table values, holes and diagonal cuts. Native corner/road blending, water and procedural frills remain pending.")
    return dict(instances=instances,placements=placements,loaded_cells=loaded,
                warnings=list(dict.fromkeys(warnings)),definitions=definitions,
                surface_metadata=surfaces)
