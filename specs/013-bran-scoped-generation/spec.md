# Feature Specification: BRAN Scoped Generation

**Feature Branch**: `013-bran-scoped-generation`

**Created**: 2026-06-15

**Status**: Draft

**Input**: User description: "Add a fast test mode for model generation that generates only one specified BRAN refno, such as 2013286704/476, and then automatically tests it in the frontend."

## User Scenarios & Testing

### User Story 1 - Generate One BRAN For Fast Testing (Priority: P1)

As a developer validating BRAN-specific frontend behavior, I can run quick deploy with a target BRAN refno so the system generates only that BRAN subtree instead of generating the whole dbnum.

**Why this priority**: This is the core speed improvement. Without scoped generation, testing one BRAN still waits for broad model generation and makes iterative frontend validation slow.

**Independent Test**: Submit a quick-deploy test request for dbnum `250160` with target root refno `2013286704/476`; verify generation succeeds and the generated model output is limited to that BRAN subtree and required pipe segment data.

**Acceptance Scenarios**:

1. **Given** a valid BRAN refno `2013286704/476`, **When** quick deploy runs with that target root refno, **Then** generation processes only the target BRAN subtree and required generated artifacts for that scope.
2. **Given** scoped generation completes, **When** the generated data is inspected, **Then** unrelated dbnum-wide model instances are excluded from the scoped output.
3. **Given** the target BRAN has pipe segments, **When** scoped generation completes, **Then** the output includes the BRAN pipe segment data required by the frontend MBD pipe annotation and flow-direction validation.

---

### User Story 2 - Reject Invalid Scoped Targets Clearly (Priority: P1)

As a developer using fast BRAN testing, I need invalid targets to fail early with a clear reason so I do not accidentally run a full model generation or test the wrong object.

**Why this priority**: A mistyped or non-BRAN refno should not silently fall back to full generation because that would hide mistakes and erase the speed benefit.

**Independent Test**: Submit quick deploy with a missing refno and with a non-BRAN refno; verify each request fails before generation and reports the reason.

**Acceptance Scenarios**:

1. **Given** the target root refno cannot be parsed, **When** quick deploy is submitted, **Then** the request fails with a parse error and no scoped generation starts.
2. **Given** the target root refno does not exist in the selected project/dbnum, **When** quick deploy is submitted, **Then** the request fails with a not-found message and no generation starts.
3. **Given** the target root refno exists but is not a BRAN, **When** quick deploy is submitted, **Then** the request fails with a BRAN-only scope message and no generation starts.

---

### User Story 3 - Open The Scoped Result In The Frontend Automatically (Priority: P2)

As a developer validating frontend overlays, I can take the quick-deploy response and open a viewer URL that loads only the scoped BRAN and triggers MBD pipe annotation for the same refno.

**Why this priority**: The user's goal is not just to generate less data; it is to quickly validate the BRAN flow-direction overlay in the frontend.

**Independent Test**: After successful scoped quick deploy, open the returned viewer URL and verify the frontend loads the scoped BRAN model, opens MBD pipe annotation for the same refno, and the flow-direction controls can be toggled.

**Acceptance Scenarios**:

1. **Given** scoped generation succeeds, **When** the response is returned, **Then** it includes or logs a viewer URL containing `show_refno`, `mbd_refno`, and Parquet data-source parameters for the target BRAN.
2. **Given** the viewer URL is opened, **When** the frontend loads, **Then** it attempts to load the scoped BRAN subtree instead of the full dbnum.
3. **Given** MBD pipe annotation is available for the target BRAN, **When** the frontend automation runs, **Then** it can open the MBD panel and toggle the flow-direction control.

---

### User Story 4 - Preserve Full Generation Defaults (Priority: P2)

As an operator or developer running normal quick deploy, I need existing full-dbnum generation behavior to remain unchanged unless I explicitly provide a target root refno.

**Why this priority**: Scoped BRAN generation is a fast-test mode and must not change established deployment behavior.

**Independent Test**: Run quick deploy without `target_root_refno`; verify the request follows existing full generation behavior.

**Acceptance Scenarios**:

1. **Given** quick deploy is submitted without a scoped target, **When** generation starts, **Then** it uses the existing full generation path.
2. **Given** scoped generation is requested and later a normal quick deploy is requested, **When** the normal request runs, **Then** the previous scoped target does not persist or affect it.

