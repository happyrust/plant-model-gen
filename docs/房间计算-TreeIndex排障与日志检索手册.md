# 房间计算 TreeIndex 排障与日志检索手册

> 适用范围：`src/fast_model/room_model.rs` 中房间计算的 TreeIndex 查询路径。  
> 目标：当房间计算失败时，能通过固定日志标签快速定位问题、判断责任边界，并给出最短排查路径。

---

## 1. 背景

房间计算中的“房间 -> 面板”映射，当前优先走 TreeIndex，而不是旧的嵌套 SurrealQL。

核心路径：

1. 查询候选房间
2. 将候选房间按 `dbnum` 分组
3. 加载对应 `output/<project>/scene_tree/{dbnum}.tree`
4. 在已加载的 TreeIndex 中查询房间的 `PANE` 子孙

因此，一旦 TreeIndex、`db_meta_info.json`、项目目录配置或 `.tree` 内容异常，房间计算会直接失败。

---

## 2. 固定日志标签一览

当前房间计算的 TreeIndex 失败路径，统一输出以下 4 类标签：

| 标签 | 含义 | 典型原因 |
|------|------|----------|
| `[ROOM_TREE_INDEX_DBNUM_RESOLVE_FAILED]` | 无法从 refno 解析出 dbnum | `db_meta_info.json` 缺失、配置目录错误、refno 不在缓存中 |
| `[ROOM_TREE_INDEX_LOAD_FAILED]` | `.tree` 文件加载失败 | `.tree` 缺失、损坏、目录指向错误、项目切换后路径错位 |
| `[ROOM_TREE_INDEX_ROOM_MISSING]` | `.tree` 已加载，但里面没有目标房间节点 | tree 数据过旧、parse-db 未覆盖该数据库、项目数据与 tree 不一致 |
| `[ROOM_TREE_INDEX_QUERY_FAILED]` | 按 dbnum 批量查询房间面板阶段失败 | 上面任一错误被包装后向上抛出 |

---

## 3. 日志检索方法

### 3.1 本地日志检索

```bash
rg -n "ROOM_TREE_INDEX_" .
```

如果是服务日志文件：

```bash
rg -n "ROOM_TREE_INDEX_" /path/to/logs
```

### 3.2 按具体标签检索

```bash
rg -n "\\[ROOM_TREE_INDEX_DBNUM_RESOLVE_FAILED\\]" /path/to/logs
rg -n "\\[ROOM_TREE_INDEX_LOAD_FAILED\\]" /path/to/logs
rg -n "\\[ROOM_TREE_INDEX_ROOM_MISSING\\]" /path/to/logs
rg -n "\\[ROOM_TREE_INDEX_QUERY_FAILED\\]" /path/to/logs
```

### 3.3 关键信息字段

每条诊断消息会尽量带出以下字段：

- `dbnum`
- `room_refno`
- `room_num`
- `tree_dir`
- `tree_file`
- `error`

因此排查时建议优先按：

1. `room_refno`
2. `room_num`
3. `dbnum`
4. `tree_file`

来反向定位。

---

## 4. 各标签排查手册

## 4.1 `[ROOM_TREE_INDEX_DBNUM_RESOLVE_FAILED]`

### 现象

无法从房间 `refno` 推导出对应 `dbnum`，房间计算在分组阶段失败。

### 优先检查

1. `db_meta_info.json` 是否存在
2. 当前运行目录是否指向了正确项目
3. `DB_OPTION_FILE` 是否切到了别的项目配置
4. 该 `room_refno` 是否真的属于当前项目数据

### 建议命令

```bash
ls output/*/scene_tree/db_meta_info.json
```

如果知道项目名，例如 `AvevaMarineSample`：

```bash
ls output/AvevaMarineSample/scene_tree/db_meta_info.json
```

如怀疑 tree / meta 未生成：

```bash
cargo run --bin aios-database -- --parse-db
```

### 常见根因

- 新环境只同步了数据库，没同步 `scene_tree`
- 切换项目后仍复用了旧的 `output/<project>/scene_tree`
- 当前 `DbOption.toml` 指向的项目名不对

---

## 4.2 `[ROOM_TREE_INDEX_LOAD_FAILED]`

### 现象

