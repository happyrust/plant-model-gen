# Plan: CATA 属性解析回归修复（spec 003）

## 证据链与根因

### A — LEVE 数组截断（已定位，可直接修）

`pdms-io-fork/crates/parse_pdms_db/src/parser/attribute/implicit.rs::parse_int_array`：

```rust
let count = if attr_name == "LEVEL" || attr_name == "PTS" {
    i32::from_be_bytes(...) as usize   // 读真实计数
} else {
    1                                   // ← 其余整型数组只取首元素
};
```

字典规范名是 `LEVE`（非 `LEVEL`）→ 进 else → `[8,10]` 截成 `[8]`。
显式路径（`parse.rs` `DbAttributeType::INTVEC`：`be_u32(len)` 后循环读）行为正确，
说明同一属性在不同元素上可能走两条路径，仅隐式路径截断 —— 与审计观察一致
（63 处全部来自 GMSE 子树基元，这批正是隐式属性页）。

修法（按优先级）：
1. 首选：取消白名单 —— 从 `attr_info`（字典元数据）拿数组长度或一律读数据头
   计数；需核对隐式区 INTVEC 的编码是否总带计数字（对照 8020 正确产物 + db 文件
   字节）。
2. 保底：白名单补 `LEVE`（与 `LEVEL` 并列）；同时排查同模式的其它整型数组属性
   （JUST/LSTA 等，扫字典里 INTVEC 类型属性全集，T003）。

### B/C/D — 表达式/字符串字段丢失（待字节级定位）

入口：`parse.rs` 显式属性循环。
- `EXPR_ATT_SET`（rs-core consts）成员走双引擎（新 `parse_explicit_entry_expression_value_with_type`
  + legacy `parse_legacy_explicit_entry_expression_value`）按 score 选优；
  两边都解不出 → `att_value=None` → **字段静默丢失**（当前无 miss 统计）。
- PTCD 有专属 dual-parse 分支（`prefer_dual_parse`），但 E3D 的两种原文
  （`AXIS -Z` / 裸 `Y`）在新引擎 `parser/attribute/expression.rs::显式轴向字符串`
  的覆盖待验证；裸轴格式疑似两边都不认。
- PHEI='223'（纯数字字符串）：score 函数给纯数值 **-5 分**（"更可能是误解析"）——
  若 legacy 解出 '223' 但被降权丢弃，则是选优策略误杀（T002 验证）。
- SKEY/RTEX（SDTE 表）：SKEY 可能不在 `EXPR_ATT_SET`，走 STRING 显式分支；
  丢失点可能在类型码映射或 SDTE 类型布局（T002 用字节 dump 确认）。

定位手段（不跑 cargo test）：`parse_pdms_db` 加 `--features debug_parse` 的
单元素 dump example（输入 ams5054_0001 + refno），对照 8020 值与 E3D 原文逐属性
比对，输出三列报告。

### E — unset 不存 0（正确行为，防回退）

E3D `Q ATT` 证实 PSKE/PURP 等为 `unset`；8020 旧解析把 unset 物化为 `0`。
新行为更正确 —— 把这批字段写入金标准 fixture 的 expected=unset，防止未来
"对齐旧库"式的错误修复。

## 修复顺序（依赖关系）

```
T001 LEVE 修复（独立，已定位）──────────────┐
T002 B/C/D 字节级定位（dump example）        ├─→ T004 金标准回归脚本
T003 INTVEC 全字典扫描（A 的同类隐患）───────┘      ↓
T005 PTCD 消费端兼容排查（AXIS 前缀）          T006 verify-cata-closure --golden
                                                    ↓
                                  T007 端到端重解析 + 生成不回退验证
```

## 风险

- R1 隐式 INTVEC 计数编码假设错误 → 用 8020 正确产物 + 原始 db 字节双重校验。
- R2 PTCD 保留 `AXIS ` 前缀可能影响 ptset dir 推导 → T005 全量排查消费点
  （rs-core `resolve_axis_params` / expression 解析），兼容两种格式。
- R3 表达式 score 选优调整可能影响其它字段 → 金标准 fixture + BRAN 151 元素
  审计兜底；改动最小化（仅放宽纯数值降权对已知字段的误杀）。
- R4 pdms-io-fork 为共享依赖（path patch），主仓与 worktree 同时受影响 →
  修复后两边均需 cargo check；提交单独成 commit 便于回滚。

## 验证（CLI + JSON，不用 cargo test）

1. dump example 对 4 个金标准 refno 输出 → 与 fixture diff（脚本断言）。
2. 按需站点清库重解析 → `bran_full_field_audit.py`（升级：豁免 E/F 类）PASS。
3. `--debug-model` 重生成 BRAN + FITT → cata_hash/ptset/trans 与本轮存档一致。
4. `verify-cata-closure --golden runtime/bran-closure/e3d_golden_reference.json`
   退出码 0。
