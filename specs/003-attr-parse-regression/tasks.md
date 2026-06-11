# Tasks（spec 003 — CATA 属性解析回归修复）

> 每个 Task 后跑 `cargo check -q`（不跑 test）。验证一律 CLI + JSON 对比。
> 金标准：`runtime/bran-closure/e3d_golden_reference.json`（E3D Q ATT 原文）。

## 已完成的前置（审计阶段，2026-06-11）

- [x] **T000 字段级审计与金标准建立**
  - 三方对比（8020 / e2e 全量 / 按需）：元素级零缺失；字段级差异 207 处 6 类。
  - E3D `Q ATT` 正宗参考（4 refno）入档 fixture；E 类反转定性（新解析正确）。
  - 工具：`bran_full_field_audit.py` / `valv_data_audit*.py` / `field_diff*.py`。

## 修复

- [x] **T001 LEVE 数组截断修复（已完成，pdms-io-fork 5d46d68e）**
  - `implicit.rs::parse_int_array` 白名单补 `LEVE` 别名 + 计数防御。
  - **实现偏差**：原计划"移除白名单按元数据读计数"实测不可行 —— 其余 INTVEC
    属性头 4 字节并非计数，通用化会误读邻接字节、污染同元素后续几何属性
    （ATTA/SCTO 几何构建当场回归），已收缩为白名单别名方案。
  - 验证：13246_243926 `LEVE=[8,10]` ✓；63 处截断归零（审计 207→144 处）；
    BRAN/FITT 生成 cata_hash 不回退。
  - 连带语义修正（正确行为）：LEVE=[8,10] 高级别几何（中心线/支架标记）在
    默认级别 6 下被正确过滤 —— ATTA 24381_145020 不再产出实例（17→16 管件
    实例 + FITT），与 E3D 渲染语义对齐。
  - 事故记录：验证期间发现 rs-core 工作区 prim_geo 三文件被外部进程回退到
    提交前状态（U 弯代码丢失导致假回归），已 `git restore --source=HEAD` 恢复；
    并行会话操作共享依赖仓的工作区需警惕。
- [ ] **T002 B/C/D 字节级定位与修复**
  - 新增 `parse_pdms_db` 单元素 dump example（输入 db 文件 + refno，输出全属性
    JSON），对 13246_243891/243869/243899 跑三列对照（E3D/8020/新）。
  - 修 PTCD（`AXIS -Z` 与裸 `Y` 两种原文）、PHEI 纯数字字符串（score 降权误杀
    则放宽）、SDTE.SKEY/RTEX（STRING 路径或类型布局）。
  - 字段静默丢失（双引擎全失败）补 debug 统计口径，禁止无声丢字段。
- [ ] **T003 INTVEC 同类隐患全字典扫描**
  - 扫字典中全部 INTVEC 属性，列出走隐式路径且可能被 count=1 截断的属性清单；
    逐一核对 8020 产物中数组长度 >1 的样本。
- [ ] **T005 PTCD 消费端兼容排查**
  - rs-core `resolve_axis_params` / expression 轴解析对 `AXIS -Z` 前缀与裸 `Y`
    的兼容；确保 ptset dir 推导两种格式等价。

## 回归防护

- [ ] **T004 金标准回归脚本**
  - fixture 驱动：按需站点在线库（或 dump example 输出）vs
    `e3d_golden_reference.json` expected 逐字段断言；E 类按 unset 判定、
    F 类按语义等价豁免。
- [ ] **T006 verify-cata-closure 增加 `--golden <fixture>`**
  - 复用 T008（spec 002）的 HTTP 基准侧通道；fixture 不通过则非零退出（CI 门禁）。

## 端到端收尾

- [ ] **T007 重解析 + 生成不回退验证**
  - 按需站点清库 → 闭包 → manifest 解析 → `--debug-model` 重生成 BRAN + FITT；
  - 断言：A–D 审计归零；BRAN 17/17 与 FITT 的 cata_hash / ptset / trans hash
    与修复前存档一致；`verify-cata-closure --golden` 退出码 0。
- [ ] **T008 文档与提交**
  - pdms-io-fork 修复独立 commit；本仓 spec/工具 commit；
    审计结论回填 spec 002 的 R 残余清单。
