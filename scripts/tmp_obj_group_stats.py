import math
import sys


def clamp(x, a, b):
    return a if x < a else (b if x > b else x)


def dist_point_aabb(p, mn, mx):
    # Euclidean distance from point to AABB
    cx = clamp(p[0], mn[0], mx[0])
    cy = clamp(p[1], mn[1], mx[1])
    cz = clamp(p[2], mn[2], mx[2])
    dx = p[0] - cx
    dy = p[1] - cy
    dz = p[2] - cz
    return math.sqrt(dx * dx + dy * dy + dz * dz)


def dist2(a, b):
    dx = a[0] - b[0]
    dy = a[1] - b[1]
    dz = a[2] - b[2]
    return dx * dx + dy * dy + dz * dz


def farthest_pair(points):
    best = None
    best_d2 = -1.0
    n = len(points)
    for i in range(n):
        pi = points[i]
        for j in range(i + 1, n):
            d2 = dist2(pi, points[j])
            if d2 > best_d2:
                best_d2 = d2
                best = (pi, points[j])
    return best, best_d2


def bbox(points):
    mn = [float("inf")] * 3
    mx = [-float("inf")] * 3
    for x, y, z in points:
        if x < mn[0]:
            mn[0] = x
        if y < mn[1]:
            mn[1] = y
        if z < mn[2]:
            mn[2] = z
        if x > mx[0]:
            mx[0] = x
        if y > mx[1]:
            mx[1] = y
        if z > mx[2]:
            mx[2] = z
    return mn, mx


def main():
    if len(sys.argv) < 2:
        print("usage: tmp_obj_group_stats.py <path.obj>")
        return 2
    p = sys.argv[1]

    groups = {}
    cur = None
    with open(p, "r", encoding="utf-8", errors="ignore") as f:
        for line in f:
            if line.startswith("g "):
                cur = line.split(None, 1)[1].strip()
                groups.setdefault(cur, [])
                continue
            if cur is None:
                continue
            if line.startswith("v "):
                parts = line.split()
                if len(parts) >= 4:
                    groups[cur].append((float(parts[1]), float(parts[2]), float(parts[3])))

    print("obj:", p)
    print("groups:", len(groups))
    if not groups:
        return 0

    stats = {}
    for name, pts in groups.items():
        if not pts:
            continue
        mn, mx = bbox(pts)
        size = [mx[i] - mn[i] for i in range(3)]
        stats[name] = {"n": len(pts), "mn": mn, "mx": mx, "size": size}

    for name in sorted(stats.keys()):
        s = stats[name]
        print(
            "g %-20s verts=%5d bbox_size=(%.3f,%.3f,%.3f)"
            % (name, s["n"], s["size"][0], s["size"][1], s["size"][2])
        )

    # For TUBI groups, compute endpoints (farthest pair) and check proximity to component AABBs.
    tubi = {k: groups[k] for k in groups if k.startswith("TUBI_") and groups[k]}
    comps = {k: stats[k] for k in stats if not k.startswith("TUBI_")}
    if not tubi:
        print("no TUBI_* groups")
        return 0

    print("\n== TUBI endpoints vs component AABBs ==")
    for tname in sorted(tubi.keys()):
        pts = tubi[tname]
        (a, b), d2 = farthest_pair(pts)
        L = math.sqrt(d2) if d2 > 0 else 0.0
        best = None
        for cname, c in comps.items():
            da = dist_point_aabb(a, c["mn"], c["mx"])
            db = dist_point_aabb(b, c["mn"], c["mx"])
            m = min(da, db)
            if best is None or m < best[0]:
                best = (m, cname, da, db)
        if best:
            m, cname, da, db = best
            print(
                "%s len=%.3f  nearest_comp=%s  dist_endA=%.3f dist_endB=%.3f"
                % (tname, L, cname, da, db)
            )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

