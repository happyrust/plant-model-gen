# NXTR 24381/35961 SurrealDB 查询分析指南

用于排查 NXTR 查不到 LOOP 的原因：启动数据库后用 Surreal CLI 直接查询并分析层级结构。

## ✅ 实际查询结果摘要（已确认）

| 查询 | 结果 |
|------|------|
| NXTR 自身 | `children: [pe:\`24381_35962\`]`，`owner: pe:\`24381_35960\``（PANE），`refno.HEIG=6130` |
| pe_owner 子节点 | NXTR 有 LOOP（24381_35962） |
| geom.rs 同款（LOOP→PAVE/PONT） | `positions: [], frads: []` **为空** |
| LOOP 24381_35962 子节点 | **VERT x4**（24381_35963～66），**不是 PAVE/PONT** |
| **根因** | 该 LOOP 的顶点在 **VERT** 子节点，geom.rs 只查 PAVE/PONT，导致取不到顶点 |
| VERT refno | `pos: [0, 0, 6130]`, `frad: 0` ✅ 顶点数据存在于 VERT |

## 1. 启动 SurrealDB

### 方式 A：文件模式启动（推荐）

```powershell
cd D:\work\plant-code\plant-model-gen

# 数据库路径见 db_options/DbOption.toml [web_server].surreal_data_path
surreal start --bind 0.0.0.0:8020 file:D:/backup-dbs/ams-8020.db
```

### 方式 B：若已通过 web_server 自动启动

若 `auto_start_surreal = true`，启动 web 服务时会自动拉起 SurrealDB，可不单独执行 `surreal start`。

---

## 2. 连接并执行查询

**连接参数**（来自 DbOption.toml）：
- Endpoint: `ws://127.0.0.1:8020`
- 用户: `root` / 密码: `root`
- Namespace: `1516`
- Database: `AvevaMarineSample`

### 2.1 使用 Surreal SQL 命令（交互式）

```powershell
surreal sql --endpoint ws://127.0.0.1:8020 --user root --pass root --ns 1516 --db AvevaMarineSample
```

连接成功后，在 `sql>` 提示符下依次执行以下查询。

### 2.2 关键查询语句

#### ① NXTR 自身记录

```sql
SELECT * FROM ONLY pe:⟨24381_35961⟩;
```

#### ② 通过 pe_owner 查 NXTR 的直接子节点（标准层级）

```sql
SELECT id, noun, name, deleted FROM pe:⟨24381_35961⟩<-pe_owner.in;
```

#### ③ 与 geom.rs 相同的 children 查询（LOOP/PLOO）

```sql
SELECT
    children[WHERE noun IN ["PAVE", "PONT"]].refno.POS as positions,
    children[WHERE noun IN ["PAVE", "PONT"]].refno.FRAD as frads,
    refno.HEIG as height
FROM pe:⟨24381_35961⟩.children WHERE noun IN ["LOOP", "PLOO"];
```

#### ④ 直接 children 子节点（PAVE/PONT 兜底结构）

```sql
SELECT
    children[WHERE noun IN ["PAVE", "PONT"]].refno.POS as positions,
    children[WHERE noun IN ["PAVE", "PONT"]].refno.FRAD as frads
FROM pe:⟨24381_35961⟩;
```

#### ⑤ NXTR 的 HEIG、noun、owner

```sql
SELECT refno.HEIG as height, noun as noun, record::id(owner) as owner
FROM pe:⟨24381_35961⟩;
```

#### ⑥ 对比 PANE 24381/35960 的子节点结构

```sql
SELECT id, noun, name FROM pe:⟨24381_35960⟩<-pe_owner.in;
SELECT id, noun, name FROM pe:⟨24381_35960⟩.children WHERE noun IN ["LOOP", "PLOO"];
```

---

## 3. 预期与排查要点

| 查询 | 预期 | 若不符合则说明 |
|------|------|----------------|
| ② pe_owner 子节点 | 应包含 LOOP/PLOO/PAVE 等 | pe_owner 未建立或方向错误 |
| ③ .children + LOOP/PLOO | 应返回至少 1 条 | `.children` 与 pe_owner 不一致，或 children 未正确填充 |
| ⑤ HEIG | 应约 6130 | 属性读取路径有问题 |

**若 ② 有 LOOP 但 ③ 的 positions/frads 为空**：说明 LOOP 下的顶点在 **VERT** 而非 PAVE/PONT，需在 geom.rs 中扩展对 VERT 的支持。

### 修复方向

在 `fetch_loops_and_height` 中，当 `children[WHERE noun IN ["PAVE", "PONT"]]` 为空时，增加对 **VERT** 的查询：

```sql
children[WHERE noun IN ["PAVE", "PONT", "VERT"]].refno.POS as positions,
children[WHERE noun IN ["PAVE", "PONT", "VERT"]].refno.FRAD as frads,
```

---

## 4. 批量执行（管道）

将上述 SQL 保存为 `docs/queries_nxtr.surql`，然后通过管道执行：

```powershell
cd D:\work\plant-code\plant-model-gen
Get-Content docs\queries_nxtr.surql | surreal sql -e ws://127.0.0.1:8020 -u root -p root -ns 1516 -db AvevaMarineSample
```
