# Investigation Report: inst_relate_booled_aabb Write Failure

## Executive Summary

The `inst_relate_booled_aabb` table doesn't exist after boolean operations because:
1. **DbLegacy mode uses direct DB writes (DbBoolWriter), not SQL file writes**
2. **Error handling silently logs failures without propagating them**
3. **SurrealDB tables are implicitly created on first INSERT, no explicit schema setup is run**
4. **The INSERT may be failing silently due to constraint violations or data issues**

## Detailed Findings

### 1. BoolResultWriter Implementation Selection (DbLegacy Mode)

**Location:** `src/fast_model/gen_model/manifold_bool.rs:2665-2680`

```rust
pub async fn run_bool_worker_from_tasks(
    tasks: Vec<BooleanTask>,
    db_option: Arc<aios_core::options::DbOption>,
    sql_writer: Option<Arc<SqlFileWriter>>,
) -> anyhow::Result<BoolWorkerReport> {
    let writer: Arc<dyn BoolResultWriter> = if let Some(ref w) = sql_writer {
        Arc::new(SqlBoolWriter::new(w.clone()))
    } else {
        Arc::new(DbBoolWriter)  // ← DbLegacy mode uses this
    };
```

**Answer to Question 1:**
- When `boolean_pipeline_mode=DbLegacy`, the code path is:
  - `orchestrator.rs:1040` → `run_boolean_worker()` → `booleans_meshes_in_db()` → `apply_cata_neg_boolean_manifold()` / `apply_insts_boolean_manifold()`
  - These functions call `update_booled_result()` directly at line `manifold_bool.rs:1942`
