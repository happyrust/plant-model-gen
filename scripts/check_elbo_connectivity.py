import json
import math
import struct
import sys
from collections import Counter, defaultdict, deque
from pathlib import Path

import duckdb


def transform(m, p):
    x, y, z = p
    return (
        m[0] * x + m[4] * y + m[8] * z + m[12],
        m[1] * x + m[5] * y + m[9] * z + m[13],
        m[2] * x + m[6] * y + m[10] * z + m[14],
    )


def distance(a, b):
    return math.sqrt(sum((x - y) ** 2 for x, y in zip(a, b)))


def point_segment_distance(point, start, end):
    axis = tuple(end[i] - start[i] for i in range(3))
    length_squared = sum(value * value for value in axis)
    if length_squared == 0:
        return distance(point, start)
    t = max(0.0, min(1.0, sum((point[i] - start[i]) * axis[i] for i in range(3)) / length_squared))
    closest = tuple(start[i] + t * axis[i] for i in range(3))
    return distance(point, closest)


def read_accessor(doc, blob, index):
    accessor = doc["accessors"][index]
    view = doc["bufferViews"][accessor["bufferView"]]
    component = accessor["componentType"]
    fmt, size = {5123: ("H", 2), 5125: ("I", 4), 5126: ("f", 4)}[component]
    width = {"SCALAR": 1, "VEC3": 3}[accessor["type"]]
    offset = view.get("byteOffset", 0) + accessor.get("byteOffset", 0)
    stride = view.get("byteStride", width * size)
    return [
        struct.unpack_from("<" + fmt * width, blob, offset + i * stride)
        for i in range(accessor["count"])
    ]


def load_glb(path):
    data = path.read_bytes()
    _, _, _ = struct.unpack_from("<4sII", data)
    pos = 12
    chunks = {}
    while pos < len(data):
        length, kind = struct.unpack_from("<II", data, pos)
        chunks[kind] = data[pos + 8 : pos + 8 + length]
        pos += 8 + length
    doc = json.loads(chunks[0x4E4F534A].rstrip(b" \0"))
    blob = chunks[0x004E4942]
    vertices, triangles = [], []
    for mesh in doc["meshes"]:
        for primitive in mesh["primitives"]:
            base = len(vertices)
            vertices.extend(read_accessor(doc, blob, primitive["attributes"]["POSITION"]))
            indices = [v[0] + base for v in read_accessor(doc, blob, primitive["indices"])]
            triangles.extend(zip(indices[::3], indices[1::3], indices[2::3]))
    return vertices, triangles


def boundary_centers(vertices, triangles):
    welded, remap, by_position = [], [], {}
    for vertex in vertices:
        key = tuple(round(value, 6) for value in vertex)
        index = by_position.setdefault(key, len(welded))
        if index == len(welded):
            welded.append(vertex)
        remap.append(index)
    vertices = welded
    triangles = [tuple(remap[index] for index in tri) for tri in triangles]
    counts = Counter()
    for tri in triangles:
        for a, b in ((tri[0], tri[1]), (tri[1], tri[2]), (tri[2], tri[0])):
            counts[tuple(sorted((a, b)))] += 1
    graph = defaultdict(set)
    for (a, b), count in counts.items():
        if count == 1:
            graph[a].add(b)
            graph[b].add(a)
    centers = []
    while graph:
        start = next(iter(graph))
        todo, component = deque([start]), set()
        while todo:
            vertex = todo.popleft()
            if vertex in component:
                continue
            component.add(vertex)
            todo.extend(graph.get(vertex, ()))
        for vertex in component:
            graph.pop(vertex, None)
        centers.append(tuple(sum(vertices[i][axis] for i in component) / len(component) for axis in range(3)))
    return centers


