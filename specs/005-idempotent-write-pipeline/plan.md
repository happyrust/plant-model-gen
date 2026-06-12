# Plan: 模型写入管线幂等化

## 职责划分（目标态）

```
                    ┌──────────────────────────────────────────────┐
站点部署 (空库)      │ 写入层 (pdms_inst.rs)                         │
sidecar generate ──→│  · INSERT IGNORE / INSERT RELATION IGNORE    │──→ SurrealDB
（不带 regen，       │  · 同批次同 id 去重 (dedupe, 保留最后一行)      │
 零清理零预查）       │  · 永不 DELETE / 永不预查旧数据                │
                    └──────────────────────────────────────────────┘
                                      ↑ 前置条件：目标行不存在
--regen-model ──→ pre_cleanup_for_regen（唯一清理权威）──┘
                  · refno_assoc_index 快路径 / Legacy 扫描降级
                  · 删 inst_relate / inst_info / geo_relate / inst_geo /
                    neg_relate / ngmr_relate / inst_relate_aabb / tubi_relate
```

历史形态（已废弃）：写入层逐 chunk `DELETE + INSERT` 成对替换 + 写入前
`delete_inst_relate_by_in` 扫描 + `tubi_info` existing 预查。三者在部署
场景全是白跑，且成对语句是 `already exists` / `Cannot COMMIT` 的事故源。

## 改动清单

### 已落地（工作区，基于 51c7f516 之上）

| # | 位置 | 改动 |
|---|---|---|
| 1 | `save_instance_data_with_report` | `inst_relate_aabb` 逐 chunk `DELETE+INSERT` → 纯 `INSERT IGNORE`（去重保留） |
| 2 | `save_inst_relate_aabb_rows` | 同上（mesh 收尾批量写路径） |
| 3 | `save_instance_data_with_report` | 删除写入前无条件 `delete_inst_relate_by_in(&inst_refnos)` 调用；函数本身删除（`build_delete_inst_relate_by_in_sql` 保留，pre_cleanup 与导出 replace_exist 块仍用） |
| 4 | 同上 | `inst_relate` / `geo_relate` / `neg_relate` / `ngmr_relate`（replace_exist 分支）写入补 `IGNORE` |
| 5 | `save_tubi_info_batch` | INSERT 补 `IGNORE` |
| 6 | `save_instance_data_to_sql_file` | `inst_relate_aabb` 导出与 DB 直写同口径：`DELETE+INSERT`（无 IGNORE）→ `INSERT IGNORE` |
| 7 | 注释 | 清理两处「必须为偶数：DELETE+INSERT 成对语句」过时注释 |

前置提交 51c7f516（同批次 id 去重 + MAX_TX_STATEMENTS 偶数配对保护）中，
去重逻辑保留（不变量 3），偶数约束随成对语句消失而失去必要性（保留无害）。

### 待办

| # | 位置 | 改动 |
|---|---|---|
| 8 | `save_tubi_info_batch` | 删除 `query_existing_tubi_info_ids` 预查与过滤，全量直接 `INSERT IGNORE`；返回值语义改为「提交条数」（仅 debug 日志消费）；`query_existing_tubi_info_ids` 若无其他调用方一并删除 |
| 9 | `specs/004-model-generation-write-pipeline-performance/spec.md` | Decisions 表 Q4 注记「两阶段方案被 spec 005 取代（入口清理 + INSERT IGNORE 幂等写入）」 |
| 10 | `CHANGELOG.md` | 记录写入幂等化变更 |

## 关键决策依据

- **为什么不是两阶段写入（004 Q4 旧方案）**：两阶段（收集 → 收尾统一删除 + bulk insert）仍保留收尾 DELETE，复杂度高（spool 文件、touched_refnos 追踪）；而「入口清理 + IGNORE」把清理挪到 regen 唯一入口，部署场景彻底零清理，写入层零状态。
- **IGNORE 静默保留旧值的风险**（Q2）：仅在 `--regen-model` 且 pre_cleanup 漏删时发生；漏删本身是清理层 bug（refno_assoc_index 有逐表删除统计日志可审计），写入层兜底等于退回逐行 DELETE 老路。
- **tubi_info 预查为何可删**（Q3）：IGNORE 已是幂等闸，预查只为返回精确新增数，该返回值仅进 debug 日志。

## 风险与对策

- **R1 pre_cleanup 漏删 → regen 旧值存活**：契约上归清理层；regen 验收场景 3 抽查同 refno 新 aabb_id 可暴露。
- **R2 IGNORE 掩盖真实写入错误**：IGNORE 只跳过主键冲突，schema/事务错误仍报错并走 failed_sql 转储（`infer_failed_sql_stage` 匹配串已同步更新）。
- **R3 `.surql` 导出路径无调用方**（`save_instance_data_to_sql_file` 当前 0 引用）：仍统一口径，避免未来接线时语义分叉。

## 验证方式

仓库规则不跑 Rust test：`cargo build --release` + 三场景实测（见 spec Acceptance Criteria）。
