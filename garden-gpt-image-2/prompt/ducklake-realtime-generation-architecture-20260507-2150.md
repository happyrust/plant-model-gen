Create a 16:9 technical system architecture diagram titled:
"DuckLake × plant-model-gen — Real-time On-demand Generation"

Subtitle:
"trait ModelWriter boundary · request-driven generation · snapshot-aware visibility · 2026-05-07"

Visual style:
Dark engineering diagram on deep slate #0F172A background, subtle 1px grid #1E293B at 32px spacing, JetBrains Mono / SF Mono typography, rounded rectangle nodes, semi-transparent fills, role-colored borders (2px), orthogonal arrows, no 3D, no logos, no emojis, no shadows. Include a compact legend in the bottom-right and a thin horizontal status bar at the very bottom labeled "phase: P1 writer · P2 api · P3 cdc · P4 compaction" in muted gray.

Layout:
Use FIVE left-to-right regions wrapped in dashed containers, plus ONE bottom feedback lane spanning the full width.

1. "Trigger / Request"
Nodes:
- HTTP: POST /api/lake/realtime-generate
- Body: { refnos:[...], dbnum:N, lod }
- (replaces) CLI --regen-model batch path (greyed out, dashed)

2. "Generation Pipeline (existing)"
Nodes:
- gen_all_geos_data(seed_roots)
- gen_index_tree_geos_optimized
- flume bounded channel: ShapeInstancesData batches
- tokio workers: CATA / LOOP / PRIM / Manifold boolean / mesh

3. "Trait Boundary (the seam)"
Central highlighted interface node, larger than others:
- trait ModelWriter
  methods: prepare(), write_batch(), finish(), writes_to_surreal()
Add a callout label above: "producer stable · persistence pluggable"
Add a small badge under it: "OCC transaction = 1 refno subtree"

4. "Writer Implementations"
Two parallel lanes:
- DuckLakeModelWriter (NEW, highlighted)
  responsibilities:
   • BEGIN / INSERT INTO lake.inst_geo / COMMIT
   • Inlined Data path (small batches, no parquet flush)
   • returns snapshot_id
- SurrealModelWriter (RETAINED, dimmer)
  responsibilities:
   • inst_info / inst_tubi / neg_relate
   • AABB / mesh / boolean side-effects
   • SQLite RTree refresh

5. "Storage & Visibility"
Group A — DuckLake lakehouse (violet, primary):
- Catalog metadata: metadata.ducklake (single-node) | Postgres (multi-node)
- Data: Parquet files + Inlined Data (in catalog tables)
- Snapshots: AT (VERSION => N) time travel
- Background tasks (small caption, dashed border):
   ducklake_flush_inlined_data · merge_adjacent_files · expire_snapshots
Group B — SurrealDB (violet, secondary):
- pe / pe_owner / hierarchy (graph traversal)
- inst_relate_aabb (SQLite RTree spatial)
- registry / users / sites
Group C — Filesystem mesh BLOB store (small):
- vertices/indices keyed by geo_hash (out-of-band, lake stores only path + hash)

Bottom feedback lane (full width, rose color):
- Background tokio task: poll max(snapshot_id)
- table_changes('inst_geo', from, to)  ← Change Data Feed
- SSE /api/lake/changes
- Frontend: subscribe + AT (VERSION=>N) read

Edges:
- Trigger -> Generation Pipeline (solid emerald)
- Generation Pipeline emits batches into flume (solid emerald)
- flume -> trait ModelWriter::write_batch (solid blue, thick)
- trait ModelWriter -> DuckLakeModelWriter (solid orange) and SurrealModelWriter (dashed orange, "retained")
- DuckLakeModelWriter -> Lake Catalog/Data (solid violet, with callout "Inlined Data: ms-level visibility")
- DuckLakeModelWriter -> snapshot_id returned upstream to HTTP response (small dashed cyan curve)
- SurrealModelWriter -> SurrealDB tables and RTree (solid violet, dimmer)
- DuckLakeModelWriter writes mesh path/hash, actual mesh BLOB written to filesystem (dashed violet)
- After successful Lake commit -> upsert SurrealDB.pe.snapshot_id (small dashed orange, label "compensating link, lake-first ordering")
- Lake snapshot stream -> bottom CDC lane (solid rose)
- CDC lane -> Frontend (solid rose, terminal arrow)

Color semantics:
- Trigger / request: cyan #22D3EE
- Generation pipeline: emerald #34D399
- Trait boundary (the seam): blue #60A5FA, brighter highlight
- Writer implementations: orange #FB923C
- Storage / data layer: violet #A78BFA
- Realtime / CDC / change feed: rose #FB7185
- Greyed-out / retained-but-secondary: muted slate #64748B

Constraints:
- Keep total visible nodes under 16 by grouping fine details inside nodes.
- Make the trait boundary visually central and the largest node.
- Emphasize the architectural intent: REQUEST-DRIVEN, COMMIT-VISIBLE, SNAPSHOT-AWARE — not pre-baked batch.
- The DuckLakeModelWriter lane is the highlighted "happy path"; SurrealModelWriter lane visibly dimmer to signal "retained for graph/spatial only".
- Show the bidirectional relationship: writes flow left-to-right, snapshot_id and CDC flow right-to-left.
- Text crisp and readable; no tiny illegible labels; legend lists all 6 colors with their semantic role.
- Do not claim SVG/editable output; produce a polished PNG-style technical diagram suitable for a design document.
- No emoji anywhere. No 3D. No skeuomorphism. No screenshot frames.
