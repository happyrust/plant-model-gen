# ptset Measurement Consistency Facts

- plant3d-web formal ptset display uses the current parquet model package as source of truth.
- plant3d-web measurement hover capture uses the same parquet ptset data and transform chain as formal ptset display.
- BRAN child ptset summaries and batch drawing are derived from instances.parquet owner_refno_str and ptsets.parquet, not realtime backend children or batch ptset APIs.
- The backend parquet export writes enough manifest counters to diagnose missing cata_hash separately from empty ptset rows.
- Missing ptset data reports the exact failing parquet link: missing instance row, missing ptsets table/file, missing cata_hash, missing transform row, or missing point rows.
- Verification avoids cargo tests; web_server checks use HTTP/POST against a running service, and aios-database checks use CLI plus JSON/parquet inspection.
