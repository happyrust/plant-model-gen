# Contract: Deployment Project Identity Over E3D Collection

## Purpose

Define externally verifiable behavior for the deployment-name-as-sole-outward-identity model, the independence invariant, the coincidence warning, and the regression guard.

## Outward Identity Contract

For a deployment with `project_name = D` and E3D collection `[E1, E2, ...]`, the following MUST all equal `D`:

- generated DbOption `project_name`
- runtime directory segment (`runtime/admin_sites/<slug(D)>/<site_id>` or the site's stored runtime_dir)
- output namespace segment (`output/<D>/scene_tree`, `output/<D>/parquet`, `output/<D>/scene_tree/cata_closure.json`)
- viewer access parameter (`output_project=D`)
- runtime database name returned to callers

For the worked example `D=9002`, `E=[AvevaPlantSample, AvevaCatalogue]`:

```toml
project_name = "9002"
included_projects = ["AvevaPlantSample", "AvevaCatalogue"]
```

and the active output is under `output/9002/...`.

## Source Identity Contract

The following MUST carry E3D source names/paths and MUST NOT be derived from `project_name`:

- `included_projects` (E3D names)
- `project_dirs` (E3D paths)
- source DB discovery roots / parse roots

## Independence Contract

- `project_name` and any E3D source name are independent. Setting `project_name = E1` MUST succeed.
- When `project_name` equals any E3D source name in the collection, the create/edit/clone/quick-deploy/preview response MUST include a non-blocking warning.
- No outward-identity surface MAY read an E3D source name under any configuration.

## Uniqueness Contract

- Two managed sites MUST NOT share the same normalized `project_name` (consistent with 014 FR-003).
- Uniqueness MUST be enforced consistently in create, edit, clone, and quick-deploy.

## Regression Guard Contract

`scripts/guard/deployment_identity_guard.ps1`:

- MUST pass on the current correct codebase.
- MUST fail when an outward-identity consumer (DB name, runtime dir, output dir, viewer output_project, parquet root) is changed to use the E3D source-name helper.
- MUST print the offending location on failure and use a non-zero exit code.

## Presentation Contract

- Details and preview surfaces MUST present `project_name` as the outward identity and list the E3D source projects as the collection.
- Create/edit/clone/quick-deploy MUST apply the same name normalization and uniqueness checks.
