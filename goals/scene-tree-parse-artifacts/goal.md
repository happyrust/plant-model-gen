# SceneTree Parse Artifacts Goal

Fix SceneTree parse artifact handling so all parse entry points use the current `DB_OPTION_FILE` output root and fail closed when required parse artifacts cannot be written or validated.

Shared understanding lives in `facts.md`. The implementation order and verification contract live in `plan.md`.

Done means:

- SceneTree paths are centralized through `db_meta_info` for `<output_root>/<project>/scene_tree`.
- `sync_pdms_with_callback` and `parse_single_db_file` propagate SceneTree artifact failures instead of reporting success.
- Empty tree-node DBs remain warning-only for `.tree` absence.
- Managed-site `output_root` behavior remains intact.
- Verification follows the repository rule: no `cargo test` by default; use code checks, CLI/file artifacts, and Web POST checks when a fixture is available.
