"""Screen/world math for direct editor manipulation, independent of Tk/GL."""
import math


def dot(a, b):
    return sum(x*y for x, y in zip(a, b))


def ray_bounds(eye, direction, bounds):
    near, far = 0., float("inf")
    for axis in range(3):
        low, high = bounds[0][axis], bounds[1][axis]
        if abs(direction[axis]) < 1e-12:
            if eye[axis] < low or eye[axis] > high:
                return False
            continue
        a, b = (low-eye[axis])/direction[axis], (high-eye[axis])/direction[axis]
        near, far = max(near, min(a,b)), min(far, max(a,b))
        if far < near:
            return False
    return far >= .15


def project(point, camera, width, height):
    eye, forward, right, up = camera
    offset = tuple(point[i]-eye[i] for i in range(3))
    depth = dot(offset, forward)
    if depth <= .15:
        return None
    focal = min(width, height)*.92
    return (width/2+focal*dot(offset, right)/depth, height/2-focal*dot(offset, up)/depth)


def plane_hit(x, y, z, camera, width, height):
    eye, forward, right, up = camera
    focal = min(width, height)*.92
    if focal <= 0:
        return None
    direction = tuple(forward[i]+right[i]*(x-width/2)/focal+up[i]*(height/2-y)/focal for i in range(3))
    if abs(direction[2]) < 1e-8:
        return None
    distance = (z-eye[2])/direction[2]
    if distance < 0 or distance > 1_000_000:
        return None
    return tuple(eye[i]+direction[i]*distance for i in range(3))


def descriptor(instances, tool, camera, width, height):
    if tool not in ("move", "rotate", "scale"):
        return None
    anchors = {}
    for item in instances:
        if item.get("selected") or item.get("role") == "preview":
            point = tuple(item["position"])
            anchors[item.get("object_id") or point] = point
    if not anchors:
        return None
    center = tuple(sum(point[i] for point in anchors.values())/len(anchors) for i in range(3))
    depth = dot(tuple(center[i]-camera[0][i] for i in range(3)), camera[1])
    length = max(.2, min(100, depth/max(1, min(width, height)*.92)*72))
    ends = [tuple(center[i]+(length if i == axis else 0) for i in range(3)) for axis in range(3)]
    return dict(center=center, ends=ends, length=length, tool=tool)


def hit_axis(x, y, gizmo, camera, width, height):
    if gizmo is None:
        return None
    start = project(gizmo["center"], camera, width, height)
    if start is None:
        return None
    best, result = 10, None
    for axis, endpoint in enumerate(gizmo["ends"]):
        end = project(endpoint, camera, width, height)
        if end is None:
            continue
        dx, dy = end[0]-start[0], end[1]-start[1]
        squared = dx*dx+dy*dy
        if squared < 100:
            continue
        t = max(.2, min(1, ((x-start[0])*dx+(y-start[1])*dy)/squared))
        distance = math.hypot(x-start[0]-dx*t, y-start[1]-dy*t)
        if distance < best:
            best, result = distance, axis
    return result


def drag_options(tool, start, current, gizmo, camera, width, height, axis=None):
    dx, dy = current[0]-start[0], current[1]-start[1]
    if tool == "rotate":
        return dict(degrees=dx*.65)
    if tool == "scale":
        return dict(scale=max(.01, min(100, math.exp(-dy*.012))))
    translation = [0., 0., 0.]
    if axis is not None:
        a, b = project(gizmo["center"], camera, width, height), project(gizmo["ends"][axis], camera, width, height)
        if a and b:
            vx, vy = b[0]-a[0], b[1]-a[1]
            squared = vx*vx+vy*vy
            if squared > 1:
                translation[axis] = (dx*vx+dy*vy)/squared*gizmo["length"]
    else:
        before = plane_hit(*start, gizmo["center"][2], camera, width, height)
        after = plane_hit(*current, gizmo["center"][2], camera, width, height)
        if before and after:
            translation = [after[i]-before[i] for i in range(3)]
    return dict(translation=tuple(translation))
