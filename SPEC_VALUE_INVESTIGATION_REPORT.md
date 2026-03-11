# spec_value Backend Investigation Report

**Date:** 2026-03-11  
**Investigator:** backend-architect subagent  
**Request:** Investigate `spec_value` storage/exposure and grouping capability for nearby-query results

---

## Executive Summary

`spec_value` is **already stored** in Parquet instances and **partially available** in the database, but **NOT currently exposed** by the `/api/sqlite-spatial/query` endpoint. The lightest-weight solution is to **extend the spatial API response** with a lookup or join against `instances.parquet` (or a new lightweight index table).

---

## 1. Where `spec_value` Comes From

### 1.1 Data Source & Meaning

**Purpose:** Professional/discipline classifier for components  
**Mapping Logic:** (from `spec_info.rs`)

```rust
fn site_name_to_spec_value(name: &str) -> i64 {
    let name = name.to_uppercase();
    if name.contains("PIPE") { 1 }
    else if name.contains("ELEC") { 2 }
    else if name.contains("INST") { 3 }
    else if name.contains("HVAC") { 4 }
    else { 0 }
}
```

**Value Semantics:**
- `0` = Unknown/unspecified (fallback default)
- `1` = PIPE (Piping)
- `2` = ELEC (Electrical)
- `3` = INST (Instrumentation)
- `4` = HVAC (Heating/Ventilation/Air Conditioning)

### 1.2 Storage Locations

#### A. **Parquet Files** (Primary storage for exported data)

**File:** `output/<project>/parquet/<dbnum>/instances.parquet`

**Schema:**
```
instances.parquet:
  - refno_str: String
  - refno_u64: UInt64
  - noun: String
  - spec_value: Int64   ← HERE
  - owner_refno_str: String (nullable)
  - trans_hash: String
  - aabb_hash: String
  - has_neg: Boolean
  - dbnum: UInt32
```

**Derivation Logic** (from `export_dbnum_instances_parquet.rs:921-929`):

```rust
let mut spec_value = row.spec_value.unwrap_or(0);
if spec_value == 0 {
    // Try spec_info_map (BRAN/HANG/EQUI/WALL/FLOOR professional mapping)
    spec_value = *spec_info_map.get(&refno_to_u64(&row.refno)).unwrap_or(&0);
    
    // Fallback: use owner's spec_value (for components like ELBO/BEND)
    if spec_value == 0 {
        if let Some(owner) = &row.owner_refno {
            spec_value = *spec_info_map.get(&refno_to_u64(owner)).unwrap_or(&0);
        }
    }
}
```

**Coverage:**
- ✅ All instances in `instances.parquet` have `spec_value` (may be 0)
- ✅ BRAN/HANG/EQUI/WALL/FLOOR get mapped from SITE hierarchy
- ✅ Components inherit from owner if their own value is 0

#### B. **SurrealDB** (Runtime database)

**Table:** `inst_relate`  
**Field:** `spec_value: i64`

**Current Status:**
- ⚠️ **Disabled at ingestion** (hardcoded to 0 during initial data import)
- From `pdms_inst.rs:1600,1622`:
  ```rust
  // spec_value: 使用默认值 0（已禁用 DB 查询）
  spec_map.insert(refno, 0);
  ```
- ✅ **Enabled during Parquet export** (derived from `spec_info` TreeIndex traversal)

**Implication:** `spec_value` is NOT reliable in SurrealDB queries; only Parquet has accurate values.

#### C. **SQLite Spatial Index**

**File:** `output/spatial_index.sqlite`  
**Tables:**
- `items (id, noun)` ← Has `noun` but **NO `spec_value`**
- `aabb_index (id, min_x, max_x, ...)` ← Spatial R-tree only

**Current API Response** (`/api/sqlite-spatial/query`):
```rust
pub struct SpatialQueryResultItem {
    pub refno: String,
    pub noun: String,
    pub aabb: Option<AabbDto>,
}
```

**Missing:** `spec_value` is **not stored** in SQLite index and **not returned** by spatial API.

---

## 2. Can Current Nearby API Expose `spec_value`?

**Answer:** ❌ **NO** (but fixable with minimal changes)

### Current Spatial Query Flow

```
Frontend Request
    ↓
GET /api/sqlite-spatial/query?refno=17496_123456&distance=5000
    ↓
sqlite_spatial_api.rs::do_spatial_query()
    ↓
1. Query SQLite RTree index → get nearby `id` list
2. JOIN with `items` table → get `noun` for each id
3. Return {refno, noun, aabb}
```