已经拿到了 `dbnum`，但加载 `output/<project>/scene_tree/{dbnum}.tree` 失败。

### 优先检查

1. `tree_file` 是否真实存在
2. 文件大小是否异常（0 字节或明显过小）
3. `tree_dir` 是否指向当前项目
4. `.tree` 是否为旧数据、损坏数据

### 建议命令

```bash
ls -lh output/<project>/scene_tree/
```

如果日志里已给出具体 `tree_file`：

```bash
ls -lh output/<project>/scene_tree/7997.tree
```

重新生成：

```bash
cargo run --bin aios-database -- --parse-db
```

### 常见根因

- `.tree` 文件缺失
- parse-db 没跑完或产物不完整
- 部署时只同步了二进制，没有同步 `output/<project>/scene_tree`

---

## 4.3 `[ROOM_TREE_INDEX_ROOM_MISSING]`

### 现象

`.tree` 文件能加载，但 `index.contains_refno(room_refno)` 为假，说明该房间节点不在当前 tree 中。

### 优先检查

1. 该房间是否属于当前 `dbnum`
2. parse-db 产出的 tree 是否是旧版本
3. 项目数据与 tree 数据是否来自不同时间点
4. 该房间是否在数据源中被删除/迁移

### 建议命令

先确认 tree 是否最新：

```bash
cargo run --bin aios-database -- --parse-db
```

再结合房间计算命令复现：

```bash
cargo run --release --bin aios-database -- room compute
```

如果只想缩小范围：

```bash
cargo run --release --bin aios-database -- room compute --room-keyword "-RM"
```

### 常见根因

- tree 与数据库不同步
- 项目切换后指向了错误的 `scene_tree`
- 线上增量更新了数据库，但没重建 tree

---

## 4.4 `[ROOM_TREE_INDEX_QUERY_FAILED]`

### 现象

按 `dbnum` 批量查询房间面板阶段失败。这个标签本质上是上层包装错误，通常不是根因本身。

### 建议动作

1. 继续向下找同一时间窗口内更早出现的 3 类底层标签
2. 优先看同一个 `dbnum`、`room_refno`、`room_num` 的错误
3. 如果只看到这一条，没有更细日志，说明错误在更底层被吞掉或日志采集不完整

---

## 5. 标准排查流程

建议按下面顺序排查，不要一开始就盲目重跑：

1. **看标签**
   - 先确认是 `DBNUM_RESOLVE_FAILED`、`LOAD_FAILED`、`ROOM_MISSING` 还是 `QUERY_FAILED`
2. **看路径**
   - 核对 `tree_dir` / `tree_file`
3. **看项目**
   - 核对 `DbOption.toml` / `DB_OPTION_FILE` / `project_name`
4. **看产物**
   - 核对 `output/<project>/scene_tree/`
5. **必要时重建**
   - 执行 `cargo run --bin aios-database -- --parse-db`
6. **再复现**
   - 重新执行房间计算命令

---

## 6. 推荐复现命令

### 6.1 全量房间计算

```bash
cargo run --release --bin aios-database -- room compute
```

### 6.2 指定关键词缩小范围

```bash
cargo run --release --bin aios-database -- room compute --room-keyword "-RM"
```

### 6.3 先重建 TreeIndex 再复现

```bash
cargo run --bin aios-database -- --parse-db
cargo run --release --bin aios-database -- room compute --room-keyword "-RM"
```

---

## 7. 运维建议

- **部署时**：不要只发二进制，`output/<project>/scene_tree` 也要视为运行时关键产物
- **切项目时**：优先确认 `DbOption.toml` 的 `project_name`
- **数据库更新后**：若层级结构有变化，建议重跑 `parse-db`
- **日志平台中**：建议为 `ROOM_TREE_INDEX_*` 标签建立固定检索视图

---

## 8. 关联文档

- `/Volumes/DPC/work/plant-code/plant-model-gen/ROOM_COMPUTE_OPTIMIZATION.md`
- `/Volumes/DPC/work/plant-code/plant-model-gen/docs/房间计算流程分析.md`
- `/Volumes/DPC/work/plant-code/plant-model-gen/docs/DEBUG_24381_145018_ANALYSIS.md`
- `/Volumes/DPC/work/plant-code/plant-model-gen/docs/DEBUG_24381_145019_PARQUET.md`

