"""Bounded native DAT animation clips and bind-correct CPU skinning.

Evidence (7148): AnimMap 006796f0, DBAnimator 0067a5f0, AnimAsset
0069ab70/00699400, key records 007366b0/007366f0/006e5e80,
CharacterSetup 006105a0, inverse bind 006124f0, deformation 004ee0c0,
single-asset weight normalization/placement fallback004f0de0.
Animation graph leaves are listed separately. Native behavior selection,
movement blending, event hooks, attachment groups and root motion application
are not executed. No synthesized motion is substituted for unsupported clips.
"""
import bisect
import math
import struct


class AnimationFormatError(ValueError):
    pass


class Reader:
    def __init__(self, data):
        if len(data) > 64 * 1024 * 1024:
            raise AnimationFormatError("Animation record exceeds 64 MiB")
        self.data, self.at = data, 0

    def read(self, fmt):
        size = struct.calcsize("<" + fmt)
        if self.at + size > len(self.data):
            raise AnimationFormatError(f"Truncated animation field at {self.at:#x}")
        values = struct.unpack_from("<" + fmt, self.data, self.at)
        self.at += size
        return values[0] if len(values) == 1 else values

    def floats(self, count):
        values = self.read(f"{count}f")
        values = (values,) if count == 1 else values
        if any(not math.isfinite(value) or abs(value) > 1_000_000 for value in values):
            raise AnimationFormatError("Non-finite or excessive animation value")
        return values

    def count(self, maximum):
        value = self.read("I")
        if value > maximum:
            raise AnimationFormatError("Animation count exceeds preview limit")
        return value


def normalized(q):
    length = math.sqrt(sum(value * value for value in q))
    if not 0.99 <= length <= 1.01:
        raise AnimationFormatError("Non-normalized native animation quaternion")
    return tuple(value / length for value in q)


def identity():
    return (1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.)


def multiply(a, b):
    return tuple(sum(a[row*4+k] * b[k*4+column] for k in range(4))
                 for row in range(4) for column in range(4))


def inverse_affine(matrix):
    a, b, c, x, d, e, f, y, g, h, i, z = matrix[:12]
    determinant = a*(e*i-f*h)-b*(d*i-f*g)+c*(d*h-e*g)
    if abs(determinant) < 1e-12:
        raise AnimationFormatError("Singular bind transform")
    k = 1 / determinant
    v = ((e*i-f*h)*k, (c*h-b*i)*k, (b*f-c*e)*k,
         (f*g-d*i)*k, (a*i-c*g)*k, (c*d-a*f)*k,
         (d*h-e*g)*k, (b*g-a*h)*k, (a*e-b*d)*k)
    return (v[0], v[1], v[2], -v[0]*x-v[1]*y-v[2]*z,
            v[3], v[4], v[5], -v[3]*x-v[4]*y-v[5]*z,
            v[6], v[7], v[8], -v[6]*x-v[7]*y-v[8]*z,
            0., 0., 0., 1.)


def transform_matrix(offset, scale, quaternion):
    w, x, y, z = quaternion
    sx, sy, sz = scale
    return ((1-2*y*y-2*z*z)*sx, (2*x*y-2*z*w)*sy, (2*x*z+2*y*w)*sz, offset[0],
            (2*x*y+2*z*w)*sx, (1-2*x*x-2*z*z)*sy, (2*y*z-2*x*w)*sz, offset[1],
            (2*x*z-2*y*w)*sx, (2*y*z+2*x*w)*sy, (1-2*x*x-2*y*y)*sz, offset[2],
            0., 0., 0., 1.)


