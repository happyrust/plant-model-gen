# Environment

Environment variables, external dependencies, and setup notes.

**What belongs here:** Required env vars, external API keys/services, dependency quirks, platform-specific notes.
**What does NOT belong here:** Service ports/commands (use `.factory/services.yaml`).

---

## Database Connection

- **SurrealDB**: localhost:8020, user=root, pass=root
- **Backend**: Configured via db_options/DbOption.toml
- **Connection status**: Check logs for "✅ 数据库连接初始化成功"

## Test Data

- **Project**: AvevaMarineSample
- **dbnum**: 7997
- **Output directory**: /Volumes/DPC/work/plant-code/plant-model-gen/output/AvevaMarineSample/
- **Valid nouns**: BRAN, HANG, PANE, EQUI, PIPE, VALV, TUBI, etc.
