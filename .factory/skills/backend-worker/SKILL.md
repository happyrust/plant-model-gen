---
name: backend-worker
description: Handles Rust backend changes for plant-model-gen (parameter threading, struct modifications)
---

# Backend Worker

NOTE: Startup and cleanup are handled by `worker-base`. This skill defines the WORK PROCEDURE.

## When to Use This Skill

Use this skill for features that modify the Rust backend (plant-model-gen):
- Adding fields to DatabaseConfig or other API structs
- Threading parameters from API layer to generation pipeline
- Modifying handlers.rs for task execution logic
- Backend-only changes that don't touch frontend

## Work Procedure

1. **Read feature requirements** from features.json
2. **Write tests first** (if applicable):
   - Add unit tests for new struct fields (serialization/deserialization)
   - Add integration tests for parameter flow if needed
   - Run `cargo test` - tests should FAIL (red)
3. **Implement changes**:
   - Modify structs in src/web_server/models.rs
   - Thread parameters in src/web_server/handlers.rs
   - Follow existing patterns (Option<T> for new fields)
   - Keep changes minimal and focused
4. **Run tests** - should now PASS (green):
   - `cargo test --no-default-features --features ws,sqlite-index,web_server`
5. **Run validators**:
   - `cargo check --bin web_server --features web_server`
   - `cargo fmt --all`
   - `cargo clippy --features web_server -- -D warnings`
6. **Manual verification**:
   - Start web_server: `./target/debug/web_server`
   - Test API with curl: `curl -X POST http://localhost:3100/api/tasks -H "Content-Type: application/json" -d '{"name":"test","task_type":"DataGeneration","config":{...}}'`
   - Check logs for parameter flow
7. **Commit changes** with clear message
8. **Fill handoff** with all verification details

## Example Handoff

```json
{
  "salientSummary": "Added enabled_nouns, excluded_nouns, and debug_limit_per_noun_type fields to DatabaseConfig; threaded parameters from config to DbOptionExt in execute_real_task. Ran cargo test (all passing), cargo clippy (no warnings), and manual API test with curl showing parameters correctly flow to generation pipeline.",
  "whatWasImplemented": "Modified src/web_server/models.rs to add three optional fields to DatabaseConfig struct with serde annotations. Modified src/web_server/handlers.rs execute_real_task function (lines 4200-4250) to copy config.enabled_nouns to db_option_ext.index_tree_enabled_target_types, config.excluded_nouns to db_option_ext.index_tree_excluded_target_types, and config.debug_limit_per_noun_type to db_option_ext.index_tree_debug_limit_per_noun_type. Parameters now flow correctly to IndexTreeConfig and generation pipeline.",
  "whatWasLeftUndone": "",
  "verification": {
    "commandsRun": [
      {
        "command": "cargo test --no-default-features --features ws,sqlite-index,web_server",
        "exitCode": 0,
        "observation": "All 47 tests passed, including new serialization tests for DatabaseConfig"
      },
      {
        "command": "cargo check --bin web_server --features web_server",
        "exitCode": 0,
        "observation": "No compilation errors"
      },
      {
        "command": "cargo clippy --features web_server -- -D warnings",
        "exitCode": 0,
        "observation": "No clippy warnings"
      },
      {
        "command": "curl -X POST http://localhost:3100/api/tasks -d '{\"name\":\"test\",\"task_type\":\"DataGeneration\",\"config\":{\"enabled_nouns\":[\"BRAN\"],\"debug_limit_per_noun_type\":10,...}}'",
        "exitCode": 0,
        "observation": "Task created successfully (201), logs show 'index_tree_enabled_target_types: [\"BRAN\"]' and 'debug_limit_per_noun_type: Some(10)'"
      }
    ],
    "interactiveChecks": []
  },
  "tests": {
    "added": [
      {
        "file": "src/web_server/models.rs",
        "cases": [
          {
            "name": "test_database_config_with_noun_filters",
            "verifies": "DatabaseConfig serializes/deserializes with enabled_nouns and excluded_nouns fields"
          }
        ]
      }
    ]
  },
  "discoveredIssues": []
}
```

## When to Return to Orchestrator

- Feature depends on frontend changes that must be done first
- Compilation errors that indicate missing dependencies or incompatible changes
- Test failures that reveal design issues requiring orchestrator decision
- Parameter flow doesn't work as expected and requires architecture discussion
