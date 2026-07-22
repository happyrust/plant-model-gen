# 审计报告：8020 主库 dbnum=1112 的 pe_owner 被 run2 误写状态

- 审计时间：2026-07-20 09:12 ~ 09:35 (UTC+8)
- 审计对象：`ws://127.0.0.1:8020` → `rocksdb://D:/backup-dbs/ams-8020.db`，ns=`1516`，db=`AvevaMarineSample`
- 审计方式：HTTP `POST http://127.0.0.1:8020/sql`（Basic root:root，header `surreal-ns:1516` `surreal-db:AvevaMarineSample`，`Content-Type: text/plain`）
- 性质：**只读审计**。全程未对 8020 写入任何数据（原任务允许的"删除错误 meta"未触发，见 §2a）。8033（PID 18280）全程未触碰。

## 结论（TL;DR）

**8020 的 dbnum=1112 pe_owner 边已被旧版 CLI 的 rebuild 尝试大面积破坏，需要用新二进制对 8020 重跑一次完整 `rebuild-pe-owner --dbnum 1112` 修复。**

- 1112 共 72,040 个有 children 的 owner，其中 **53,192 个（73.8%）边与 `pe.children` 不一致**：
  - 8,060 个 owner 边被删光（含 run2 报错现场 `pe:17496_117970`，"已删未插"半成品实锤）；
  - 45,132 个 owner 只残留 children 的**真子集**边——其中 44,507 个只剩一条幽灵边 `in=pe:0_0`（id=`[owner, 0]`），另 ~625 个残留 1~37 条不等的部分边。
- 没有发现"多余/外来"边（无 superset/disjoint 型污染；无 childless-owner 挂边）。错误全部是"缺边"方向。
- `pe_owner_version_meta` 表整个不存在 → **没有错误的可信分界标记**，无需删除；versioned 回退语义安全（该库本来也是非 versioned 启动）。
- 对照组 dbnum 6890 / 5052 完全没有"子集残留"型损伤（不一致全部是历史上从未建过边的 owner，凡有边的 owner 与 children 精确一致）→ 损伤模式为 1112 独有，与 run2（及 ~01:56 的另一次失败尝试）的目标 dbnum 精确吻合。
- 08:26 的手工重试确认未写入：RocksDB LOG 轮转只显示 ~00:10、~02:34（run2）两次嵌入式打开与 08:41:10 的 server 启动，08:26 无打开痕迹。
- 09:02 的 run4 rebuild 走的是 `--config db_options/DbOption-t012-e2e` → `ws://127.0.0.1:8033`（fixture），**没有**再碰 8020（见 §5）。

## 1. 环境检查

```powershell
Get-NetTCPConnection -LocalPort 8020
# 127.0.0.1:8020 Listen, OwningProcess=69092
Get-CimInstance Win32_Process -Filter "ProcessId = 69092"
# surreal.exe: "surreal" start --user root --pass root --bind 127.0.0.1:8020 --log warn rocksdb://D:/backup-dbs/ams-8020.db
# CreationDate: 2026/7/20 8:41:10
```

- 审计开始时 8020 **已在监听**（派单文案写的"当前实测未监听"已过时）：PID 69092 于 08:41:10 由他人拉起（非本次审计拉起）。按任务约束，**审计结束后不停掉该进程**。
- 8033 仍为 PID 18280 监听，未触碰。
- 注意该 server 以**非 versioned** 方式启动（URI 无 `versioned=true`）。

RocksDB 目录打开痕迹（`D:/backup-dbs/ams-8020.db`）：

| 证据 | 时间 | 含义 |
|---|---|---|
| `LOG.old.1784486041449571`（末次写 00:10:47） | ~00:10 | 一次嵌入式打开会话结束于 00:10 |
| `LOG.old.1784508070897606`（末次写 02:34:01） | ~02:33–02:34 | **run2 会话**（与 run2 日志时间 02:33:32 吻合） |
| 当前 `LOCK`/`LOG`/`OPTIONS-014826`（08:41:10-11） | 08:41 | 当前 server 启动 |
| 08:26 前后 | 无任何打开痕迹 | **08:26 手工重试没有打开库、没有写入**（输出只到连接横幅即中止，与之吻合） |

## 2. 审计查询与结果

### 2a. pe_owner_version_meta（期望为空）

```sql
SELECT * FROM pe_owner_version_meta;
-- ERR: The table 'pe_owner_version_meta' does not exist
```

表不存在 → run2 确实死在 meta 写入之前；**没有错误可信标记需要删除**（原任务授权的唯一写动作未触发，本审计零写入）。

