# Facts

- SceneTree write paths and normal read paths resolve through `db_meta_info::get_project_tree_dir(project_name)`, which uses `output_root` from the current `DB_OPTION_FILE` and returns `<output_root>/<project>/scene_tree`.
- The legacy `output/scene_tree` directory remains only as a fallback when `TreeIndexManager` cannot resolve `project_name` from the current `DB_OPTION_FILE`; new parse writes do not target that fallback.
- `sync_pdms_with_callback` propagates SceneTree artifact failures: `db_meta_info.json` update errors, `.tree` export errors for non-empty DBs, and final artifact validation failures return `Err` instead of warning-only success.
- The callback parse path exports each processed DB tree once after that DB file has finished parsing, rather than exporting repeatedly from inside the attribute type/chunk loop.
- When a processed DB produces zero tree nodes, artifact validation only logs a warning and does not require a `<dbnum>.tree` file.
- `parse_single_db_file` returns `Err` when `update_db_meta_info_json` fails, so CLI single-file parsing cannot report success without `db_meta_info.json` being written.
- Managed-site parse and generate configs continue writing `output_root = runtime/admin_sites/<site_id>/output` so their SceneTree artifacts land under the managed site runtime directory.
- Verification avoids `cargo test` by default; it uses code-level checks plus `aios-database` CLI/file artifact checks, and runs `web_server` POST validation when a managed-site fixture or config is available.
