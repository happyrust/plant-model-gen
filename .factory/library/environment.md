# Environment

Environment notes for the **web console (Vue SPA) migration mission**.

**What belongs here:** platform constraints, validation readiness, local runtime quirks.
**What does NOT belong here:** exact service start/stop commands (use `.factory/services.yaml`).

---

## Machine Snapshot (planning dry run)

- OS: darwin 25.3.0
- CPU cores: 10
- Memory: 16 GiB

## Local runtime assumptions

- Rust `web_server` runs on **3100** (see `.factory/services.yaml`).
- `WEB_SERVER_PORT=3100` can be used to force the port.
- Frontend dev server (optional) runs on **3110**.

## Mission constraints

- Do **not** run tests or compile tests (`cargo test`, `cargo check --tests` are forbidden for this mission).
- Validate via: running `web_server` + `curl`/`post` + `agent-browser`.