def decode_rig(data, did=None):
    reader = Reader(data)
    record = reader.read("I")
    if record >> 24 != 4 or did is not None and record != did:
        raise AnimationFormatError("Setup identity mismatch")
    flags = reader.read("I")
    if flags & ~0xc:
        raise AnimationFormatError("Unknown character setup flags")
    mesh_count = reader.count(256)
    meshes = [reader.read("I") for unused in range(mesh_count)]
    bones = reader.count(256)
    if bones == 0:
        raise AnimationFormatError("Setup has no animated skeleton")
    parents, types = [], []
    for index in range(bones):
        parent, kind = reader.read("iI")
        if parent < -1 or parent >= index:
            raise AnimationFormatError("Bone parent is cyclic or not topologically ordered")
        parents.append(parent)
        types.append(kind)
    frames = {}
    for unused in range(reader.count(256)):
        key, count = reader.read("II")
        if key in frames or count != bones:
            raise AnimationFormatError("Duplicate placement frame or skeleton size mismatch")
        transforms = []
        for index in range(count):
            values = reader.floats(10)
            if any(value <= 0 or value > 1000 for value in values[3:6]):
                raise AnimationFormatError("Invalid bind scale")
            transforms.append((tuple(values[:3]), tuple(values[3:6]), normalized(values[6:])))
        frames[key] = transforms
    if 1 not in frames:
        raise AnimationFormatError("Setup has no native rest placement frame 1")
    global_bind = []
    for parent, (offset, scale, quaternion) in zip(parents, frames[1]):
        local = transform_matrix(offset, scale, quaternion)
        global_bind.append(local if parent == -1 else multiply(global_bind[parent], local))
    return dict(did=record, meshes=meshes, parents=parents, bone_types=types,
                rest=frames[1], placement=frames.get(0,frames[1]),
                inverse_bind=[inverse_affine(value) for value in global_bind],
                setup_prefix_bytes=reader.at, opaque_tail_bytes=len(data)-reader.at)


def keys(reader, dimensions, quaternion=False):
    count = reader.count(65536)
    if count == 0:
        raise AnimationFormatError("Native clip has an empty key track")
    values, times = [], []
    for unused in range(count):
        item = reader.floats(dimensions + 1)
        value, time = item[:-1], item[-1]
        if time < 0 or times and time < times[-1]:
            raise AnimationFormatError("Unordered or negative animation key time")
        values.append(normalized(value) if quaternion else tuple(value))
        times.append(time)
    return dict(times=times, values=values)


def decode_clip(data, did=None, expected_bones=None):
    reader = Reader(data)
    record, kind = reader.read("II")
    if record >> 24 != 5 or did is not None and record != did or kind != 0x40000001:
        raise AnimationFormatError("Animator is not a direct native animation asset")
    period = reader.floats(1)[0]
    bones = reader.count(256)
    if not 0 < period <= 3600 or not bones or expected_bones is not None and bones != expected_bones:
        raise AnimationFormatError("Clip period or skeleton does not match")
    tracks, key_count = [], 0
    for bone in range(bones):
        scale, offset, quaternion, weight = keys(reader, 3), keys(reader, 3), keys(reader, 4, True), keys(reader, 1)
        for track in (scale, offset, quaternion, weight):
            key_count += len(track["times"])
            if track["times"][-1] > period + 0.001:
                raise AnimationFormatError("Native key lies after clip period")
        if key_count > 500000:
            raise AnimationFormatError("Clip key count exceeds preview budget")
        # CPartArray::DoAnimation004f0de0 divides accumulated transforms by
        # bone weight. For one asset, positive weights cancel exactly. A zero
        # influence falls back to the selected placement frame.
        if any(value[0] < 0 for value in weight["values"]):
            raise AnimationFormatError("Negative native animation influence")
        tracks.append(dict(scale=scale, offset=offset, quaternion=quaternion, weight=weight))
    has_info = reader.read("I")
    if has_info not in (0, 1):
        raise AnimationFormatError("Invalid native animation-info flag")
    return dict(id=record, period=period, tracks=tracks, key_count=key_count,
                has_info=bool(has_info), opaque_tail_bytes=len(data)-reader.at)


def graph_children(data, did):
    reader = Reader(data)
    record, kind = reader.read("II")
    if record != did or did >> 24 != 5:
        raise AnimationFormatError("Animator graph identity mismatch")
    if kind == 0x40000001:
        return None
    if kind == 0x40000003:  # AMHuman::StreamPack00694bd0: one referenced DBAnimator.
        children = [reader.read("I")]
    elif kind == 0x40000002:  # AnimCube00696ce0: slider/tick array then DID array.
        product = 1
        for unused in range(reader.count(8)):
            reader.read("I")
            count = reader.count(256)
            if count == 0:
                raise AnimationFormatError("Empty native animation cube axis")
            ticks = reader.floats(count)
            if any(a >= b for a,b in zip(ticks,ticks[1:])):
                raise AnimationFormatError("Unordered native animation cube axis")
            product *= count
            if product > 4096:
                raise AnimationFormatError("Native animation cube exceeds preview capacity")
        count = reader.count(4096)
        if count != product:
            raise AnimationFormatError("Animation cube tick product does not match child count")
        children = [reader.read("I") for unused in range(count)]
    elif kind == 0x40000008:  # AMSwitchME006934c0: default, type, key/DID hash.
        children = [reader.read("I")]
        reader.read("I")  # Native state selector is not evaluated in the editor.
        count = reader.read("I") & 0xffffff
        if count > 4096:
            raise AnimationFormatError("Animation state override capacity")
        seen = set()
        for unused in range(count):
            key, child = reader.read("II")
            if key in seen:
                raise AnimationFormatError("Duplicate animation state override")
            seen.add(key)
            children.append(child)
    else:
        raise AnimationFormatError(f"Unsupported animator graph type {kind:08x}")
    if reader.at != len(data) or any(child >> 24 != 5 for child in children):
        raise AnimationFormatError("Animator graph trailing data or invalid child")
    return children


