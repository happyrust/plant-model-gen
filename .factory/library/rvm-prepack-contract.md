# RVM Prepack Contract

Mission-specific contract notes for the **RVM delivery prepack integration**.

## Export Layers

- **Semantic layer:** normalized RVM + ATT records with stable identity, naming, owner linkage, transforms, and geometry references.
- **V3 layer:** loader-facing `instances_v3` JSON with grouped components, transform/aabb dictionaries, and per-geometry references.
- **Prepack layer:** `manifest.json`, `geometry_manifest.json`, `instances.json`, and emitted GLB geometry assets.
- **Serveability layer:** backend routes and static mounts that make exported artifacts retrievable over HTTP.

## Authoritative Surfaces

- CLI/export wiring: `src/main.rs`, `src/cli_args.rs`, `src/cli_modes.rs`
- Semantic/v3 export: `src/fast_model/export_model/export_dbnum_instances_v3.rs`, `src/fast_model/export_model/export_transform_config.rs`
- Prepack geometry assets: `src/fast_model/export_model/export_prepack_lod.rs`, `src/fast_model/export_model/export_instanced_bundle.rs`, `src/fast_model/export_model/export_glb.rs`
- Backend serving: `src/web_server/mod.rs`, `src/web_server/output_instances_files.rs`, `src/web_server/instance_export.rs`, `src/web_server/stream_generate.rs`, `src/bin/web_server.rs`

## Validation Rules

- Prefer scoped representative exports over full-library runs.
- Prove hash/reference integrity with artifact inspection.
- Use `web_server` POST/static validation only for backend readiness, not viewer rendering claims.
- Keep explicit note of any code/doc drift in stream/export surfaces.
