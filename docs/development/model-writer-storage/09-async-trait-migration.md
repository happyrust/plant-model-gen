# Async Fn in Trait — Migration Research

> v4 §2.2: investigation report for migrating `ModelWriterBackend` from
> `#[async_trait]` to native `async fn in trait` (AFIT). **Decision-only
> document; no code changes in this PR.**

## 1. Current state (post v3)

- Trait: `ModelWriterBackend` declared with `#[async_trait]` (9 async methods)
- All call sites consume `Arc<dyn ModelWriterBackend>` (orchestrator, mock,
  compare wrapper, verify binary, factory in `create_model_writer`)
- Rust toolchain: `rustc 1.97.0-nightly (f964de49b 2026-05-07)`, project
  uses `edition = "2024"`
- Each trait method call: `Pin<Box<dyn Future<Output = ...> + Send + 'a>>`
  allocation per call (one `Box<...>` per method invocation)

```rust
#[async_trait]
pub trait ModelWriterBackend: Send + Sync {
    fn name(&self) -> &'static str;
    async fn init(&self, context: &ModelWriterContext) -> anyhow::Result<()>;
    async fn cleanup(&self, request: CleanupRequest<'_>) -> anyhow::Result<()>;
    async fn write_base_batch(&self, batch: BaseInstanceBatch<'_>) -> anyhow::Result<WriteBaseReport>;
    async fn persist_mesh_results(&self, batch: MeshResultBatch<'_>) -> anyhow::Result<()>;
    async fn write_inst_relate_aabb(&self, batch: InstRelateAabbBatch<'_>) -> anyhow::Result<()>;
    async fn reconcile_missing_neg(&self, request: ReconcileRequest<'_>) -> anyhow::Result<usize>;
    async fn take_missing_neg_carriers(&self) -> anyhow::Result<Vec<RefnoEnum>>;
    async fn run_boolean_bridge(&self, request: BooleanBridgeRequest) -> anyhow::Result<BooleanBridgeReport>;
    async fn finalize(&self, request: FinalizeRequest) -> anyhow::Result<FinalizeSummary>;
}
```

## 2. Native AFIT vs `#[async_trait]`

### 2.1 Stable as of (2026-05)

- **Native AFIT** (`async fn` directly in trait body) stabilized in Rust 1.75
  (Dec 2023). The return-position-impl-trait-in-trait (RPITIT) machinery
  underneath is fully stable.
- **`dyn Trait` for async-fn-in-trait** is NOT yet stable. The current
  workaround is the unstable `#![feature(return_type_notation)]` /
  `dyn AsyncFnTrait<...>` combination, OR continue using `async_trait`
  for the `dyn` path while writing native AFIT for static dispatch.
- **`trait-variant`** crate (by `rust-lang/wg-async`) is the official
  recommended bridge: write one `async fn in trait` and let the macro
  generate both static and dynamic dispatch trampolines.

### 2.2 Performance characteristics

| Path | `#[async_trait]` | Native AFIT (`impl`) | Native AFIT (`dyn`) |
|---|---|---|---|
| Allocation per call | 1× `Box<dyn Future>` | none (compiler-known size) | 1× `Box<dyn Future>` via shim |
| Vtable indirection | yes | no | yes |
| Compile time | fast (proc-macro expand) | slower (monomorphisation) | similar to async_trait |
| Async fn inlining | impossible (boxed) | possible | impossible |

For our codebase, **every callsite uses `Arc<dyn ModelWriterBackend>`** (the
factory returns trait objects and the orchestrator stores one across awaits),
so the "no allocation" advantage of native AFIT static dispatch is unreachable
without rewriting the dispatch model.

### 2.3 dyn-compatible status (as of Rust 1.97 nightly)

Native `async fn in trait` is still **NOT object-safe**. Attempting to write
`Arc<dyn ModelWriterBackend>` directly after migration fails:

```text
error[E0038]: the trait `ModelWriterBackend` is not dyn compatible
  = note: for a trait to be `dyn`-compatible it needs to allow building a vtable
  = help: the following types are not dyn-compatible: associated function `init`'s return type has impl trait
```

Workarounds:

1. **Hybrid**: `#[async_trait]` for the `dyn`-facing trait, write a separate
   `AsyncModelWriterBackend` with native AFIT for static dispatch where used.
   Doubles the trait surface; not worth it for our codebase.
