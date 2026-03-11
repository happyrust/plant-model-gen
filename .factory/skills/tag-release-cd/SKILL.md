# Tag Release CD

Use this skill when implementing or validating tag-driven build and deployment automation.

## Scope

- Backend repo `plant-model-gen` tag-triggered build and deploy
- Frontend repo `plant3d-web` tag-triggered build and deploy
- Version metadata propagation into artifacts, API, and UI
- Strict separation between automated code deployment and manual model-data upload

## Mission-Specific Rules

- `git tag vX.Y.Z` is the only release trigger.
- Backend and frontend deploy independently from their own repositories.
- Automatic deployment must never upload runtime model-generation data from `output/`.
- Backend must expose `/api/version`.
- Frontend Help/About must show frontend and backend version, build date, and commit.
- Use versioned release directories and `current` symlink switching on the server.

## Required Verification

- CI evidence for tag-triggered runs
- backend `/api/version` returns release metadata
- live UI Help/About shows version metadata
- production URL loads after release
- runtime model data remains intact
