# Architecture

Architectural decisions, patterns discovered, and key design notes.

**What belongs here:** Design patterns, data flow, component relationships, architectural constraints.

---

## Backend Architecture

**Parameter Flow:**
```
API Request (DatabaseConfig)
  → DbOption
  → DbOptionExt
  → IndexTreeConfig
  → Generation Pipeline
```

**Key Structs:**
- `DatabaseConfig` (src/web_server/models.rs): API-level config
- `DbOptionExt` (src/options.rs): Extended options with noun filtering
- `IndexTreeConfig` (src/fast_model/gen_model/config.rs): Generation config

**Noun Filtering:**
- `index_tree_enabled_target_types`: Whitelist of noun types
- `index_tree_excluded_target_types`: Blacklist of noun types
- `index_tree_debug_limit_per_noun_type`: Max instances per type

## Frontend Architecture

**Task Creation Flow:**
```
TaskCreationWizard.vue (3-step UI)
  → useTaskCreation.ts (state + validation)
  → buildRequest() (payload construction)
  → POST /api/tasks
```

**Preview Flow:**
```
TaskStatusCard.vue (@preview event)
  → TaskMonitorPanel.vue (handler)
  → ensurePanelAndActivate('viewer')
  → useModelGeneration.showModelByRefno()
  → ViewerPanel.vue (DtxViewer loads model)
```