def slerp(a, b, amount):
    product = sum(x*y for x,y in zip(a,b))
    if product < 0:
        b = tuple(-value for value in b)
        product = -product
    product = min(1, max(-1, product))
    if product > 0.9995:
        q = tuple(x+(y-x)*amount for x,y in zip(a,b))
    else:
        angle = math.acos(product)
        denominator = math.sin(angle)
        p, r = math.sin((1-amount)*angle)/denominator, math.sin(amount*angle)/denominator
        q = tuple(x*p+y*r for x,y in zip(a,b))
    length = math.sqrt(sum(value*value for value in q))
    return tuple(value/length for value in q)


def sample_keys(track, time, quaternion=False):
    index = max(0, bisect.bisect_right(track["times"], time)-1)
    value = track["values"][index]
    if index == len(track["times"])-1:
        return value
    next_time, current_time = track["times"][index+1], track["times"][index]
    if next_time <= current_time:
        return value
    amount = max(0, min(1, (time-current_time)/(next_time-current_time)))
    other = track["values"][index+1]
    return slerp(value, other, amount) if quaternion else tuple(x+(y-x)*amount for x,y in zip(value,other))


class AnimationSet:
    def __init__(self, rig, assets, tokens, warnings=()):
        self.rig, self.assets, self.warnings = rig, assets, list(warnings)
        self.clips = [dict(id=did, label=f"{did:08x} · {asset['period']:.2f}s", period=asset["period"],
                           tokens=tokens.get(did, [])) for did,asset in assets.items()]
        self._matrix_key, self._matrices, self._normal_matrices = None, None, None
        self._prepared = {}
        self.setup_did = rig["did"]
        self.map_did = None
        self.bone_count = len(rig["parents"])
        self.scope = "Native clip playback with bind-correct skinning; behavior-state selection and graph blending are not evaluated"

    def matrices(self, clip_id, seconds):
        if not math.isfinite(seconds):
            raise AnimationFormatError("Non-finite animation time")
        if clip_id not in self.assets:
            raise AnimationFormatError("Unknown native clip selection")
        clip = self.assets[clip_id]
        time = seconds % clip["period"]
        key = clip_id, time
        if self._matrix_key == key:
            return self._matrices, self._normal_matrices
        global_pose = []
        for index,(parent, track) in enumerate(zip(self.rig["parents"], clip["tracks"])):
            if sample_keys(track["weight"],time)[0] < 0.0002:
                offset,scale,quaternion = self.rig["placement"][index]
            else:
                offset = sample_keys(track["offset"], time)
                scale = sample_keys(track["scale"], time)
                quaternion = sample_keys(track["quaternion"], time, True)
            local = transform_matrix(offset, scale, quaternion)
            global_pose.append(local if parent == -1 else multiply(global_pose[parent],local))
        matrices = [multiply(pose, rest) for pose,rest in zip(global_pose,self.rig["inverse_bind"])]
        inverses = [inverse_affine(value) for value in matrices]
        normals = [tuple(matrix[column*4+row] for row in range(3) for column in range(3)) for matrix in inverses]
        self._matrix_key, self._matrices, self._normal_matrices = key, matrices, normals
        return matrices, normals

    def prepare_mesh(self, mesh):
        if mesh.get("did") not in self.rig["meshes"]:
            raise AnimationFormatError("Mesh is not part of this native character setup")
        cached = self._prepared.get(id(mesh))
        if cached is not None and cached[0] is mesh:
            return cached[1]
        skin = mesh.get("skin", ())
        vertices = mesh["vertices"]
        normals = mesh.get("normals", ())
        if len(skin) != len(vertices) or normals and len(normals) != len(vertices):
            raise AnimationFormatError("Mesh has no matching native skin influences")
        prepared = []
        for index, (point, influences) in enumerate(zip(vertices, skin)):
            weights, bones = influences["weights"], influences["bones"]
            if len(weights) != len(bones) or abs(sum(weights)-1) > 0.005 or any(value < -0.0001 or value > 1.0001 for value in weights):
                raise AnimationFormatError("Native skin influences do not sum to one")
            active = tuple((bone,weight) for bone,weight in zip(bones,weights) if weight > 0)
            for bone, weight in active:
                if bone >= self.bone_count or bone < 0:
                    raise AnimationFormatError("Mesh skin bone lies outside the setup skeleton")
            prepared.append((tuple(point), tuple(normals[index]) if normals else None, active))
        if len(self._prepared) >= 8:
            self._prepared.pop(next(iter(self._prepared)))
        # Keep the mesh reference with its prepared data, preventing object-ID
        # reuse and retaining immutable bind vertices while render buffers change.
        self._prepared[id(mesh)] = (mesh,prepared)
        return prepared

    def sample(self, clip_id, seconds, mesh):
        prepared = self.prepare_mesh(mesh)
        matrices, normal_matrices = self.matrices(clip_id, seconds)
        output, output_normals = [], []
        for point, source_normal, influences in prepared:
            x,y,z = point
            px=py=pz=nx=ny=nz=0.
            for bone,weight in influences:
                matrix = matrices[bone]
                px += weight*(matrix[0]*x+matrix[1]*y+matrix[2]*z+matrix[3])
                py += weight*(matrix[4]*x+matrix[5]*y+matrix[6]*z+matrix[7])
                pz += weight*(matrix[8]*x+matrix[9]*y+matrix[10]*z+matrix[11])
                if source_normal:
                    u,v,w = source_normal
                    matrix = normal_matrices[bone]
                    nx += weight*(matrix[0]*u+matrix[1]*v+matrix[2]*w)
                    ny += weight*(matrix[3]*u+matrix[4]*v+matrix[5]*w)
                    nz += weight*(matrix[6]*u+matrix[7]*v+matrix[8]*w)
            output.append([px,py,pz])
            if source_normal:
                length = math.sqrt(nx*nx+ny*ny+nz*nz)
                output_normals.append([nx/length,ny/length,nz/length] if length > 1e-12 else list(source_normal))
        return output, output_normals


