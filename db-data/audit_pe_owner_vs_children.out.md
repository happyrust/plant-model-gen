# audit_pe_owner_vs_children — pe_owner 边 vs pe.children 完整性审计结果

- 生成时间：2026-07-20 18:48:22 +08:00（由 scripts/smoke/pe_owner_children_audit.ps1 生成，重跑会覆盖本文件）
- 实例：`http://127.0.0.1:8030`（HTTP POST /sql，Basic root/***）
- 环境：`ns=smoke023` `db=latest_tree` `dbnum=ALL`
- 审计 SQL：`db-data/audit_pe_owner_vs_children.surql`（判定规则与用法见该文件头注释）

## 判定

**PASS：抽样 4 个 parent 边计数与 children 全部一致，未发现残留脏边（抽样口径，全量修复口径见 rebuild-pe-owner）。**

## [1] 总量（per dbnum parents/children + 边行数）

```json
{
    "children_total":  6.0,
    "dbnum":  9902,
    "parents_with_children":  4
}
```

```json
{
    "pe_owner_edge_rows":  6
}
```

## [2] 抽样 parent 对比（count(<-pe_owner) vs len(children)）

```json
{
    "mismatched":  0,
    "sampled":  4
}
```

不一致清单（最多 50 条）：

```json
[]
```

## [3] childless 抽样残留脏边（最多 50 条）

```json
[]
```

## 修复口径

边不完整/陈旧/脏边：对该 dbnum 执行 `model-version rebuild-pe-owner --dbnum <n>`（先删后插全量重建，
幂等可重跑）或全量重灌；修完重跑本审计确认 PASS 后才允许该库走 pe_owner latest 树查询主路径。
