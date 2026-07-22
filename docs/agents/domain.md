# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- **`CONTEXT.md`** at the repo root (Plant Model Versioning glossary).
- **`docs/adr/`** — read ADRs that touch the area you're about to work in (created lazily by `/domain-modeling`).

If any of these files don't exist, **proceed silently**. Don't flag their absence.

## Layout

Single-context repo: one root `CONTEXT.md` + `docs/adr/`.

## Use the glossary's vocabulary

When your output names a domain concept (issue title, refactor proposal, hypothesis, test name), use the term as defined in `CONTEXT.md` (e.g. **Version Commit**, **Version Anchor**, **Commit Fingerprint**, **Commit Pending**, **Legacy Anchor**). Don't drift to synonyms the glossary explicitly avoids.

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding.
