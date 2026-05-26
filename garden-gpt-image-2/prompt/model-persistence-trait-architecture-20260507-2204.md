Create a 16:9 technical system architecture diagram titled:
"Model Persistence Trait Refactor"

Subtitle:
"plant-model-gen · IndexTree generation writer boundary · 2026-05-07"

Visual style:
Dark engineering diagram on deep slate #0F172A background, subtle 1px grid #1E293B at 32px spacing, JetBrains Mono / SF Mono typography, rounded rectangle nodes, semi-transparent fills, role-colored borders, orthogonal arrows, no 3D, no logos, no emojis. Include a compact legend in the bottom-right.

Layout:
Use five left-to-right regions with dashed containers:

1. "Configuration / CLI"
Nodes:
- CLI flag: --model-writer
- DbOptionExt.model_writer_mode
- ModelWriterMode enum: Surreal | DrainOnly
- Feature validation: write-to-surrealdb / model-writer-drain

2. "Generation Orchestrator"
Nodes:
- gen_all_geos_data()
- process_index_tree_generation()
- gen_index_tree_geos_optimized()
- flume bounded channel: ShapeInstancesData batches

3. "Trait Boundary"
Central highlighted interface node:
- trait ModelWriter
  methods: prepare(), write_batch(), finish(), writes_to_surreal()
Add a callout: "producer is stable; persistence is pluggable"

4. "Writer Implementations"
Two parallel lanes:
- SurrealModelWriter
  responsibilities: init model tables, split by dbnum, save_instance_data_with_report, mesh/AABB/boolean pipeline, sqlite spatial refresh
- DrainOnlyWriter
  responsibilities: consume batches, collect DrainOnlyStats, skip DB writes, skip mesh/AABB/boolean, throughput benchmarking

5. "Persistence / Outputs"
Nodes:
- TreeIndexManager: refno -> dbnum
- SurrealDB model tables: inst_info, inst_tubi, inst_geo, neg_relate, tubi_relate
- AABB / inst_relate_aabb
- SQLite RTree spatial index
- Console summary / perf marks

Edges:
- CLI flag and TOML config feed DbOptionExt.model_writer_mode.
- ModelWriterMode selects a ModelWriter implementation.
- gen_index_tree_geos_optimized emits ShapeInstancesData batches into the flume channel.
- process_index_tree_generation sends batches through trait ModelWriter.write_batch().
- SurrealModelWriter performs solid arrows to TreeIndexManager, then SurrealDB tables, then AABB/boolean/sqlite index outputs.
- DrainOnlyWriter uses dashed arrows to DrainOnlyStats and perf summary only, with a red "no persistence side effects" guard label.
- cata_model direct outputs are gated by writes_to_surreal(); show a small guard near SurrealModelWriter lane.

Color semantics:
- Configuration / CLI: cyan #22D3EE
- Orchestrator / business flow: emerald #34D399
- Trait boundary: blue #60A5FA, brighter highlight
- Writer implementations: orange #FB923C
- Persistence / data: violet #A78BFA
- Safety guard / no-side-effects labels: rose #FB7185

Constraints:
- Keep total visible nodes under 15 by grouping details inside nodes.
- Make the trait boundary visually central and obvious.
- Emphasize the before/after architectural intent: removing direct mode checks from orchestration over time and moving persistence decisions behind ModelWriter.
- Text must be crisp and readable. Avoid tiny unreadable labels.
- Do not claim SVG/editable output; produce a polished PNG-style technical diagram suitable for a design document.
