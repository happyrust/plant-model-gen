# Quickstart: Site Project Identity & Parse Configuration

## Preconditions

- Run the admin web server normally for this repository.
- Use HTTP/admin UI validation rather than cargo test.
- Ensure no unrelated SurrealDB process is using the target managed site data folder.

## Scenario 1: New Site Uses Project-Scoped Identity

1. Open admin Sites page.
2. Create a site with project name `IDENTITY_A` and leave CATA partial parse at default.
3. Save without manually changing the CATA partial parse option.
4. Open site details.
5. Confirm:
   - Project name is `IDENTITY_A`.
   - Data/runtime path is scoped by the project identity.
   - CATA partial parse displays as enabled.
   - Dependency partial parse displays its saved state.

## Scenario 2: Create/Edit Parse Settings Are Consistent

1. Open create drawer.
2. Select a preset or DB types that include CATA.
3. Confirm CATA partial parse is visible and defaults enabled.
4. Disable CATA partial parse and save.
5. Reopen edit drawer for the same site.
6. Confirm disabled state is prefilled.
7. Re-enable and save.
8. Open site details and confirm the same values display.

## Scenario 3: Rename Stopped Deployed Site

1. Deploy or parse a site named `OLD_IDENTITY`.
2. Stop the site and wait for all tasks to finish.
3. Open edit/rename workflow and enter `NEW_IDENTITY`.
4. Review rename preview:
   - Old/new project names.
   - Affected database path.
   - Affected generated config files.
   - E3D database/project name change.
5. Apply rename.
6. Reload site details.
7. Confirm:
   - `site_id` is unchanged.
   - `project_name` is `NEW_IDENTITY`.
   - Runtime/data/config paths reflect `NEW_IDENTITY`.
   - Logs/metrics/history remain visible.
8. Start the site again and confirm it uses the renamed data/config paths.

## Scenario 4: Rename Blockers

1. Start a site or trigger parse/generate.
2. Attempt project rename.
3. Confirm preview/apply rejects the action before moving files and explains the blocking state.
4. Stop the site or wait for task completion.
5. Retry preview and confirm blockers are cleared.

## Validation Notes

- Do not run `cargo test` or compile test targets.
- If Rust files changed, run `cargo fmt` only.
- Prefer HTTP calls and browser/admin UI checks for web_server behavior.
- For config-level verification, inspect generated DbOption TOML/JSON output after API actions.
