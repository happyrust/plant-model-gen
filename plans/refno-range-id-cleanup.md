# Refno Range ID 新架构重构计划

## Summary

将模型生成产物表的 SurrealDB record id 重构为 SurrealDB 3.1 range-friendly array id，支持按 `ref0` 快速删除。

这是全新架构：

- 不兼容旧 `table:⟨ref0_ref1⟩` 数据
- 不做旧数据迁移
- 不做 dual write
- 不做 dual cleanup

`pe` 主表 id 保持不变，模型关系仍通过 `in/out` 字段维持图关联。

## Worktree

当前主 worktree 有大量未提交改动，不能直接实施重构。执行阶段使用独立 worktree：

```powershell
git worktree add -b codex/refno-range-id-cleanup D:\work\plant-code\plant-model-gen-range-id HEAD
```

所有代码修改、验证、提交都在：

```text
D:\work\plant-code\plant-model-gen-range-id
```

本计划文件保存为：

```text
plans/refno-range-id-cleanup.md
```

## Key Design

新增统一 record id helper，建议放在：

```text
src/fast_model/gen_model/model_record_id.rs
```

所有模型生成产物 id 使用统一结构：

```text
[ref0, ref1, sesno, ...extra]
```

规则：

- `ref0` 为 PDMS refno 第一段
- `ref1` 为 PDMS refno 第二段
- `sesno = 0` 表示 latest/current
- `sesno > 0` 表示历史版本
- `extra` 只用于一对多表

一对一表：

```text
inst_relate:[ref0, ref1, sesno]
inst_relate_aabb:[ref0, ref1, sesno]
inst_relate_bool:[ref0, ref1, sesno]
inst_relate_cata_bool:[ref0, ref1, sesno]
refno_relations:[ref0, ref1, sesno]
```

一对多表：

```text
geo_relate:[ref0, ref1, sesno, geo_index]
neg_relate:[carrier_ref0, carrier_ref1, carrier_sesno, target_ref0, target_ref1, target_sesno, geo_index]
ngmr_relate:[carrier_ref0, carrier_ref1, carrier_sesno, target_ref0, target_ref1, target_sesno, ngmr_ref0, ngmr_ref1, ngmr_sesno, geo_index]
```

`pe` id 不变。`inst_relate` 仍是 relation table：

```text
inst_relate.in = pe:...
inst_relate.out = inst_info:...
```

因此 `pe->inst_relate` 图遍历继续成立。

## Record ID Helper

建议提供这些 helper：

```rust
pub fn refno_id_parts(refno: RefnoEnum) -> (u32, u32, u32);
pub fn model_refno_id(table: &str, refno: RefnoEnum) -> String;
pub fn model_refno_child_id(table: &str, refno: RefnoEnum, child: impl Display) -> String;
pub fn model_ref0_range(table: &str, ref0: u32) -> String;
pub fn model_refno_range(table: &str, refno: RefnoEnum) -> String;
```

语义：

```rust
model_refno_id("inst_relate", 24381/145569)
// inst_relate:[24381, 145569, 0]

model_refno_child_id("geo_relate", 24381/145569, 2)
// geo_relate:[24381, 145569, 0, 2]

model_ref0_range("inst_relate", 24381)
// inst_relate:[24381, NONE]..=[24381, ..]

model_refno_range("geo_relate", 24381/145569)
// geo_relate:[24381, 145569, 0, NONE]..=[24381, 145569, 0, ..]
```

`RefnoEnum::Refno` 使用 `sesno=0`。`RefnoEnum::SesRef` 使用真实 `sesno`。

## Implementation Changes

替换旧 id helper：

```rust
key.to_inst_relate_key()
key.to_table_key("inst_relate_aabb")
format!("geo_relate:⟨{}⟩", relate_id)
format!("inst_relate_bool:⟨{}⟩", refno)
```

统一改为新 array id helper。

`geo_relate` 不再使用：

```rust
gen_string_hash(relate_json)
```

作为主 id。改为每个 carrier refno 内稳定递增的 `geo_index`：