- **For MemoryTasks mode** (which the command doesn't use): `run_bool_worker_from_tasks()` is called with `sql_writer=None`, creating `DbBoolWriter`
- **DbBoolWriter** is instantiated when `sql_writer` is `None` (line 2675-2677)
- **SqlBoolWriter** is only used when `defer_db_write=true` (creates SQL file writer)

### 2. Writer Method Invocation Path

**Location:** `src/fast_model/gen_model/manifold_bool.rs:2105-2118`

```rust
impl BoolResultWriter for DbBoolWriter {
    fn deferred_mode(&self) -> bool { false }

    async fn write_inst_success(
        &self,
        refno: RefnoEnum,
        mesh_id: &str,
        aabb: Option<parry3d::bounding_volume::Aabb>,
    ) -> anyhow::Result<()> {
        update_booled_result(refno, mesh_id, aabb).await
    }
```

**Answer to Question 2:**
- In **DbLegacy mode**, `update_booled_result()` is called **directly** from `apply_cata_neg_boolean_manifold()` line 1942
- The `BoolResultWriter` trait is **NOT used** in DbLegacy mode
- Only in **MemoryTasks mode** does the code use `writer.write_inst_success()` which delegates to `update_booled_result()`
- The boolean worker in DbLegacy mode bypasses the writer abstraction entirely

### 3. Error Handling in batch_insert_aabb_table

**Location:** `src/fast_model/utils.rs:148-192`

```rust
async fn batch_insert_aabb_table(
    table: &str,
    inst_aabb_map: &DashMap<RefnoEnum, String>,
) {
    // ...
    let sql = format!("INSERT IGNORE INTO {table} [{}];", rows.join(","));
    if let Err(e) = model_primary_db().query_take::<surrealdb::types::Value>(&sql, 0).await {
        init_save_database_error(
            &format!("{sql}\n-- err: {e}"),
            &std::panic::Location::caller().to_string(),
        );
    }
}
```

**Location:** `/Volumes/DPC/work/plant-code/rs-core/src/error.rs:83-89`

```rust
pub fn init_save_database_error(sql: &str, position: &str) {
    HandleError::SaveDatabaseErr {
        sql: sql.to_string(),
        position: position.to_string(),
    }
    .init_log()  // ← Only logs via log::error!(), doesn't panic or return Err
}
```

**Answer to Question 3:**
- **YES, errors are silently swallowed**
- `batch_insert_aabb_table()` has NO return value (returns `()`)
- When INSERT fails, it calls `init_save_database_error()` which only logs to `error.log` via `log::error!()`
- The error is **never propagated** to the caller
- Calling code has no way to detect the failure
- The boolean operation appears successful even if AABB writes fail

### 4. Table Schema Initialization

**Location:** `/Volumes/DPC/work/plant-code/rs-core/resource/surreal/common.surql:962-975`

```sql
-- inst_relate_aabb / inst_relate_booled_aabb / inst_relate_bool 表定义

-- inst_relate_aabb: 原始几何 AABB
-- inst_relate_booled_aabb: 布尔运算后 AABB

DEFINE TABLE inst_relate_aabb SCHEMALESS;
DEFINE FIELD refno ON TABLE inst_relate_aabb TYPE record<pe>;
DEFINE FIELD aabb_id ON TABLE inst_relate_aabb TYPE record<aabb>;
DEFINE INDEX idx_inst_relate_aabb_refno ON TABLE inst_relate_aabb COLUMNS refno UNIQUE;

DEFINE TABLE inst_relate_booled_aabb SCHEMALESS;
DEFINE FIELD refno ON TABLE inst_relate_booled_aabb TYPE record<pe>;
DEFINE FIELD aabb_id ON TABLE inst_relate_booled_aabb TYPE record<aabb>;
DEFINE INDEX idx_inst_relate_booled_aabb_refno ON TABLE inst_relate_booled_aabb COLUMNS refno UNIQUE;
```

**Answer to Question 4:**
- Schema is defined in `.surql` files but **never automatically executed** during model generation
- The schema file exists at `rs-core/resource/surreal/common.surql`
- **No code runs these DEFINE statements before INSERT operations**
- SurrealDB implicitly creates tables on first INSERT (SCHEMALESS mode)
- However, **field type constraints and indexes are NOT applied** without explicit DEFINE
- The table may exist but **without proper schema/indexes**, leading to:
  - Type validation failures (refno/aabb_id may not match expected record types)
  - Index constraint violations if data doesn't match expected format

### 5. AABB Table INSERT Dependency

**Location:** `src/fast_model/utils.rs:72-80` and `src/fast_model/gen_model/manifold_bool.rs:608-618`

```rust
// From utils.rs:save_aabb_to_surreal()
let sql = format!("INSERT IGNORE INTO aabb [{}];", rows.join(","));
match model_primary_db().query(&sql).await {
    Ok(_) => {}
    Err(_) => {
        init_save_database_error(&sql, &std::panic::Location::caller().to_string());
    }
}

// From manifold_bool.rs:update_booled_result()
let aabb_map = DashMap::new();
aabb_map.insert(aabb_hash.to_string(), aabb);
crate::fast_model::utils::save_aabb_to_surreal(&aabb_map).await;  // ← Insert into aabb table

let inst_aabb_map = DashMap::new();
inst_aabb_map.insert(refno, aabb_hash.to_string());
crate::fast_model::utils::save_inst_relate_booled_aabb(&inst_aabb_map, "bool_mesh").await;  // ← Insert into inst_relate_booled_aabb
```

**Answer to Question 5:**
- **YES, there is a dependency chain:** `aabb` table → `inst_relate_booled_aabb` table
- The code first inserts into `aabb` table, then inserts a reference into `inst_relate_booled_aabb`
- **If `aabb` INSERT fails, it's silently logged** (same `init_save_database_error` pattern)
- The `inst_relate_booled_aabb` INSERT will still execute **but may fail** if:
  - The `aabb_id` field has type constraint `TYPE record<aabb>` (per schema)
  - The referenced `aabb:⟨hash⟩` record doesn't exist (foreign key-like constraint)
  - SurrealDB 3.x may enforce record type validation even in SCHEMALESS mode
- **Both failures are silently swallowed**, making debugging impossible

### 6. SQL File Writer Usage (Not Applicable to DbLegacy)

**Location:** `src/fast_model/gen_model/orchestrator.rs:561-574`

```rust
// defer_db_write 模式：初始化 SqlFileWriter
let sql_file_writer: Option<Arc<super::sql_file_writer::SqlFileWriter>> = if defer_db_write {
    let output_dir = db_option.get_project_output_dir();
    let path = super::sql_file_writer::SqlFileWriter::default_path(&output_dir, None);
    match super::sql_file_writer::SqlFileWriter::new(&path) {
        Ok(w) => {
            println!("[gen_model] 🗂️ defer_db_write 模式已启用，SQL 输出到: {}", path.display());
            Some(Arc::new(w))
        }
        // ...
```

**Location:** `src/fast_model/gen_model/orchestrator.rs:1037-1041`

```rust
match db_option.boolean_pipeline_mode {
    BooleanPipelineMode::DbLegacy => {
        if use_surrealdb && !defer_db_write {  // ← Command uses this branch
            if let Err(e) = run_boolean_worker(Arc::new(db_option.inner.clone()), 100).await {
                eprintln!("[gen_model] IndexTree 布尔运算失败（db_legacy）: {}", e);
            }
        }
```

**Answer to Question 6:**
- **SqlFileWriter is NOT used** when `defer_db_write=false` (the command's configuration)
- SqlBoolWriter (which uses SqlFileWriter) is only instantiated when `sql_writer.is_some()`
- In DbLegacy mode with `defer_db_write=false`, all writes go directly to SurrealDB
- **No SQL files are generated or executed** in this configuration
- The writes happen inline during boolean execution via `model_primary_db().query()`

## Root Cause Analysis

The `inst_relate_booled_aabb` table doesn't exist because:

1. **Schema Never Applied**: The `DEFINE TABLE` statements in `common.surql` are never executed before the INSERT
2. **Silent Failures**: When INSERT fails (likely due to type constraints), errors are only logged, never propagated
3. **Foreign Key Constraint**: The `aabb_id TYPE record<aabb>` constraint may fail if:
   - The referenced `aabb:⟨hash⟩` doesn't exist
   - The `aabb` table INSERT failed first (also silently)
4. **No Validation**: Without explicit schema, SurrealDB may accept malformed data or reject it inconsistently

## Verification Steps

To confirm the root cause:

1. **Check error.log**: Look for `SaveDatabaseErr` entries
   ```bash
   grep "inst_relate_booled_aabb\|SaveDatabaseErr" error.log
   ```

2. **Check if aabb table exists and has data**:
   ```sql
   SELECT count() FROM aabb GROUP ALL;
   SELECT * FROM aabb LIMIT 5;
   ```

3. **Check table schema**:
   ```sql
   INFO FOR TABLE inst_relate_booled_aabb;
   INFO FOR TABLE aabb;
   ```

4. **Manually run schema definition**:
   ```bash
   surreal import --conn ws://localhost:8000 --user root --pass root --ns test --db test rs-core/resource/surreal/common.surql
   ```

5. **Test INSERT manually**:
   ```sql
   -- First insert test aabb
   INSERT INTO aabb { id: aabb:⟨test123⟩, d: {"mins": [0.0, 0.0, 0.0], "maxs": [1.0, 1.0, 1.0]} };
   
   -- Then insert test inst_relate_booled_aabb
   INSERT INTO inst_relate_booled_aabb { 
       id: inst_relate_booled_aabb:⟨17496_106028⟩, 
       refno: pe:⟨17496_106028⟩, 
       aabb_id: aabb:⟨test123⟩ 
   };
   ```

## Recommendations

1. **Add explicit schema initialization** in model generation startup
2. **Change error handling** to propagate failures instead of silent logging
3. **Add validation** after AABB writes to confirm success
4. **Log AABB write operations** at info level for debugging
5. **Consider transaction** for aabb + inst_relate_booled_aabb writes

## Related Code Locations

- Boolean orchestration: `src/fast_model/gen_model/orchestrator.rs:1037-1041`
- Boolean execution: `src/fast_model/gen_model/mesh_generate.rs:1150-1205`
- AABB write: `src/fast_model/gen_model/manifold_bool.rs:580-618`
- Error handling: `rs-core/src/error.rs:83-89`
- Table schema: `rs-core/resource/surreal/common.surql:962-975`
