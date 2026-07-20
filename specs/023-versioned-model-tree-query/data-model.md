# Data Model: 按版本实时查询模型树（versioned pe_owner）

**Date**: 2026-07-19 | **Plan**: `plan.md`

本 feature 不新增业务表，复用两个既有实体并新增一条元记录。

## pe_owner（既有关系表，核心数据源）

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | `pe_owner:[<owner pe 记录键>, <子序号 int>]` | 数组 id；第二段承载同胞顺序（0 起） |
| `in` | `record<pe>` | 子节点 |
| `out` | `record<pe>` | 父节点（owner） |

**版本语义**: 实例级 `versioned=true` 下，边的创建/删除自动携带存储时间戳；`VERSION $t` 查询返回 t 时刻存在的边集合（research C1 实测验证，含"删后重建同 id"场景）。

**写入约束**:
- 幂等策略统一**先删后插**：重写 owner X 的子列表前先 `DELETE pe:<X><-pe_owner`（撞 id 且值不同会报错，`INSERT IGNORE RELATION` 语法不存在——research C6/C7）
- 增量：边维护 SQL 与 PE/ATT 同一 `mutation_sqls` 批次，参与 commit fingerprint 与 counts
- 删除元素 X：`DELETE pe:<X>->pe_owner`（membership）+ `DELETE pe:<X><-pe_owner`（名下子边）

**读取约束**:
- 版本查询只允许图遍历（`pe:<X><-pe_owner`）或边记录点查；**禁止 id 区间扫 + VERSION**（research C3：静默返回当前态）
- 顺序以 `record::id(id)[1]` 显式排序兜底

## pe.children（既有字段，保底数据源）

| 字段 | 类型 | 说明 |
|------|------|------|
| `children` | `array<record<pe>>` | owner 上的有序子列表冗余；全量（pe.rs 注入）与增量（inject_children_into_pe_json）均维护 |

**用途**: ① `maintained_since_sesno` 之前的历史锚点回退查询（`SELECT VALUE children FROM pe:<X> VERSION $t`，research C4 实测正确）；② 与 pe_owner 的交叉一致性校验基准。

## sesno_version_anchor（specs/022 既有，只读复用）

`{dbnum, sesno, anchored_at, source, fingerprint}`——版本入参唯一桥梁；本 feature 不改其写入语义。换算入口：rs-core `resolve_anchor` / SurrealQL `fn::sesno_version(dbnum, sesno)`。

## pe_owner_version_meta（新增元记录）

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | `pe_owner_version_meta:<dbnum>` | 每 dbnum 一条 |
| `maintained_since_sesno` | `int` | 自该 sesno（含）起 pe_owner 历史可信 |
| `updated_at` | `datetime` | 写入时刻 |
| `source` | `string` | `full_reload` / `rebuild_cli` |

**状态迁移**（2026-07-19 实现期修正：增量不写 meta）:
- 全量重灌（M2）：UPSERT 为本次 full sesno（重置可信起点）
- 存量重建 CLI（M5）：UPSERT 为重建时刻的 latest_sesno
- ~~增量首次成功维护边：create-once~~ **废弃**——增量只重写"本批变更过的 owner"，
  修不了旧二进制时期已陈旧的边；由首个新增量打可信标记会产生静默错误历史。
  可信起点只能由全量重灌或重建 CLI（全量重建边）建立；meta 缺失时读侧一律回退 pe.children（天然正确）。

**读侧规则（FR-008）**: `requested_sesno >= maintained_since_sesno` → 主路径 pe_owner；否则（或记录缺失）→ 回退 pe.children，响应 `version.source = "pe_children_fallback"`。