### 2b. 焦点 owner `pe:17496_117970`（run2 报错现场）

run2 报错：`Database index 'unique_pe_owner' already contains [pe:'17496_117971', pe:'17496_117970'], with record pe_owner:[pe:'17496_117970', 0]`（`db-data/tree_version_smoke_run2.log`）。

```sql
SELECT VALUE children FROM pe:`17496_117970`;
-- [["pe:`17496_117971`"]]   （1 个 child）
SELECT VALUE in FROM pe:`17496_117970`<-pe_owner ORDER BY id;
-- []                        （0 条边）
```

**不一致确认**：children=1、边=0，即"已删未插"半成品残留。该 pe 记录本身完好（noun=NXTR, dbnum=1112, owner=pe:`17496_39512`）。

### 2c. 全量一致性审计（dbnum=1112，比抽样 20 个更强）

索引定义：`DEFINE INDEX unique_pe_owner ON pe_owner FIELDS in, out UNIQUE`。
dbnum=1112 共 225,813 条 pe（含 `17496_*` 与 `25688_*` 两个 ref0 前缀，后者 54,281 条，均属 1112，已全部覆盖）。

```sql
SELECT count() FROM pe WHERE dbnum = 1112 AND children != [] GROUP ALL;                       -- 72040
RETURN math::sum((SELECT VALUE array::len(children ?? []) FROM pe WHERE dbnum = 1112));       -- 270253
RETURN math::sum((SELECT VALUE count(<-pe_owner) FROM pe WHERE dbnum = 1112));                -- 101975
SELECT count() FROM pe WHERE dbnum = 1112
  AND array::sort(children ?? []) != array::sort(<-pe_owner.in) GROUP ALL;                    -- 53192（集合级不一致）
-- 错型分解：
SELECT count() FROM pe WHERE dbnum = 1112 AND children != [] AND count(<-pe_owner) = 0 GROUP ALL;   -- 8060（零边）
SELECT count() FROM pe WHERE dbnum = 1112 AND children != [] AND count(<-pe_owner) > 0
  AND array::sort(children ?? []) != array::sort(<-pe_owner.in) GROUP ALL;                          -- 45132（有边但失配）
SELECT count() FROM pe WHERE dbnum = 1112 AND (children ?? []) = [] AND count(<-pe_owner) > 0 GROUP ALL;  -- 0（无"幽灵挂边"）
-- 45132 个失配 owner 全部是"边 ⊊ children"真子集（无外来边）：
SELECT count() ... AND (<-pe_owner.in ALLINSIDE (children ?? [])) GROUP ALL;                        -- 45132
-- 失配 owner 的残边数分布：ne=1 → 44983 个；ne=2..37 → 共 149 个
-- 其中"只剩一条 in=pe:`0_0` 幽灵边"的：
SELECT count() FROM pe WHERE dbnum = 1112 AND children != [] AND count(<-pe_owner) = 1
  AND <-pe_owner.in = [pe:`0_0`] AND array::sort(children ?? []) != array::sort(<-pe_owner.in) GROUP ALL;  -- 44507
```

**dbnum=1112 汇总**：

| 指标 | 数值 |
|---|---|
| 有 children 的 owner | 72,040 |
| 与 children 完全一致 | 18,848（26.2%） |
| **不一致合计** | **53,192（73.8%）** |
| ├─ 零边（已删未插/从未建） | 8,060 |
| ├─ 真子集残留（缺边） | 45,132 |
| │   └─ 其中只剩 `pe:0_0` 幽灵边 | 44,507 |
| 外来边 / childless 挂边 | 0 |
| Σ len(children) | 270,253 |
| Σ 实际入边 | 101,975（覆盖率 37.7%） |

抽样核对（c 项，确定性分页 START 30000 取 20 个有 children 的 owner）：20/20 不一致，均为 ne=1 幽灵边型——与全量统计吻合（全量数字为准，抽样仅作交叉验证）。失败批次文件中的 owner 逐个核对同样吻合：`pe:25688_22264` / `pe:17496_228657` 只剩 `0_0` 边，`pe:17496_375549`（children 无 0_0）零边。

幽灵子说明：`pe:0_0` 作为记录**不存在**，但出现在 1112 的 45,141 个 owner 的 `children` 数组里（悬空哨兵引用）。旧 CLI 会为它插边（见 `db-data/rebuild_pe_owner_failed_batch.sql`，66 条 `INSERT RELATION`，几乎每条第 0 行都是 `in: pe:`0_0``）；新 CLI 将其按 ghost 处理（run4 fixture 输出 `ghost_edges_deleted: 39`）。

### 2d. 总量对照（d 项）

