# Feature Specification: CATA 属性解析回归修复 — 字段级保真（spec 003）

## User Need

按需解析（spec 002）落地后，以 8020 老库与 E3D 原生 `Q ATT` 双基准对 BRAN
24381_145018 的生成依赖闭包（151 元素）做字段级审计，发现新版 `parse_pdms_db`
存在**系统性属性字段丢失/截断**（与按需裁剪无关：e2e 全量解析逐字段相同）。
需要以 E3D 原生输出为金标准修复解析器，使按需/全量解析的属性数据字段级保真。

## 背景：审计结论（2026-06-11，三方对比 8020 / e2e / 按需 + E3D Q ATT）

| 类 | 现象 | 规模 | E3D 真值 | 判定 |
|---|---|---|---|---|
| A | LEVE 整型数组截断 `[8,10]→[8]` | 63 处（全部几何基元） | `Level 8 10` | **新解析回归** |
| B | PTCA.PTCD 丢失 | 26 处 | `AXIS -Z` / 裸 `Y` 两种格式 | **新解析回归**（8020 也欠忠实：丢 `AXIS ` 前缀） |
| C | PTCA.PHEI 丢失 | 1 处 | `223` | **新解析回归** |
| D | SDTE.SKEY / RTEX 丢失 | 7 处 | `VGBW` / `( ATTRIB NAMN )` | **新解析回归**（8020 的 RTEX 变形为 `NAMN[500 ]`） |
| E | PSKE/PURP/GTYP/ATTY/LNTP/MTOH/FLOW 等 `0` → 空 | ~110 处 | `unset` | **新解析正确**；8020 把 unset 存 0 是旧错误，行为予以固化 |
| F | 表达式括号与 E3D 原文不一致 | 16 类 | `( ATTRIB PARA[8 ] * 1.2 )` | 三方语义等价；忠实原文为后续项（Non-Goal） |

金标准 fixture：`runtime/bran-closure/e3d_golden_reference.json`（4 个代表 refno
的 E3D Q ATT 原文：13246_243891 / 243926 / 243869 / 243899）。

## 已定位根因（T001 复核）

- **A 类**：`parse_pdms_db/src/parser/attribute/implicit.rs::parse_int_array`
  的计数白名单写的是 `attr_name == "LEVEL" || attr_name == "PTS"`，而字典属性名
  实际为 `LEVE`（4 字符规范名）→ 走 else 分支 `count=1` → 数组截断。
  显式路径（`parse.rs` `DbAttributeType::INTVEC` 分支）按 len 读取、行为正确，
  仅隐式路径受影响。
- **B/C/D 类**：表达式/字符串属性路径（`parse.rs` 的 `EXPR_ATT_SET` 双引擎选优
  与 STRING 显式分支）对这些 hash 的覆盖缺口，待 T002 字节级定位。

## Scope

- `parse_pdms_db`（pdms-io-fork 仓）的隐式/显式属性解析修复（A–D 类）。
- E 类行为（unset 不再存 0）作为正确行为写入回归基线，防止回退。
- 字段级回归校验工具化：金标准 fixture 驱动 + 与 `verify-cata-closure`（T008）
  集成的字段级对比模式。
- 修复后按需站点重解析验证 + BRAN/FITT 生成结果不回退（hash 级）。

## Non-Goals

- F 类表达式文本忠实原文化（语义等价，涉及表达式重建器与求值端回归，单独立项）。
- 8020 老库的数据修复（只作历史对照，不回写）。
- 不跑 `cargo test`/不编译 test 目标（仓库规则）；验证走 CLI + JSON 对比。

## Requirements

1. `parse_int_array` 不得依赖属性名白名单判定数组长度：以属性元数据（attr_info
   长度/类型）或数据头计数为准；至少修正 `LEVE`（含 `LEVEL` 别名）与 `PTS`。
2. PTCD 支持 E3D 两种原生格式：`AXIS -Z`（带前缀）与裸轴 `Y`；解析结果保留完整
   原文（含 `AXIS ` 前缀，比 8020 更忠实）。
3. PTCA.PHEI（字符串数值 `'223'`）、SDTE.SKEY（`VGBW`）、SDTE.RTEX
   （`( ATTRIB NAMN )`）不得丢失；RTEX 不得出现 `NAMN[500 ]` 式下标变形。
4. E 类：E3D `unset` 字段不得写为 `0`；维持现行"空/缺省"行为并纳入回归基线。
5. 金标准回归：4 个代表 refno 的解析输出与
   `runtime/bran-closure/e3d_golden_reference.json` 的 expected 一致
   （A–D 字段必须命中；表达式字段按语义等价判定，原文一致为加分项）。
6. 全量审计回归：BRAN 24381_145018 闭包 151 元素的字段级审计中，A–D 类差异
   归零（对照 8020，扣除 E 类正确差异与 F 类豁免清单）。
7. 生成结果不回退：修复后重解析 + 重生成，BRAN 17/17 实例与 FITT 24381_100063
   的 `cata_hash` / ptset / trans hash 与修复前一致（几何路径不受影响的证明）。

## Open Questions（grill-me 访谈未决项，附推荐答案）

- Q1 范围：是否将 F 类并入本期？**推荐：否**，列后续项（改动面大、收益低）。
- Q2 E 类落库形态：空串 vs 不写字段？**推荐：维持现状（空串/缺省）**，
  改"不写字段"虽更省空间但需全量排查读取端 `get_*` 默认值行为，单独评估。
- Q3 存量站点数据修复策略：**推荐：仅修复解析器 + 新解析自然生效**；
  存量站点（e2e 等）按需重解析，不做后台批量迁移。
- Q4 字段级校验纳入 CI 的口径：**推荐：verify-cata-closure 增加
  `--golden <fixture>` 可选参数**，比对在线库与 fixture，非零退出码门禁。
- Q5 PTCD 的 `AXIS ` 前缀保留是否影响下游消费（ptset dir 推导）？
  **推荐：保留原文 + 下游解析处兼容两种格式**（T005 排查消费点）。

## Acceptance Criteria

- 4 个金标准 refno 解析输出满足 Requirements 5（脚本：fixture 驱动 diff，零 A–D 缺失）。
- BRAN 151 元素审计 A–D 归零（`bran_full_field_audit.py` 升级版输出 PASS）。
- BRAN / FITT 生成 hash 级不回退（Requirements 7）。
- `verify-cata-closure --golden` 在按需站点跑通并以 0 退出。