### Edge Cases

- Target refno is supplied as `2013286704/476` or `2013286704_476`.
- Target refno parses but its dbnum does not match the quick-deploy dbnum.
- Target refno exists but has no generated geometry itself and only children carry geometry.
- Target BRAN has pipe segment records but no mesh for the BRAN root.
- Target BRAN expansion returns zero descendants.
- MBD pipe annotation data is unavailable even though scoped model data generated.
- Scoped quick-deploy is requested while another site task is already running.
- Scoped output directory already exists from a prior run.

## Requirements

### Functional Requirements

- **FR-001**: Quick-deploy test requests MUST accept an optional `target_root_refno` field.
- **FR-002**: `target_root_refno` MUST accept both slash and underscore refno notation.
- **FR-003**: When `target_root_refno` is absent, quick deploy MUST preserve existing full generation behavior.
- **FR-004**: When `target_root_refno` is present, the system MUST validate that the target exists in the selected project/dbnum before generation starts.
- **FR-005**: When `target_root_refno` is present, the system MUST validate that the target noun is `BRAN` before generation starts.
- **FR-006**: If the target refno is invalid, missing, dbnum-mismatched, or not BRAN, the system MUST fail with a clear message and MUST NOT fall back to full generation.
- **FR-007**: Scoped generation MUST expand the target BRAN to the refno set needed to generate that BRAN subtree and its pipe segment data.
- **FR-008**: Scoped generation MUST apply the target scope before or during generation, not only at frontend load time.
- **FR-009**: Scoped export MUST write Parquet/model artifacts containing the scoped generated data needed by the frontend viewer.
- **FR-010**: Scoped generation MUST include BRAN pipe segment data required for MBD pipe annotation and flow-direction validation.
- **FR-011**: A successful scoped quick-deploy response MUST include enough information to open the frontend on the scoped BRAN result.
- **FR-012**: The generated viewer URL MUST include the target refno as both the model load target and the MBD pipe annotation target.
- **FR-013**: The frontend automation MUST verify that the scoped viewer URL loads, the MBD panel can open, and the flow-direction control can be toggled.
- **FR-014**: Scoped generation state MUST be request-scoped and MUST NOT persist into later normal quick-deploy requests.
- **FR-015**: Logs or response metadata MUST identify scoped generation as scoped and include the target root refno and generated refno count.

### Key Entities

- **Scoped Quick-Deploy Request**: A quick-deploy request with optional `target_root_refno`, used only for fast testing.
- **Target Root Refno**: The BRAN refno selected as the root of the scoped generation.
- **Scoped Refno Set**: The target BRAN plus required descendants and pipe segment related records used for generation/export.
- **Scoped Generation Result**: The generated model/parquet data and response metadata for the target BRAN.
- **Scoped Viewer URL**: A frontend URL that loads the scoped BRAN and triggers MBD pipe annotation for the same refno.

## Success Criteria

### Measurable Outcomes

- **SC-001**: A valid scoped quick-deploy request for `2013286704/476` completes in less than 25% of the time required by the same dbnum's full model generation in the same environment.
- **SC-002**: A scoped result for `2013286704/476` contains fewer model instances than full dbnum generation while still containing all generated records required to view that BRAN subtree.
- **SC-003**: Invalid, missing, dbnum-mismatched, and non-BRAN targets fail before generation starts with a clear user-visible reason.
- **SC-004**: The scoped quick-deploy response includes a viewer URL that opens the target BRAN with model target and MBD target set to the same refno.
- **SC-005**: Frontend automation can open the scoped viewer URL and toggle MBD flow direction successfully.
- **SC-006**: A normal quick-deploy request without `target_root_refno` continues to run with existing full generation behavior.

## Assumptions

- v1 is a quick-deploy test mode only and does not change normal production deployment defaults.
- v1 accepts only BRAN targets; generic EQUI/ZONE scoped generation is out of scope.
- Parse may reuse the existing dbnum/related-library parsing behavior; the speed win is expected from scoped generation/export.
- The target sample for first validation is `2013286704/476` in AvevaPlantSample dbnum `250160`.
- The frontend validation can use existing URL parameters `show_refno`, `mbd_refno`, and `data_source=parquet`.
- Re-generating a new quick-deploy test site is allowed only when explicitly requested; this spec focuses on the capability, not on preserving old deployed test-site state.