**Bottleneck:** SQLite index has no `spec_value` column.

---

## 3. Enrichment Strategies (Lightest → Heaviest)

### ✅ **Option 1: Add `spec_value` to SQLite Index** (RECOMMENDED)

**Implementation:**

1. **Schema Change** (`sqlite_index.rs`):
   ```rust
   CREATE TABLE IF NOT EXISTS items (
       id INTEGER PRIMARY KEY,
       noun TEXT,
       spec_value INTEGER DEFAULT 0  -- NEW
   );
   ```

2. **Import Enhancement** (`sqlite_index.rs::import_from_instances_json`):
   - Read `spec_value` from `instances.parquet` during index build
   - Or extract from `instances.json` (already contains `spec_value` in groups)

3. **API Response Extension** (`sqlite_spatial_api.rs`):
   ```rust
   pub struct SpatialQueryResultItem {
       pub refno: String,
       pub noun: String,
       pub aabb: Option<AabbDto>,
       pub spec_value: Option<i64>,  // NEW
   }
   ```

4. **Query Join**:
   ```rust
   let spec_value: Option<i64> = stmt_spec
       .query_row([id], |r| r.get(0))
       .optional()
       .unwrap_or(None);
   ```

**Pros:**
- ✅ Minimal latency overhead (same SQLite query + one column)
- ✅ No additional API roundtrip
- ✅ Index rebuild required only once per data update

**Cons:**
- ⚠️ Requires re-importing spatial index (one-time operation)
- ⚠️ Schema migration needed

**Effort:** ~2-3 hours (schema + import logic + API update + testing)

---

### Option 2: Separate Batch Lookup Endpoint

**API Design:**
```
POST /api/parquet/spec-values
Body: { "refnos": ["17496_123456", "17496_789012", ...] }

Response: {
  "17496_123456": 1,  // PIPE
  "17496_789012": 2,  // ELEC
  ...
}
```

**Implementation:**
- Load `instances.parquet` into DuckDB (or Arrow IPC)
- Execute batch query: `SELECT refno_str, spec_value FROM instances WHERE refno_str IN (...)`

**Pros:**
- ✅ No SQLite schema change
- ✅ Direct access to Parquet data

**Cons:**
- ❌ Additional frontend API call (latency penalty)
- ❌ DuckDB/Parquet overhead for small queries
- ⚠️ Complex caching logic needed

**Effort:** ~4-6 hours

---

### Option 3: Real-time Parquet Join

**Implementation:**
- Spatial query returns refnos → backend joins with `instances.parquet` before response

**Pros:**
- ✅ Single API call

**Cons:**
- ❌ Parquet file I/O on every spatial query (slow)
- ❌ No indexing → O(n) scan or DuckDB query overhead

**Effort:** ~3-4 hours (not recommended due to performance)

---

## 4. `spec_value` Stability & Edge Cases

### 4.1 Missing/Null Values

**Fallback Cascade:**
1. Try element's own `spec_value`
2. If `0`, try `spec_info_map[refno]` (from TreeIndex SITE hierarchy)
3. If still `0`, try owner's `spec_value`
4. Final fallback: `0` (Unknown)

**Prevalence:**
- From code inspection: Most BRAN/HANG/EQUI get values from SITE traversal
- Components (ELBO/BEND/VALV) inherit from owner
- Orphaned/unclassified elements default to `0`

**Recommendation:**
- Frontend should **treat `0` as "Uncategorized"** group
- Display as "未分类" or "Other"

### 4.2 Consistency Across Sources

| Source | Reliability | Notes |
|--------|------------|-------|
| `instances.parquet` | ✅ **High** | Derived during export with full context |
| `inst_relate` (SurrealDB) | ❌ **Low** | Hardcoded to `0` at ingestion |
| `spatial_index.sqlite` | ❌ **None** | Not stored currently |

**Critical:** Only Parquet has authoritative `spec_value`.

### 4.3 Data Freshness

**Parquet Export Triggers:**
1. CLI: `--export-parquet`
2. Model generation: `--export-parquet-after-gen`
3. MBD Pipe: `trigger_async_parquet_export(dbnum)`

**Spatial Index Update:** Must be rebuilt when Parquet changes.

**Sync Risk:**
- ⚠️ If spatial index is stale, refnos may exist without corresponding Parquet records
- **Mitigation:** Return `spec_value: null` when lookup fails

---

## 5. Recommended API Shape

### 5.1 Enhanced Spatial Query Response