```text
geo_relate:[ref0, ref1, sesno, 0]
geo_relate:[ref0, ref1, sesno, 1]
geo_relate:[ref0, ref1, sesno, 2]
```

`neg_relate` / `ngmr_relate` 必须保存并引用完整的新 `geo_relate` record id，不再传裸 hash/string id。

所有直接从旧 id 推导记录的查询必须替换：

```sql
type::record("inst_relate_aabb", record::id(in))
record::exists(type::record("inst_relate_aabb", record::id(in)))
inst_relate_aabb:`...`
inst_relate_bool:⟨...⟩
```

Rust 侧统一使用 helper 预构造新 id。SurrealQL 内如果不方便构造 array id，改为图遍历或字段查询，但清理路径必须使用 record id range。

## Range Cleanup

新增 range cleanup 主路径。对每个目标 `ref0` 执行：

```sql
DELETE inst_relate:[ref0, NONE]..=[ref0, ..];
DELETE geo_relate:[ref0, NONE]..=[ref0, ..];
DELETE neg_relate:[ref0, NONE]..=[ref0, ..];
DELETE ngmr_relate:[ref0, NONE]..=[ref0, ..];
DELETE inst_relate_aabb:[ref0, NONE]..=[ref0, ..];
DELETE inst_relate_bool:[ref0, NONE]..=[ref0, ..];
DELETE inst_relate_cata_bool:[ref0, NONE]..=[ref0, ..];
DELETE refno_relations:[ref0, NONE]..=[ref0, ..];
```

清理入口策略：

- `--dbnum` / `manual_db_nums` / 全库：解析 dbnum 覆盖的所有 `ref0`，逐个执行 range cleanup。
- 显式 refno：按目标 refno 的 `[ref0, ref1, sesno, ...]` 范围删除，不用 min/max 推断子树连续性。
- `inst_geo` 仍按 `geo_relate.out` 收集 hash 后删除，继续跳过 hash `< 10`。

`refno_assoc_index` 不再作为清理主路径。新架构下清理依赖 range id。

## Test Plan

不要运行 `cargo test`。

使用 CLI + JSON/Surreal 查询验证。

基础 range 行为：

```sql
CREATE inst_relate:[24381, 1, 0] CONTENT { dbnum: 1 };
CREATE inst_relate:[24381, 2, 0] CONTENT { dbnum: 1 };
CREATE inst_relate:[24382, 1, 0] CONTENT { dbnum: 2 };

DELETE inst_relate:[24381, NONE]..=[24381, ..];
```

验收：

- 只删除 `24381`
- 不删除 `24382`

生成链路验证：

- 生成一个包含多 geometry 的 refno，确认 `geo_relate:[ref0, ref1, 0, index]` 无冲突。
- 验证 `pe:...->inst_relate` 仍能查到 relation。
- 验证 `inst_relate.out->geo_relate` 仍能查到 geometry。
- 验证 `neg_relate/ngmr_relate` 能通过新的 `geo_relate` id 正确关联。
- 对一个 dbnum 执行 regen，确认 cleanup SQL 数量按 `ref0` 数量增长，不按 refno 数量增长。

如涉及 `web_server`，启动服务后通过 HTTP/POST 验证，不写 Rust test。

针对 `aios-database`，使用 CLI + JSON 查询结果验证。

## Acceptance Criteria

- 新生成的数据只使用 array record id。
- 清理路径只使用 SurrealDB record id range，不使用 `WHERE refno/dbnum IN ...` 作为主删除策略。
- `pe` 图关系保持可遍历。
- 一对多表不会因为同 refno 多记录产生 id 冲突。
- 不保留旧 id 兼容代码。
- 不新增旧数据迁移逻辑。

## Assumptions

- 这是全新架构，不需要兼容已有旧数据。
- 已能从 dbnum 获得对应 `ref0` 列表。
- SurrealDB 版本支持 3.1 record ID range 和 array record id natural sorting。
- `sesno=0` 是 latest/current 的固定编码。
