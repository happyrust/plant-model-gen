# SceneTree Parse Artifacts Plan

## Solution Approach

Unify SceneTree artifact paths around `versioned_db::db_meta_info`, then make every parse entry fail closed when required artifacts cannot be written or validated. The implementation should reuse the existing non-callback artifact contract instead of introducing a second validation path.

## Ordered Steps

1. Centralize the default TreeIndex path helper.

   Touch `src/versioned_db/db_meta_info.rs` and `src/fast_model/gen_model/tree_index_manager.rs`.

   - Add or expose a helper that resolves the current config's `project_name` from `DB_OPTION_FILE` using TOML parsing, mirroring the existing `output_root` behavior.
   - Replace `tree_index_manager.rs`'s private line-scanning helper with the shared helper.
   - Keep `DEFAULT_TREE_DIR` only as a compatibility fallback when `project_name` cannot be resolved.
   - Update comments and cache-miss guidance so `output/scene_tree` is no longer described as the normal target.

   Verification:
   - Use `rg "PathBuf::from\\(\"output\"\\).*scene_tree|output/scene_tree|DB_OPTION_FILE"` scoped to `src/fast_model/gen_model/tree_index_manager.rs` and `src/data_interface/db_meta_manager.rs`.
   - Confirm normal helper flow is `TreeIndexManager::with_default_dir` -> shared `db_meta_info` helper -> `<output_root>/<project>/scene_tree`.

2. Make `sync_total_async_threaded_with_callback` produce and validate artifacts like the non-callback path.

   Touch `src/versioned_db/database.rs`.

   - Introduce `let mut parsed_artifacts = Vec::new();` near the callback parse loop.
   - Move `export_tree_file(...)` out of the inner `type_ele_map` loop.
   - After all chunks for one processed DB file complete, compute `output_dir = db_meta_info::get_project_tree_dir(&project_name)`.
   - Call `export_tree_file`; if `tree_nodes` is non-empty and export fails, return `Err` with `dbnum`, file name, and output path context.
   - Update `db_meta_info.json` for each processed DB using the existing `DbFileMetaUpdate` shape; return `Err` on failure.
   - Push a `ParsedDbArtifact { project_name, dbnum, db_type, file_name, tree_node_count }` entry for every processed DB.
   - After the loop and writer drain, call `validate_parse_scene_tree_artifacts(&parsed_artifacts)`.
   - Preserve the current rule that `tree_nodes.is_empty()` is warning-only and does not require `.tree`.

   Verification:
   - Compare callback and non-callback paths for artifact update/export/validation parity.
   - Use precise `rg` to confirm no `warn!("[tree_export] ... 导出失败")` remains in the callback path.
   - Confirm final validation happens after all per-file artifacts have been written.

3. Make `parse_single_db_file` fail closed on metadata write failure.

   Touch `src/versioned_db/database.rs`.

   - Replace the current warning-only `update_db_meta_info_json` failure branch with `?` or `anyhow::Context`.
   - Keep existing tree export failure behavior as `Err`.
   - Consider calling `validate_parse_scene_tree_artifacts` for the single parsed DB after metadata update, using the same `ParsedDbArtifact` structure.

   Verification:
   - Confirm `parse_single_db_file` cannot reach the success print after metadata write failure.
   - Confirm empty tree-node files still follow the warning-only rule if validation is added.

4. Preserve managed-site `output_root` behavior.

   Touch only if necessary: `src/web_server/managed_project_sites.rs` and related config serialization paths.

   - Do not change the existing behavior that writes `output_root = runtime/admin_sites/<site_id>/output`.
   - Add a focused code-level check to ensure parse/generate configs and metadata still include the managed-site output root.

   Verification:
   - Use `rg "output_root|runtime/admin_sites|parse.*config|generate.*config"` in `src/web_server/managed_project_sites.rs`.
   - Confirm generated paths still point to the managed site runtime output directory.

5. Validate without `cargo test`.

   Follow repository rules: do not run `cargo test`.

   Code-level checks:
   - Run `sigmap ask "SceneTree parse artifacts callback output_root validation"`.
   - Use `rg` to ensure SceneTree paths no longer bypass the shared helper.
   - Use `cargo check` only if a compile check is needed and the user approves or the implementation phase needs it.

   CLI/file artifact checks:
   - Run `aios-database` with a `DbOption-parse.toml` that sets `output_root`.
   - Confirm `runtime/admin_sites/<site_id>/output/<project>/scene_tree/db_meta_info.json` exists.
   - For non-empty DBs, confirm `<dbnum>.tree` exists.
   - Simulate metadata or tree write failure and confirm the command returns failure.

   Web checks when a fixture is available:
   - Start `web_server` in debug mode.
   - POST the managed-site parse endpoint.
   - Confirm success transitions the site to Parsed.
   - Simulate write failure and confirm the site transitions to Failed with an artifact path in the error.

## Risks And Open Questions

- `sync_total_async_threaded_with_callback` currently uses `unwrap` and `expect` in several parse/write branches. This plan only closes SceneTree artifact failures; broader parse hardening is out of scope unless it blocks the artifact contract.
- If a parse chunk fails but later chunks succeed, current behavior continues. This plan preserves that unless the user separately decides chunk parse errors should fail the DB.
- CLI failure-injection may need a writable fixture with controlled permissions; on Windows this may require using a blocked file path or read-only directory rather than chmod-style permissions.
