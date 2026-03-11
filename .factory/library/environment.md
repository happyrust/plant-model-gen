# Environment

Environment variables, external dependencies, and setup notes.

**What belongs here:** Required env vars, external API/service assumptions, local runtime notes.
**What does NOT belong here:** Service ports and start/stop commands (use `.factory/services.yaml`).

---

- Backend repository: `/Volumes/DPC/work/plant-code/plant-model-gen`
- Frontend repository: `/Volumes/DPC/work/plant-code/plant3d-web`
- Mission local runtime contract: backend on `3100`, frontend on `3101`
- Backend currently uses `db_options/DbOption-mac`; mission work must preserve local startup while enabling the 3100 port strategy.
- The machine already has a SurrealDB-related local environment. Mission work should reuse it instead of provisioning a second stack.
- External review platform sync may be mocked or degraded locally; local workflow correctness remains mandatory.
