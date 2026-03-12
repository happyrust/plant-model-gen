# Repository Guidelines

## 部署目标服务器 
服务器是123.57.182.243      ssh 的账号密码是 root    Happytest123_

## Project Structure & Module Organization
This repository is a Rust workspace centered on model generation, spatial query, and web delivery. Core code lives in `src/`, with major areas such as `src/fast_model/` for generation/export pipelines, `src/web_server/` and `src/web_api/` for HTTP/UI endpoints, `src/grpc_service/` for gRPC, and `src/data_interface/` for database access. Integration tests live in `tests/`, while subsystem and regression tests also exist under `src/test/`. Use `examples/` for diagnostics and one-off verification tools. Supporting assets and runtime data are kept in `assets/`, `resource/`, `proto/`, `db_options/`, `test_data/`, and `docs/`.

## Build, Test, and Development Commands
- `cargo check --bin web_server --features web_server`: fast validation for the main web binary and feature wiring.
- `cargo build --release --bin web_server --no-default-features --features ws,sqlite-index,surreal-save,web_server`: CI-aligned release build for deployment.
- `cargo test --no-default-features --features ws,sqlite-index,web_server`: default local test entry matching CI coverage.
- `cargo fmt --all`: apply standard Rust formatting.
- `cargo clippy --features web_server -- -D warnings`: enforce lint cleanliness before review.
- `cargo run --bin aios-database -- --export-obj --dbnum 7997`: example local workflow for export/debug tasks.

## Coding Style & Naming Conventions
Follow `rustfmt` defaults and keep code simple over defensive abstraction. Use `snake_case` for files, modules, and functions; `CamelCase` for structs/enums/traits; and behavior-oriented names for tests such as `regression_room_batch_compute.rs`. Keep new modules close to the owning domain instead of creating broad utility buckets. Prefer focused feature flags and avoid expanding the default feature set unless required.

## Testing Guidelines
Add integration coverage in `tests/` for cross-module behavior and place domain-specific regression tests near existing suites in `src/test/`. Name tests after the behavior or bug being protected. Run at minimum `cargo test --no-default-features --features ws,sqlite-index,web_server` before opening a PR; when touching formatting or API surfaces, also run `cargo fmt` and `cargo clippy`.

## Commit & Pull Request Guidelines
Recent history follows Conventional Commits, for example `fix(ci): ...`, `feat: ...`, and `refactor: ...`. Keep commits scoped and descriptive. PRs should include a short problem statement, the chosen approach, impacted features/flags, and the exact verification commands run. Include screenshots only for `src/web_server/` UI/template changes and link related issues when applicable.

## Configuration Notes
This project depends on private Git repositories during CI and some local builds. Ensure Git credentials are configured before dependency resolution. Keep secrets out of tracked config files; store environment-specific settings in local, uncommitted overrides under the existing config workflow.