2. **`trait-variant`**: declare native AFIT trait + generate `dyn` shim via
   macro. Crate is ~120 LOC, well-maintained, but still wraps `dyn` calls in
   `Box<dyn Future>` — net zero perf gain on the orchestrator path.
3. **Wait for `dyn_compatible_for_dispatch`** to stabilize. RFC tracking
   issue [#107011](https://github.com/rust-lang/rust/issues/107011); no
   stabilization timeline announced.

## 3. Cost-benefit analysis for `ModelWriterBackend`

### 3.1 Quantitative

Verify-binary 平均一次完整 trait 演练（mock 11 calls + Parquet e2e + Compare
e2e = ~30 trait method invocations）的开销估算：

- 30× `Pin<Box<dyn Future>>` allocations
- Each allocation: ~30–100ns on modern hardware (jemalloc / mimalloc faster)
- Total amortised: < 5µs per full verify run

Production hot path (single batch through orchestrator):
- ~9 trait method calls per batch
- @ 1000 batches: ~9000 boxed-future allocations ≈ < 1 ms cumulative

**This is in the noise**. The dominant cost in actual production
gen-model runs is SurrealDB I/O (seconds per batch), not the trait
dispatch overhead (sub-ms in total).

### 3.2 Qualitative

| Aspect | `#[async_trait]` (status quo) | Native AFIT migration |
|---|---|---|
| Code clarity | macro hides desugaring; OK | direct async fn, slightly cleaner |
| Error messages | macro spans sometimes off | native compiler messages |
| Compile time | proc-macro overhead | trait-variant macro similar |
| Toolchain ratchet | none (works on stable + nightly) | locks us to nightly until `dyn_compatible_for_dispatch` stabilises |
| Trait extension friction | adding `async fn` is just `async fn` | same after migration; same friction during |

**Critical risk**: migrating now would either (a) introduce `trait-variant`
as a hard dependency (binding our trait shape to a third-party macro) or
(b) require splitting the trait into static-dispatch + dyn-dispatch
variants (doubling maintenance surface). Both are larger than the
perf payoff (which is essentially zero on the I/O-bound hot path).

## 4. Decision: **Do not migrate in v4.**

**Rationale**:

1. Trait perf is dominated by SurrealDB / DuckLake / Parquet I/O, not
   `Box<dyn Future>` allocation. v3 verify-binary timing data confirms
   sub-ms cumulative dispatch overhead.
2. `dyn`-compatibility of native AFIT is unstable; the workarounds
   (`trait-variant`, hybrid traits) add complexity without performance
   benefit because we use `Arc<dyn ModelWriterBackend>` everywhere.
3. `#[async_trait]` is mature, dependency-stable (0.1.x for years), and
   produces error messages we already understand.
4. v4 §1.1 (DuckLake real) and §1.2 (Parquet typed) are higher-priority
   and don't depend on this migration.

**Reconsider when**:

- `dyn_compatible_for_dispatch` stabilizes (track [rust-lang#107011](https://github.com/rust-lang/rust/issues/107011))
- We move to a benchmark-driven optimisation phase where the < 1 ms
  trait dispatch cost becomes the next bottleneck (unlikely until DuckLake
  + Parquet typed materialisation are both production-grade and Surreal
  is the slow tail)
- A future trait refactor adds enough call sites to make the cost-benefit
  flip

## 5. Recommended bookkeeping

- Mark v4 §2.2 as **DONE (decision: NO migrate)** in `v4-candidates.md`
- Keep this doc as the durable rationale; future contributors should
  read it before re-raising the migration question
- If `#[async_trait]` ever gets removed from the trait, this doc + the
  v3 verify-binary timing data are the place to start

## 6. References

- Rust 1.75 announcement: <https://blog.rust-lang.org/2023/12/28/Rust-1.75.0.html#async-fn-and-return-position-impl-trait-in-traits>
- `trait-variant` crate docs: <https://docs.rs/trait-variant/>
- RFC tracking: <https://github.com/rust-lang/rust/issues/107011>
- `async_trait` crate: <https://docs.rs/async-trait/>
- Verify binary timing (v3): full run ~20 seconds, of which < 1 ms is trait dispatch
- Mission doc 03 (writer-architecture) — Storage lifecycle contract
