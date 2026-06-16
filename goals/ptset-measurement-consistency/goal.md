# ptset Measurement Consistency Goal

Repair and verify ptset measurement consistency across `plant3d-web` and `plant-model-gen-cata-closure` so formal ptset display, BRAN ptset summaries, and measurement capture all use the same current parquet model snapshot.

Shared understanding: `goals/ptset-measurement-consistency/facts.md`

Execution plan: `goals/ptset-measurement-consistency/plan.md`

Done condition: display and snap targets align for real model data, missing ptset data identifies the exact failing parquet link, and validation is recorded through CLI/JSON plus HTTP/browser checks without cargo tests.
