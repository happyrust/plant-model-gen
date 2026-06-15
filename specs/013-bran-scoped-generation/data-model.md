# Data Model: BRAN Scoped Generation

## Scoped Quick-Deploy Request

Extends the existing quick-deploy request with an optional BRAN scope.

**Fields**:

- `project_path` / `projects` / `mbd_name` / `search_roots`: Existing quick-deploy project discovery inputs.
- `db_file` / `dbnum`: Existing target database selection inputs.
- `gen_model`, `gen_mesh`, `gen_spatial_tree`, `start_site`, `wait`: Existing quick-deploy execution controls.
- `target_root_refno`: Optional scoped target. Accepts slash or underscore notation, for example `2013286704/476` or `2013286704_476`.

**Validation rules**:

- If `target_root_refno` is absent, request follows existing full behavior.
- If present, it must parse into a refno.
- The target must exist in the selected project/dbnum.
- The target must belong to the selected dbnum.
- The target noun must be `BRAN`.

## Scoped Generation Target

Normalized and validated representation of the scoped BRAN target.

**Fields**:

- `root_refno`: Canonical slash-form target refno.
- `root_refno_key`: URL/file-safe underscore-form target refno.
- `dbnum`: Selected dbnum.
- `noun`: Must be `BRAN`.
- `project_name`: Project name used by the generated viewer URL.

**Relationships**:

- Created from one scoped quick-deploy request.
- Produces one scoped refno set.

## Scoped Refno Set

The effective set of refnos that should participate in scoped generation/export.

**Fields**:

- `root_refno`: The BRAN root.
- `refnos`: Target root plus required descendants.
- `expanded_count`: Number of refnos in the set.
- `source`: Expansion method, such as existing descendant query semantics.

**Validation rules**:

- Must contain the root refno.
- Must not be empty.
- Should preserve deterministic ordering for logs and artifact validation.

## Scoped Generation Result

Metadata returned or logged after scoped quick deploy.

**Fields**:

- `success`: Whether scoped generation completed.
- `site_id`: Managed site id.
- `dbnum`: Selected dbnum.
- `target_root_refno`: Canonical target refno.
- `scoped_refno_count`: Count of generated/expanded refnos.
- `generated`: Whether generation completed successfully.
- `entry_url`: Existing site/viewer entry URL when the site starts.
- `scoped_viewer_url`: URL that opens the scoped BRAN in plant3d-web.
- `warnings`: Non-fatal warnings, including MBD annotation unavailability.

**State transitions**:

- `Pending`: Request accepted, target not yet generated.
- `Validated`: Target exists and is BRAN.
- `Generating`: Scoped generation/export in progress.
- `Generated`: Scoped artifacts are available.
- `Failed`: Validation or generation failed; must include reason.

## Scoped Viewer URL

Frontend URL used for automated validation.

**Fields**:

- `output_project`: Generated project name.
- `show_refno`: Target BRAN refno in underscore form.
- `mbd_refno`: Same target BRAN refno in underscore form.
- `data_source`: `parquet`.

**Validation rules**:

- Must use the same target refno for model loading and MBD annotation.
- Must not use broad `show_dbnum` as the primary target when scoped refno is available.
