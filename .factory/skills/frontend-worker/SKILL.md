---
name: frontend-worker
description: Handles Vue 3 frontend changes for plant3d-web (task creation UI, preview functionality)
---

# Frontend Worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use this skill for features that modify the Vue 3 frontend (plant3d-web):
- Adding UI inputs to task creation wizard
- Modifying composables for state management
- Adding preview buttons and viewer integration
- Frontend-only changes that don't touch backend

## Work Procedure

1. **Read feature requirements** from features.json
2. Before using exact-text search, use `ace-tool` first for the initial codebase retrieval pass. Treat `grep`/`rg` only as secondary confirmation tools after `ace-tool`, unless the identifier is already known or the task explicitly requires exhaustive literal matching.
3. **Implement changes**:
   - Modify Vue components in src/components/task/
   - Update composables in src/composables/
   - Follow existing patterns (Vuetify components, Composition API)
   - Keep changes minimal and focused
4. **Run type check**:
   - `npm run type-check` - must pass with no errors
5. **Manual verification with agent-browser or browser**:
   - Start dev server if not running: `npm run dev`
   - Navigate to http://localhost:3101
   - Test all UI interactions described in feature
   - Verify network requests in DevTools
   - Take screenshots of key states
6. **Build verification**:
   - `npm run build-only` - must succeed
7. **Commit changes** with clear message
8. **Fill handoff** with all verification details including screenshots

## Example Handoff

```json
{
  "salientSummary": "Added noun filter chip input to task creation wizard Step 2. Users can add/remove noun tags (BRAN, HANG, etc.). Updated useTaskCreation.ts to include enabled_nouns in API payload. Ran npm run type-check (passed), manual testing shows chips work correctly, network payload verified.",
  "whatWasImplemented": "Modified src/components/task/TaskCreationWizard.vue to add v-combobox for noun input with chip display in Step 2 (lines 145-165). Updated src/composables/useTaskCreation.ts to add nouns: string[] to formData and buildRequest() to include enabled_nouns in config payload (lines 78, 210-215). Chips display uppercase nouns, duplicates prevented, whitespace trimmed.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      {
        "command": "npm run type-check",
        "exitCode": 0,
        "observation": "TypeScript compilation successful, no type errors"
      },
      {
        "command": "npm run build-only",
        "exitCode": 0,
        "observation": "Production build successful"
      }
    ],
    "interactiveChecks": [
      {
        "action": "Opened task creation wizard, typed 'BRAN' and pressed Enter",
        "observed": "BRAN chip appeared with X button, screenshot: noun-chip-added.png"
      },
      {
        "action": "Clicked X on BRAN chip",
        "observed": "Chip removed from list, screenshot: noun-chip-removed.png"
      },
      {
        "action": "Added BRAN, HANG, PANE chips and submitted form",
        "observed": "POST /api/tasks payload contains enabled_nouns: ['BRAN','HANG','PANE'], screenshot: network-payload.png"
      },
      {
        "action": "Tried adding 'bran' (lowercase)",
        "observed": "Normalized to 'BRAN' uppercase, screenshot: case-normalized.png"
      }
    ]
  },
  "tests": {
    "added": []
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- Feature depends on backend API changes that don't exist yet
- Type errors that indicate API contract mismatch
- UI patterns needed don't exist in current component library
- Preview functionality requires viewer changes beyond scope