```sql
SELECT count() FROM pe_owner GROUP ALL;   -- 4,963,050（全库所有 dbnum）
```

1112 的边量级明显亏缺（101,975 vs children 期望 270,253）。全库 4.96M 条边分布在几十个 dbnum 上（GROUP BY dbnum 全表已跑，1112 pe 记录 225,813 条）。

### 2e. 对照组（基线模式，未被 run2 触碰的 dbnum）

| dbnum | 有 children 的 owner | 集合不一致 | 其中零边型 | 有边但失配型 | Σ children | Σ 边 |
|---|---|---|---|---|---|---|
| 6890 | 36,686 | 17,566 | 17,566 | **0** | 274,025 | 136,374 |
| 5052 | 53,017 | 25,149 | 25,149 | **0** | （未取） | （未取） |

对照组的"不一致"**全部**是"从未建过边"（历史覆盖不全，凡有边的 owner 与 children 精确一致）。**"真子集/0_0 残留"型损伤只在 1112 出现** → 定性为 run2（老语义 rebuild）损伤，而非本库固有状态。附带发现：8020 各 dbnum 的 pe_owner 基线覆盖本来就只有约一半 owner（历史遗留，与本事故无关，不需要本次处置）。

## 3. 损伤机理（与现场证据吻合的解释）

- 旧版 CLI 的 rebuild 语义（specs/023 research C6/C7 时代）：全 owner 先删后插、`INSERT RELATION` 携带显式 id `[owner, i]`、children 含 `0_0` 也照插。
- 观察到的三态（一致 18,848 / 零边 8,060 / 子集残留 45,132）与"删除与插入分批提交、插入批因 `unique_pe_owner (in,out)` 撞已存在旧边而中断"的执行序一致；`db-data/rebuild_pe_owner_failed_batch.sql`（01:56:43 落盘，66 条语句）证明 02:33 的 run2 之前**还有一次**同样打到 8020 的失败尝试——两次尝试共同造成现状。
- 撞索引的直接原因：插入的 `(in,out)` 对与**尚未被删除的旧边**重复（`pe_owner:[pe:`17496_117970`, 0]` 现场即是）。

## 4. run2 / 08:26 / run4 时间线

| 时间 | 事件 | 对 8020 的影响 |
|---|---|---|
| ~00:10 | 一次嵌入式打开（run1 时段；run1 日志因脚本 PS 报错未跑到 rebuild） | 不明确，无损伤证据 |
| ~01:56 | 一次 rebuild 失败尝试（`rebuild_pe_owner_failed_batch.sql` 落盘） | **写入损伤** |
| 02:33–02:34 | run2：`rebuild-pe-owner --dbnum 1112` 撞 `unique_pe_owner` 退出（`tree_version_smoke_run2.log`） | **写入损伤** |
| 08:26 | 手工重试，输出只到连接横幅 | **零写入**（无库打开痕迹） |
| 08:41:10 | 他人以 server 模式拉起 8020（PID 69092，本审计沿用、不停） | — |
| 08:41 | run3（修复后脚本）日志截断于 rebuild 前 | 无 8020 写入证据 |
| 09:02 | run4 + `tree_version_smoke_rebuild.out.txt`：`--config db_options/DbOption-t012-e2e` → **ws://127.0.0.1:8033**（fixture），exit 0，edges_inserted=191 | **未触碰 8020** |

## 5. 修复建议（只列清单，动作由总指挥排期）

1. **需要修复**：用新二进制对 8020 跑一次完整 `model-version rebuild-pe-owner --dbnum 1112 --config db_options/DbOption`（等 build-bins 完成、避开 run-smoke3 资源窗口）。新语义（每 owner 先删后插 + ghost 跳过 + 撞 id 报错）预计将 53,192 个不一致 owner 全部归位，并清掉 44,507 条 `0_0` 幽灵边。
2. 修复后复核（本报告 §2c 的四条聚合 SQL 重跑一遍即可，预期：不一致=0、Σ边≈Σchildren−ghost 数）。
3. 决策点：新 rebuild 会写 `pe_owner_version_meta:1112`（`meta_source=rebuild_cli`）。8020 是**非 versioned** 主库，该 meta 只是无害标记，但要不要留由总指挥定（不留可 rebuild 后手工删）。
4. 无需处置：对照组暴露的"历史覆盖不全"（约半数 owner 从未建边）是本库长期状态，与本事故无关；若日后要求 8020 全 dbnum 树查询走 pe_owner，需另立全量重建任务。
5. 8020 server（PID 69092）为他人拉起，本审计**未停**；8033（PID 18280）未触碰。