```typescript
// GET /api/sqlite-spatial/query?refno=17496_123456&distance=5000

interface SpatialQueryResponse {
  success: boolean;
  results: SpatialQueryResultItem[];
  truncated?: boolean;
  query_bbox?: AabbDto;
  error?: string;
}

interface SpatialQueryResultItem {
  refno: string;           // "17496_123456"
  noun: string;            // "PIPE"
  aabb?: AabbDto;
  spec_value?: number;     // 0-4, null if unavailable  ← NEW
}

// Frontend can group by spec_value:
const grouped = results.reduce((acc, item) => {
  const key = item.spec_value ?? 0;  // 0 = "Uncategorized"
  if (!acc[key]) acc[key] = [];
  acc[key].push(item);
  return acc;
}, {} as Record<number, SpatialQueryResultItem[]>);
```

### 5.2 Display Mapping (Frontend)

```typescript
const SPEC_VALUE_LABELS: Record<number, string> = {
  0: "未分类",
  1: "管道 (PIPE)",
  2: "电气 (ELEC)",
  3: "仪表 (INST)",
  4: "暖通 (HVAC)",
};
```

---

## 6. Implementation Recommendation

### Phase 1: SQLite Index Enhancement (Recommended)

**Files to Modify:**
1. `src/sqlite_index.rs`
   - Add `spec_value INTEGER DEFAULT 0` to `items` table schema
   - Update `insert_aabbs_with_items()` to accept `spec_value` parameter
   - Modify import logic to read `spec_value` from Parquet

2. `src/web_server/sqlite_spatial_api.rs`
   - Add `spec_value: Option<i64>` to `SpatialQueryResultItem`
   - Query `spec_value` from `items` table alongside `noun`

3. **Index Rebuild Script:**
   ```bash
   cargo run --release -- --import-spatial-index \
     --instances-json output/project/instances.json \
     --dbnum 17496
   ```

### Phase 2: Frontend Integration

**Changes in `plant3d-web`:**
1. Update `genModelE3dParquetApi.ts` type definitions
2. Modify nearby panel grouping logic
3. Add spec_value legend/labels

**Estimated Timeline:**
- Backend changes: 2-3 hours
- Index rebuild: 5-10 minutes (one-time)
- Frontend integration: 1-2 hours
- Testing: 1 hour

**Total:** ~4-6 hours end-to-end

---

## 7. Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|-----------|
| Stale spatial index | Missing `spec_value` for new items | Rebuild index in CI/CD pipeline |
| `spec_value = 0` dominance | Poor grouping UX | Surface as "Uncategorized" group explicitly |
| Parquet unavailable | API fails | Return `spec_value: null`, log warning |
| Multi-dbnum queries | Spec values from different projects mix | Document assumption: queries are dbnum-scoped |

---

## 8. Alternatives Considered

### Why NOT Real-time Parquet Join?
- ❌ Parquet I/O latency (~50-200ms per query)
- ❌ DuckDB initialization overhead
- ❌ File locking contention in multi-request scenarios

### Why NOT Separate Batch API?
- ❌ Requires frontend state management (pending → resolved)
- ❌ Race conditions if spatial results change during lookup
- ❌ Extra network roundtrip

### Why SQLite Index Extension Wins?
- ✅ Single query, minimal latency
- ✅ Co-located data (refno + noun + spec_value)
- ✅ Leverages existing RTree spatial filtering

---

## 9. Next Steps

1. **Confirm Approach** with parent agent/team
2. **Prototype Schema Change** in `sqlite_index.rs`
3. **Test with Sample Data** (dbnum 7997 or 17496)
4. **Update API Contract** documentation
5. **Frontend Coordination** for type definitions

---

## Appendix: Key Code Locations

| Component | File | Line Range |
|-----------|------|-----------|
| spec_value mapping | `src/fast_model/export_model/spec_info.rs` | 23-37 |
| Parquet export logic | `src/fast_model/export_model/export_dbnum_instances_parquet.rs` | 921-929 |
| SQLite index schema | `src/sqlite_index.rs` | 39-47 |
| Spatial query API | `src/web_server/sqlite_spatial_api.rs` | 96-99 |
| Import from Parquet | `src/sqlite_index.rs` | 244-580 |

---

**Conclusion:** `spec_value` is **available in Parquet** but **missing from spatial index**. The **lightest-weight solution** is to **extend SQLite `items` table** with `spec_value` column and populate it during index import. This enables frontend grouping with **zero latency overhead** beyond a single column fetch.