def load_animation_set(portal, setup_did, map_did):
    rig = decode_rig(portal.read(setup_did), setup_did)
    reader = Reader(portal.read(map_did))
    if reader.read("I") != map_did or map_did >> 24 != 3:
        raise AnimationFormatError("Animation map identity mismatch")
    count = reader.read("I") & 0xffffff
    if count > 2048:
        raise AnimationFormatError("Animation map exceeds preview capacity")
    mapping = [reader.read("II") for unused in range(count)]
    if reader.at != len(reader.data) or len({token for token,did in mapping}) != count:
        raise AnimationFormatError("Trailing animation map data")
    assets, tokens, warnings, visited = {}, {}, [], {}
    total_keys = 0
    def visit(did, trail=()):
        nonlocal total_keys
        if did in trail or len(trail) >= 16 or len(visited) > 512:
            raise AnimationFormatError("Cyclic or excessive animator graph")
        if did in visited:
            return visited[did]
        data = portal.read(did)
        children = graph_children(data,did)
        leaves = []
        if children is None:
            if len(assets) >= 128:
                raise AnimationFormatError("Animation clip capacity")
            asset = decode_clip(data,did,len(rig["parents"]))
            if total_keys + asset["key_count"] > 500000:
                raise AnimationFormatError("Animation set exceeds the 500,000-key preview budget")
            total_keys += asset["key_count"]
            assets[did] = asset
            leaves = [did]
        else:
            for child in dict.fromkeys(children):
                leaves.extend(visit(child, trail+(did,)))
        visited[did] = list(dict.fromkeys(leaves))
        return visited[did]
    for token, did in sorted(mapping):
        try:
            for leaf in visit(did):
                tokens.setdefault(leaf,[]).append(token)
        except (KeyError, OSError, AnimationFormatError) as error:
            warnings.append(f"Animation token {token:08x}: {error}")
    if not assets:
        raise AnimationFormatError("No supported native animation assets match this skeleton")
    result = AnimationSet(rig,assets,tokens,list(dict.fromkeys(warnings)))
    result.map_did = map_did
    return result