def planar_cap_centers(vertices, triangles):
    planes = defaultdict(set)
    for tri in triangles:
        a, b, c = (vertices[i] for i in tri)
        ab = tuple(b[i] - a[i] for i in range(3))
        ac = tuple(c[i] - a[i] for i in range(3))
        normal = (
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        )
        length = math.sqrt(sum(value * value for value in normal))
        if length <= 1e-9:
            continue
        normal = tuple(value / length for value in normal)
        plane = (*normal, sum(normal[i] * a[i] for i in range(3)))
        first = next((value for value in plane[:3] if abs(value) > 1e-6), 1.0)
        if first < 0:
            plane = tuple(-value for value in plane)
        planes[tuple(round(value, 4) for value in plane)].update(tri)
    caps = sorted(planes.values(), key=len, reverse=True)[:2]
    centers = []
    for cap in caps:
        points = {tuple(round(value, 6) for value in vertices[i]) for i in cap}
        centers.append(tuple(sum(point[axis] for point in points) / len(points) for axis in range(3)))
    return centers


def main():
    refno = sys.argv[1] if len(sys.argv) > 1 else "24381_145019"
    root = Path("output/AvevaMarineSample/parquet/7997")
    generation = max((p for p in root.glob("generation-*") if p.is_dir()), key=lambda p: p.stat().st_mtime)
    con = duckdb.connect()
    inst = con.execute(
        "SELECT cata_hash, trans_hash FROM read_parquet(?) WHERE refno_str=?",
        [str(generation / "instances.parquet"), refno],
    ).fetchone()
    geo = con.execute(
        "SELECT geo_hash, geo_trans_hash FROM read_parquet(?) WHERE refno_str=? ORDER BY geo_index LIMIT 1",
        [str(generation / "geo_instances.parquet"), refno],
    ).fetchone()
    hashes = [inst[1], geo[1]]
    rows = con.execute("SELECT * FROM read_parquet(?) WHERE trans_hash IN (?, ?)", [str(generation / "transforms.parquet"), *hashes]).fetchall()
    matrices = {row[0]: row[1:] for row in rows}
    ports = con.execute(
        "SELECT point_number, pt_x, pt_y, pt_z FROM read_parquet(?) WHERE cata_hash=? ORDER BY point_number",
        [str(generation / "ptsets.parquet"), inst[0]],
    ).fetchall()
    vertices, triangles = load_glb(Path("assets/meshes/lod_L1") / f"{geo[0]}_L1.glb")
    centers = boundary_centers(vertices, triangles)
    if not centers:
        centers = planar_cap_centers(vertices, triangles)
    world_ports = [transform(matrices[inst[1]], row[1:]) for row in ports]
    world_centers = [transform(matrices[inst[1]], transform(matrices[geo[1]], p)) for p in centers]
    tubing_order = con.execute(
        'SELECT owner_refno_str, "order" FROM read_parquet(?) WHERE tubi_refno_str=?',
        [str(generation / "tubings.parquet"), refno],
    ).fetchone()
    tubing_rows = con.execute(
        '''SELECT x.* FROM read_parquet(?) t JOIN read_parquet(?) x USING (trans_hash)
           WHERE t.owner_refno_str=? AND t."order" BETWEEN ? AND ? ORDER BY t."order"''',
        [str(generation / "tubings.parquet"), str(generation / "transforms.parquet"),
         tubing_order[0], max(0, tubing_order[1] - 1), tubing_order[1]],
    ).fetchall()
    segments = [
        ((row[13], row[14], row[15]),
         (row[13] + row[9], row[14] + row[10], row[15] + row[11]))
        for row in tubing_rows
    ]
    gaps = [min(point_segment_distance(center, *segment) for segment in segments) for center in world_centers]
    endpoint_gaps = [
        min(distance(center, endpoint) for segment in segments for endpoint in segment)
        for center in world_centers
    ]
    print(
        f"refno={refno} ports={world_ports} mesh_ends={world_centers} "
        f"tubing_segments={segments} gaps_mm={gaps} endpoint_gaps_mm={endpoint_gaps}"
    )
    assert len(world_ports) == len(world_centers) == 2, "expected two catalogue ports and two mesh ends"
    assert len(segments) == 2, "expected incoming and outgoing tubing segments"
    assert max(gaps) <= 1.0, f"ELBO mesh is disconnected from tubing: max_gap_mm={max(gaps):.3f}"
    assert max(endpoint_gaps) <= 1.0, (
        f"tubing overruns or stops short of ELBO tangent: max_endpoint_gap_mm={max(endpoint_gaps):.3f}"
    )


if __name__ == "__main__":
    main()
