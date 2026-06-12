# Tasks

> 按序执行；每个 Task 后跑 `cargo check -q`（不跑 test）。
> 验收走 `cargo build --release` + 三场景实测（空库部署 / 不清理重跑 / regen）。

## 写入层幂等化

- [x] **T201 `inst_relate_aabb` DB 直写幂等化**
  - `save_instance_data_with_report` / `save_inst_relate_aabb_rows`：逐 chunk `DELETE+INSERT` → `INSERT IGNORE`；
    同批次 id 去重保留（51c7f516 的 `dedupe_inst_relate_aabb_rows`）。
- [x] **T202 删除写入前 `inst_relate` 扫描删除**
  - 移除 `save_instance_data_with_report` 中无条件 `delete_inst_relate_by_in` 调用与函数本身；
    `build_delete_inst_relate_by_in_sql` 保留（pre_cleanup / 导出 replace_exist 块仍用）。
- [x] **T203 关系表写入统一 IGNORE**
  - `inst_relate` / `geo_relate` / `neg_relate` / `ngmr_relate`（replace_exist 分支）/ `tubi_info` 写入语句补 `IGNORE`；
    `infer_failed_sql_stage` 匹配串同步。
- [x] **T204 `.surql` 导出路径同口径**
  - `save_instance_data_to_sql_file` 的 `inst_relate_aabb`：`DELETE+INSERT`（无 IGNORE）→ `INSERT IGNORE`。
- [x] **T205 过时注释清理**
  - 移除两处「必须为偶数：DELETE+INSERT 成对语句」注释（语句已不成对）。
- [x] **T206 `save_tubi_info_batch` 删预查**（grill-me Q3）
  - 删除 `query_existing_tubi_info_ids` 预查与过滤，全量直接 `INSERT IGNORE`；
    返回值语义改「提交条数」并更新 doc 注释；`query_existing_tubi_info_ids` 无其他调用方，一并删除。

## 文档收尾

- [x] **T207 spec 004 注记**
  - `specs/004-model-generation-write-pipeline-performance/spec.md` Decisions 表 Q4 加注：
    「两阶段方案被 spec 005 取代（入口清理 + INSERT IGNORE 幂等写入）」。
- [x] **T208 CHANGELOG**
  - 记录：写入管线统一 INSERT IGNORE、部署零旧数据检查、`already exists`/`Cannot COMMIT` 事故源消除。

## 验收（spec Acceptance Criteria 三场景）

- [ ] **T209 场景 1：空库首次部署**
  - 站点部署 parse + generate 退出 0；generate 日志无 DELETE 扫描 / tubi 预查痕迹；
    三表（`inst_relate` / `inst_relate_aabb` / `tubi_info`）行数与生成报告一致；
    记录 `base_write_ms` / `inst_aabb_ms` 观测值（供 spec 004 参考，不作硬验收）。
- [ ] **T210 场景 2：不清理重跑 generate**
  - 同站点不删库再跑 generate：退出 0，零 `already exists` / `Cannot COMMIT`，三表行数与场景 1 持平。
- [ ] **T211 场景 3：`--regen-model` 重生成**
  - pre_cleanup 删除统计正常输出；重生成成功；抽查 ≥3 个 refno 的 `inst_relate_aabb.aabb_id` 为本轮新值。
