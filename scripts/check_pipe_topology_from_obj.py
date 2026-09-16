import math
import sys


def dist2(a, b):
    dx = a[0] - b[0]
    dy = a[1] - b[1]
    dz = a[2] - b[2]
    return dx * dx + dy * dy + dz * dz


def norm(v):
    l = math.sqrt(v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
    if l <= 1e-12:
        return (0.0, 0.0, 0.0)
    return (v[0] / l, v[1] / l, v[2] / l)


def dot(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def sub(a, b):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def farthest_pair(points):
    # O(n^2) is fine for typical tubi groups (tens/hundreds of verts)
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


def cluster_points(endpoints, eps):
    # endpoints: list of (seg_name, end_idx, point)
    reps = []
    clusters = []  # list of list of (seg_name, end_idx)
    for seg, end_idx, p in endpoints:
        hit = -1
        for i, r in enumerate(reps):
            if dist2(r, p) <= eps * eps:
                hit = i
                break
        if hit >= 0:
            clusters[hit].append((seg, end_idx))
        else:
            reps.append(p)
            clusters.append([(seg, end_idx)])
    return reps, clusters


def main():
    if len(sys.argv) < 2:
        print("usage: check_pipe_topology_from_obj.py <path.obj> [eps_mm]")
        return 2
    p = sys.argv[1]
    eps = float(sys.argv[2]) if len(sys.argv) >= 3 else 0.5  # mm tolerance

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

    tubi = {k: v for (k, v) in groups.items() if k.startswith("TUBI_")}
    print("obj:", p)
    print("groups_total:", len(groups))
    print("tubi_groups:", len(tubi))
    if not tubi:
        print("no TUBI_* groups found")
        return 0

    segs = {}
    endpoints = []
    for name, pts in tubi.items():
        if len(pts) < 2:
            continue
        (a, b), d2 = farthest_pair(pts)
        d = norm(sub(b, a))
        segs[name] = {"a": a, "b": b, "dir": d, "len": math.sqrt(d2), "n": len(pts)}
        endpoints.append((name, 0, a))
        endpoints.append((name, 1, b))

    reps, clusters = cluster_points(endpoints, eps)
    degs = [len(c) for c in clusters]
    degs_sorted = sorted(degs, reverse=True)
    print("eps_mm:", eps)
    print("segments:", len(segs))
    print("unique_endpoints:", len(clusters))
    print("endpoint_degree_hist:", {d: degs.count(d) for d in sorted(set(degs))})

    # Identify junctions: degree == 2 (pipe chain bends/joins) and endpoints: degree == 1
    # Compute angles for degree==2 where they are between two segments.
    for ci, members in enumerate(clusters):
        if len(members) != 2:
            continue
        (s0, e0), (s1, e1) = members
        p = reps[ci]
        seg0 = segs[s0]
        seg1 = segs[s1]
        # Direction away from junction
        d0 = seg0["dir"] if e0 == 0 else (-seg0["dir"][0], -seg0["dir"][1], -seg0["dir"][2])
        d1 = seg1["dir"] if e1 == 0 else (-seg1["dir"][0], -seg1["dir"][1], -seg1["dir"][2])
        c = max(-1.0, min(1.0, dot(d0, d1)))
        ang = math.degrees(math.acos(c))
        print(
            "junction[%d] at (%.3f,%.3f,%.3f): %s[%d] <-> %s[%d] angle=%.1f deg"
            % (ci, p[0], p[1], p[2], s0, e0, s1, e1, ang)
        )

    # Print segment summary (lengths)
    for name in sorted(segs.keys()):
        s = segs[name]
        print(
            "seg %s: len=%.3f verts=%d a=(%.3f,%.3f,%.3f) b=(%.3f,%.3f,%.3f)"
            % (name, s["len"], s["n"], s["a"][0], s["a"][1], s["a"][2], s["b"][0], s["b"][1], s["b"][2])
        )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())

