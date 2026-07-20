# Research: fork surreal 对 versioned pe_owner 查询的能力验证（FR-011）

**Date**: 2026-07-19

**环境**: fork surreal `3.2.0-nightly`（D:\work\plant-code\surrealdb, dev-3.1 构建），versioned 实例 `127.0.0.1:8030`（`rocksdb://...?versioned=true&retention=30d`，`db-data/run_surrealkv_versioned.ps1`）

**脚本**: `scripts/smoke/pe_owner_version_capability_smoke.ps1` + `db-data/smoke_023_pe_owner_version.surql`（ns=`smoke023` db=`cap`，独立于业务数据，可重复执行）

**原始结果**: `db-data/smoke_023_result.json`

## 实验设计

状态 1（t1）：`pe:p` 有序子 `[a, b]`（pe_owner 边 `[p,0]→a`、`[p,1]→b`）。
状态 2（t2）：模拟一次增量——删 b（边+记录）、增 c（复用边 id `[p,1]`）、a 改名、更新 `p.children`。

对 t1/t2 两个时刻分别用五种写法查询，核对语义正确性。

## 结论矩阵

| # | 能力 | 写法 | 语法 | **语义** | 判定 |
|---|------|------|------|----------|------|
| C0 | VERSION 点查（基线） | `SELECT ... FROM pe:a VERSION $t` | ✅ | ✅ t1 旧名 / t2 新名 | 可用（022 已验证，本次复核） |
| C1 | VERSION + 图遍历 | `SELECT VALUE in FROM pe:p<-pe_owner VERSION $t` | ✅ | ✅ t1=[a,b]、t2=[a,c] | **选用（children 主查询）** |
| C2 | VERSION + 图 idiom | `SELECT VALUE <-pe_owner.in FROM ONLY pe:p VERSION $t` | ✅ | ✅ 同 C1 | 可用（等价写法） |
| C3 | VERSION + id 区间扫 | `FROM pe_owner:[pe:p,0]..=[pe:p,4294967295] VERSION $t` | ✅ | ❌ **t1 返回了当前态 [a,c]**（不回溯） | **禁用**：语法接受但 VERSION 不生效，静默返回错误数据 |
| C4 | VERSION 点查 children 字段 | `SELECT VALUE children FROM pe:p VERSION $t` | ✅ | ✅ t1=[a,b]、t2=[a,c] | 可用（**保底/回退路径**） |
| C5 | 已删除记录历史点查 | `SELECT ... FROM pe:b VERSION $t1` / 当前 | ✅ | ✅ t1 返回删除前值，当前为空 | 可用 |
| C6 | INSERT RELATION 撞已有 id（in 不同） | — | — | 报错 `Found pe:a for the ... does not match the existing field value`，不覆盖 | 幂等不能靠"重插" |
| C7 | `INSERT IGNORE RELATION INTO` | — | ❌ parse error（`Unexpected token INTO`） | — | 语法不存在，不可用 |

## 决策（回填 spec FR-011）

- **Decision 1 — children 查询写法**：图遍历 `SELECT VALUE in FROM pe:<owner><-pe_owner ORDER BY id VERSION $t`；**严禁 id 区间扫 + VERSION**（C3 实测语义错误，会静默返回当前态）。排序写法注意（2026-07-19 补充实测，见 incr_shapes smoke）：`ORDER BY record::id(id)[1]` 是解析错误（fork 不接受 ORDER BY 内函数表达式）；`ORDER BY id` 可用（数组 id 按 [owner, 序号] 结构序排序），子查询别名排序亦可作为兜底。
  - Rationale: C1/C2 语义实测正确；C3 是唯一语义翻车的写法。
  - Alternatives: C3 区间扫（性能理论最好，但 VERSION 不回溯，弃）；C4 点查 children（保留为回退，见 Decision 3）。
- **Decision 2 — 边写入幂等策略**：统一"先删后插"——重写某 owner 的子列表前先 `DELETE pe:<owner><-pe_owner`，再 `INSERT RELATION` 新列表。
  - Rationale: 撞 id 且值不同直接报错（C6）；`INSERT IGNORE RELATION` 语法不存在（C7）；先删后插同时天然覆盖"子数变少"的残留边问题，且在 versioned 存储里删除本身就是一次可回溯的版本事件（C5/C1 已验证 delete→VERSION 语义正确，本实验状态 2 正是删+重建同 id 边，t1 查询不受污染）。
- **Decision 3 — 回退路径**：`pe.children` 字段 VERSION 点查（C4 实测正确）。用于：① pe_owner 历史不可信区间（功能上线前锚点 / 站点未完成重建）；② 若图遍历在真实规模下性能不达标的兜底。
- **Decision 4 — 时间戳换算**：沿用 022 锚点体系（`fn::sesno_version(dbnum, sesno)` / rs-core `resolve_anchor`），本 feature 不新增任何时间戳来源。实验中两状态间 200ms 间隔即可稳定区分版本（HLC 粒度足够）。

## 遗留验证项（进 tasks）

- ~~同值重插（in/out 完全一致）是否幂等通过~~ **已关闭（2026-07-19，T001/C8 实测）**：同值重插返回 OK 且边完好（值不同才报错，C6）。先删后插策略不受影响。
- 图遍历 + VERSION 在真实规模（单 owner 数百子、库内千万 pe）下的延迟——进 M4 端到端 smoke，用真实站点 fixture 核对 SC-002（P95 ≤ 1s）。
- 图遍历返回顺序是否恒等于边 id 序（本实验有序）；实现层显式 `ORDER BY record::id(id)[1]` 兜底，不依赖隐式顺序。
