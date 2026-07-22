# core.dll 中「NOUN 的 ATT 修改是否需要重算模型」的判定机制

> 逆向对象：`D:\AVEVA\Everything3D3.1\core.dll`（AVEVA Everything3D 3.1 的 DABACON 数据库层）
> 分析工具：IDA Professional 9.2 + `ida-pro-mcp`（headless idalib，会话 `core31-readonly`）
> 模块信息：32 位 PE，imagebase `0x5170000`，image_size `0x4113000`，MD5 `52a236db644b0e11950c9cda7b93dd34`，
> 34,308 个函数 / 52,255 条字符串；本文所有地址均为 IDA 内已重定基地址。
> 结论与代码片段均来自反编译（Hex-Rays）实证，域含义处会标注「PDMS 语义」。

---

## 0. 结论速览（TL;DR）

core.dll 判定「一次属性修改要不要触发模型/图形重算」**不是**用一张写死的 `if noun==xxx && att==yyy` 表，而是一套**由数据字典（DDL/Dabacon Dictionary）驱动的、声明式的、事件门控**的机制。它分三层：

1. **NOUN 门（元素类型是否有几何）**：字典字段 `DB_Noun::geomset()`、`DB_Noun::graphicsBehaviour()`、`DB_Noun::extrusion()`。只有「几何集」类型的元素才可能产生/影响 3D 模型。
2. **ATT 门（该属性改动是否"有意义"）**：字典标志 `DB_Attribute::wnoevt()`（will-no-event，无事件）是**主开关**——置位的属性改了**不发任何事件、不记变更、不触发重算**；辅以 `wnoclm`（无需 claim）、`change`（可变）、`isPseudo`（伪/派生属性）。
3. **分发 + 变更日志**：通过门控后，`DB_ElementChangesPlugger::PostSetAttribute` 向「全局订阅者」和「按属性订阅者」广播，并写入 `DB_UserChanges` 变更日志；下游（其它模块的图形/绘图子系统）据此重算几何。增量范围由 `DB_DB::elementsChangedSince/Between`（按 session 号）给出。

一句话：**「NOUN 有没有几何」由 noun 的 `geomset`/`graphicsBehaviour` 决定；「这次 ATT 改动要不要往下游走」由 att 的 `wnoevt` 决定；决定往下走后，靠发布/订阅 + `DB_UserChanges` 把"谁变了"交给几何子系统去重算。**

> 真正的几何重建不在 core.dll，而在消费方 **`Core3D.dll`**（`DESDRA_SCPlugs` / `PartialUpdateDesiMgr` / `DES_DrawListManager` / `GFX_GraphicsManager`）——完整消费链见 §11。「是否重算」= `wnoevt`×`geomset`；「重算多大范围」= `SignificantOwner`/`XGEOM` 粒度。

---

## 1. 写属性的调用链（Write Path）

```
DB_Element::putAtt() / putAttSegment()                （公开 API）
        │
        ▼
DB_Element::internalPutAtt(attr, qual, value, doRules) （按值类型分多个重载：int/double/string/各种 vector…）
        │  写入属性值后
        ▼
DB_Element::postSetAttribute(attr, qual, doRules)      @ 0x59453b0   ← 核心
        │
        ▼
DB_ElementChangesPlugger::Instance()->PostSetAttribute(el, attr, qual)  @ 0x591e5b0  ← 门控 + 分发
```

`DB_Element::postSetAttribute` 的调用者（`xrefs_to 0x59453b0`）证实了上面的入口——全部是各类型的 `internalPutAtt` 重载与 `putAtt/putAttSegment`：

- `DB_Element::internalPutAtt(...)` 多个重载：`0x593f220`、`0x593f680`、`0x5940190`、`0x5940480`、`0x59410c0`、`0x59415a0`、`0x59419d0`、`0x5941ed0`、`0x5942370` …
- `DB_Element::putAtt(...,DB_Expression&)` `0x5947c60`
- `DB_Element::putAttSegment(...)` `0x59490c0`

---

## 2. 核心决策函数 `DB_Element::postSetAttribute` @ `0x59453b0`

反编译（节选、已加注释）：

```c
char DB_Element::postSetAttribute(DB_Element *this, const DB_Attribute *a2,
                                  const DB_Qualifier *a3, bool a4 /*doRules*/)
{
    // sub_5992C80 收集"受此次改动影响的元素集合" [v28, v29)（本体 + 关联体）
    sub_5992C80(&v28, this, a2, 0);
    ...
    for (每个受影响元素 v5) {
        // (1) 触发属性变更事件（见 §3）
        DB_ElementChangesPlugger::Instance()->PostSetAttribute(v5, a2, a3);  // vtbl+52

        // (2) 改的是 NAME 属性 → 额外的名字变更处理
        if (a2 == ATT_NAME)
            DB_ElementChangesPlugger::Instance()->/*vtbl+68*/(v5);

        // (3) 改的是 UDA（用户自定义属性）→ 重算"受控 UDA"依赖
        if (v9 = RTDynamicCast(a2, DB_Attribute -> DB_Uda)) {
            for (每个受控 UDA / 每个被过滤命中的元素 v12) {
                DB_Element::getAtt(v12, controlledUda, ...);           // 取当前值
                if (!controlledUda->/*filter*/(v12, ...)) {           // 过滤不再命中
                    DB_Element::claim(v12, 0);                        // 认领
                    DB_Element::clearUda(v12, controlledUda, 1);      // 清除失效 UDA
                }
            }
        }

        // (4) doRules 时：评估规则，重算规则/派生属性（可级联再次触发 postSetAttribute）
        if (a4 && !DB_Element::evaluateRules(v5, a2))
            break;
    }
    ...
}
```

要点：
- 该函数是「一次属性写入后的中枢」，负责**事件触发**、**NAME 特判**、**UDA 依赖重算**、**规则重算（`evaluateRules`）**。
- 规则/UDA 重算会**级联**产生新的属性改动，从而再次进入本函数——这是 PDMS「改一个参数，一串派生属性跟着变」的实现根源。

---

## 3. 门控 + 分发 `DB_ElementChangesPlugger::PostSetAttribute` @ `0x591e5b0`

反编译（节选、已加注释）：

```c
void DB_ElementChangesPlugger::PostSetAttribute(this, const DB_Element *a2,
                                                const DB_Attribute *a3, const DB_Qualifier *a4)
{
    if ( !DB_Attribute::wnoevt(a3) )          // ★ ATT 门：属性置了 "no-event" 就直接返回，什么都不做
    {
        // (a) 通知"全局订阅者"列表（off 68..69）：handler(element, attr)
        for (each global subscriber v7) (*v7)(v7, a2, a3);

        // (b) 通知"按属性订阅者"：以 attr 指针为 key 的红黑树（off 71）里，key==a3 的 handler
        for (each per-attribute subscriber of a3) (*handler)(handler, a2, a3);

        // (c) 记入用户变更日志（见 §5）
        DB_UserChanges::currentInstance()->attributeModified(a2, a3, a4);
    }
}
```

- `wnoevt` 是**唯一的总闸**：`xrefs_to 0x58d5290` 证实它只被三个"发事件"的分发器检查：
  - `PostSetAttribute`（标量属性）`0x591e5b0`
  - `PostSetRefAttribute`（引用属性）`0x591e720`
  - `PostSetRefListAttribute`（引用列表属性）`0x591e780`
- 两种订阅：
  - 全局订阅 `SubscribePostSetAttribute(handler)` `0x581f730`
  - **按属性订阅** `SubscribePostSetAttribute(const DB_Attribute*, handler)` `0x581f750`（红黑树按属性指针建索引）——几何子系统正是靠它「只订阅影响几何的那些属性」。

---

## 4. 两级门控用到的字典标志

### 4.1 NOUN 级（`DB_Noun`，元素类型是否有几何）

| 方法 | 地址 | 语义（PDMS） | 实现证据 |
|---|---|---|---|
| `geomset()` | `0x58d8a20` | 该 NOUN **拥有几何集（产生图元/图形）** | `internalGetField(this, 859903, &b)` 读字典字段 |
| `extrusion()` | `0x58d8180` | 拉伸几何 | `internalGetField(this, 663225, &b)` |
| `graphicsBehaviour()` | `0x58d9760` | 图形行为分类（int，**DDL 字段 5099119**） | 懒加载后 `return *(this+180)`；由 `DB_Noun::ReadData` 从字典填充 |
| `clasherSection()` / `clasherWithin()` | `0x58d7650` / `0x58d7670` | 参与碰撞检查 | 字典布尔字段 |
| `defaultVolumeQuery()` | `0x58d7840` | 体量查询 | 字典布尔字段 |

`graphicsBehaviour` 反编译：

```c
int DB_Noun::graphicsBehaviour(DB_Noun *this) {
    if (!*((BYTE*)this + 96)) DB_Noun::ReadData(this);  // 懒加载 noun 字典定义
    return *((DWORD*)this + 45);                          // off 180：图形行为分类整数（来自 DDL）
}
```

> 结论：**"这个元素类型会不会产生 3D 几何"是 noun 的字典属性**（`geomset`/`graphicsBehaviour`/`extrusion`），不是硬编码的类型名单。`graphicsBehaviour` 的具体枚举取值本次未逐一提取（属于字典数据），但 `geomset` 是最直接的布尔「有无几何」判据。

### 4.2 ATT 级（`DB_Attribute`，该属性改动是否"有意义"）

`func_query "@DB_Attribute@@QBE_NXZ"` 列出的布尔字典标志（挑选与"是否触发下游"相关者）：

| 方法 | 地址 | 语义（PDMS） | 对"要不要重算"的作用 |
|---|---|---|---|
| **`wnoevt()`** | `0x58d5290` | will-no-event，改动**不发事件**（**DDL 布尔字段 299311034** → off 184） | ★主开关：置位 → 不通知订阅者、不记变更 → **不触发重算** |
| **`wnoclm()`** | `0x58d5270` | will-no-claim，改动**无需 claim 元素**（**DDL 布尔字段 193514909** → off 185） | 影响是否把元素纳入工作集/session 记账（`sub_5991BE0` 的 claim 路径） |
| `change()` | `0x58cf270` | 属性可变 | 字典字段 76272573；不可变属性谈不上"改动触发" |
| `isPseudo()` | `0x58d2570` | 伪/派生属性（type∈{4,5}） | 伪属性不落库，通常由计算得出 |
| `casc()` | `0x58cf160` | cascade 级联到成员 | 改动向下级联的语义 |
| `catparam()`/`catrul()` | `0x58cf230`/`0x58cf250` | 目录参数/目录规则 | 与目录几何生成相关的输入 |
| `desrul()` | `0x58cf750` | 设计规则驱动 | 规则重算输入 |
| `connection()` `tube()` `invis()`/`visible()` `protect()` `defer()` `idref()` `isUDA()` `isTable()` … | — | 连接/管件/可见/保护/延迟/引用/UDA/表 等分类 | 各自语义 |

`wnoevt` / `wnoclm` 反编译（都是懒加载后读单字节标志位）：

```c
bool DB_Attribute::wnoevt(this){ if(!*(BYTE*)(this+8)) (*(vtbl+20))(this); return *(BYTE*)(this+184); }
bool DB_Attribute::wnoclm(this){ if(!*(BYTE*)(this+8)) (*(vtbl+20))(this); return *(BYTE*)(this+185); }
```

`change` / `geomset` / `extrusion` 则走统一的 `internalGetField(fieldId, &out)` 从字典读取，进一步印证**这些都是数据字典字段，而非代码常量**。

---

## 5. 变更累积与消费（谁真正去重算模型）

### 5.1 变更日志 `DB_UserChanges`

`DB_ElementChangesPlugger::PostSetAttribute` 末尾调用
`DB_UserChanges::currentInstance()->attributeModified(el, attr, qual)` @ `0x5987090`
把「某元素的某属性被改」记入当前变更集。`DB_UserChanges` 提供分类视图：

- `ElementsCreated / ElementsDeleted / ElementsModified / ElementsMoved / ElementsReordered / ElementsMemberChanged`
- 逐元素的 `AttributesModified(el, vector<DB_Attribute*>)`、`AttributesQualsModified(...)`
- 判定：`isElementCreated / isElementDeleted / isElementMoved`
- 增量查询：`DB_DB::elementsChangedSince(sesno,...)` `0x5900230`、`DB_DB::elementsChangedBetween(...)` `0x58ffc50`

### 5.2 把变更交给消费者

- **批量插件**：`DB_DBPlugger::PreHandleUserChanges` `0x591b7f0` / `handleUserChanges` `0x591bd20` / `PostHandleUserChanges` `0x591b5c0`——遍历所有注册的 DB 插件，逐个调用其 vtbl `+44/+48/+52`，把整批 `DB_UserChanges` 交给它们：

```c
void DB_DBPlugger::handleUserChanges(this, const DB_UserChanges *a2){
    for (each registered plug v2)                       // 插件表 [this[0], this[1])
        if (*v2) (*(vtbl(*v2)+48))(*v2, a2);            // 把整批变更交给每个插件
}
```

- **按元素订阅**：`DB_StatusEvents::SubscribePostDBChangesEvent(DB_Element&, DB_PostDBChangesHandler*)` `0x599c6e0`（及 `Pre...` `0x599c9a0`、对应 `Unsubscribe`）——消费者可对**指定元素**订阅 DB 变更事件。

> **重要范围界定**：在 core.dll 内搜索 `geometry/rebuildModel/makeGeom/...` 命名的函数**为空**。这说明**实际的几何重算不在 core.dll**（它在设计/绘图/图形模块，如 des/draw/图形引擎）。core.dll 作为 DABACON **数据库层**，只负责：判定属性改动是否有意义（DDL 标志）、广播变更事件、维护变更日志与增量查询。真正"重画/重建几何"的模块以 §5.2 的方式订阅并消费这些变更。字符串侧也印证了图形命令的存在（图形设备命令表里有 "Regenerate picture""Regenerate view""Regenerate picture in psm"），但那是图形设备指令，不是 DB 层逻辑。

---

## 6. 附加的重算/依赖路径（都在 `postSetAttribute` 内）

1. **UDA 受控依赖**：改了某属性后，若它是 `DB_Uda`，会遍历「受控 UDA」，对过滤不再命中的元素 `claim()`+`clearUda()`，保证派生的 UDA 与源属性一致。
2. **规则重算 `DB_Element::evaluateRules`**：`doRules` 打开时对元素评估规则，重算规则/派生属性（`desrul`/`catrul` 类），可**级联**再次触发属性事件。
3. **NAME 特判**：改 `ATT_NAME` 会额外走一条名字变更通知（`plugger vtbl+68`）。
4. **引用类属性**：`PostSetRefAttribute` / `PostSetRefListAttribute` 与标量属性同样受 `wnoevt` 门控——引用（如 SPREF/目录引用、成员引用）改动也走同一套事件体系。

---

## 7. 与本仓库 plant-model-gen（Rust 重写）的对应关系

| core.dll（DABACON） | plant-model-gen（Rust 重写） | 说明 |
|---|---|---|
| `DB_Noun::geomset()` / `graphicsBehaviour()` / `extrusion()` | `scene_tree::is_geo_noun(noun)` | 「NOUN 是否产生几何」的判据。core.dll 读字典 `geomset` 标志；Rust 现用 **noun 名单** 判断 |
| `DB_Attribute::wnoevt()`（属性级门控） | *（目前缺少等价物）* | Rust 侧目前主要按「geo-noun + 变更 refno」判定重算，**尚未按"具体 ATT 是否影响几何"过滤** |
| `DB_UserChanges` + `elementsChangedSince/Between` | `IncrGeoUpdateLog`、`orchestrator::gen_all_geos_data(incr_updates,…)`、`data_interface::increment_manager::IncrementInfo` | 「谁变了」的变更集 / 增量范围 |
| session 号（sesno）增量 | `sesno_version_anchor`（见 `AGENTS.md` specs/022） | 增量起点/锚点 |
| `PartialUpdateDesiMgr` 的 `SignificantOwner`/`XGEOM` 粒度（Core3D，§11.3） | *（可借鉴）* 增量重算的"块"粒度 | 「重算多大范围」：按有意义几何容器为单位重算 |

Rust 侧 `is_geo_noun` 的 noun 名单来源（可与 `geomset` 对照）：

```56:66:src/scene_tree/init.rs
pub(crate) fn is_geo_noun(noun: &str) -> bool {
    let noun_upper = noun.to_uppercase();
    let noun_str = noun_upper.as_str();

    USE_CATE_NOUN_NAMES.contains(&noun_str)
        || GNERAL_LOOP_OWNER_NOUN_NAMES.contains(&noun_str)
        || GNERAL_PRIM_NOUN_NAMES.contains(&noun_str)
        || BRAN_COMPONENT_NOUN_NAMES.contains(&noun_str)
        || noun_str == "BRAN"
        || noun_str == "HANG"
}
```

### 可落地的改进建议

1. **引入"属性显著性"过滤（对齐 `wnoevt`）**：当前增量重算只要命中 geo-noun 且 refno 在变更集，就重算几何；但很多属性改动（描述、备注、纯业务/UDA 文本等）对几何**无影响**。可建一张**属性显著性表**（等价于 `wnoevt=false` 且几何相关的 ATT 集合），只有当变更集中该元素**被改的属性**落在显著集里才重算——可显著减少无谓重算。
2. **noun 判据向字典对齐**：`is_geo_noun` 的名单可与 DDL `geomset` 标志核对，避免名单遗漏/过宽。
3. **级联语义**：注意 core.dll 的规则/UDA 会级联改属性（`evaluateRules`/受控 UDA）。若 Rust 侧从上游 DB 取"已算好的派生属性"值则无需复刻级联；若自行计算，则需要覆盖 `catparam/catrul/desrul` 驱动的派生。

---

## 8. 关键符号 / 地址速查表

| 符号 | 地址 | 作用 |
|---|---|---|
| `DB_Element::postSetAttribute` | `0x59453b0` | 属性写入后的中枢（事件/NAME/UDA/规则） |
| `DB_ElementChangesPlugger::PostSetAttribute(el,attr,qual)` | `0x591e5b0` | 门控（wnoevt）+ 分发 + 记变更 |
| `…PostSetAttribute(el,attr)` | `0x591e530` | 上者的无 qualifier 包装 |
| `…PostSetRefAttribute` / `…PostSetRefListAttribute` | `0x591e720` / `0x591e780` | 引用/引用列表属性（同受 wnoevt 门控） |
| `SubscribePostSetAttribute(handler)` | `0x581f730` | 全局订阅 |
| `SubscribePostSetAttribute(DB_Attribute*,handler)` | `0x581f750` | 按属性订阅（RB 树） |
| `DB_Attribute::wnoevt` | `0x58d5290` | ★属性无事件标志（off 184） |
| `DB_Attribute::wnoclm` | `0x58d5270` | 属性无需 claim（off 185） |
| `DB_Attribute::change` | `0x58cf270` | 可变（DDL 字段 76272573） |
| `DB_Attribute::isPseudo` | `0x58d2570` | 伪/派生属性（type 4/5） |
| `DB_Noun::geomset` | `0x58d8a20` | ★NOUN 有几何集（DDL 字段 859903） |
| `DB_Noun::extrusion` | `0x58d8180` | 拉伸几何（DDL 字段 663225） |
| `DB_Noun::graphicsBehaviour` | `0x58d9760` | 图形行为分类 int（off 180） |
| `DB_UserChanges::attributeModified` | `0x5987090` | 记录属性变更 |
| `DB_UserChanges::AttributesModified` | `0x5986a30` | 逐元素已改属性列表 |
| `DB_DB::elementsChangedSince` / `Between` | `0x5900230` / `0x58ffc50` | 增量变更查询（按 session） |
| `DB_DBPlugger::handleUserChanges` / `Pre` / `Post` | `0x591bd20` / `0x591b7f0` / `0x591b5c0` | 批量把变更交给消费者插件 |
| `DB_StatusEvents::SubscribePostDBChangesEvent` / `Pre` | `0x599c6e0` / `0x599c9a0` | 按元素订阅 DB 变更 |
| `DB_Element::internalPutAtt`（多重载） | `0x593f220` 等 | 属性写入入口 |
| `DB_Noun::ReadData` | `0x58d6d20` | 从字典加载 noun 字段（graphicsBehaviour=5099119 等） |
| `DB_Attribute::ReadData` | `0x58ce1f0` | 从字典加载 att 标志（wnoevt=299311034, wnoclm=193514909） |
| `sub_5992C80` | `0x5992c80` | 收集"受此次改动影响的元素集合"（本体 + 克隆/绑定副本） |
| `DB_Clone::getRelatedElements` | `0x59ac380` | 取克隆副本 / 分布式属性绑定元素 |
| `DB_DBPlugger::instance` | `0x591bd60` | DB 事件中枢单例 |

---

## 9. 复现方式（idapro-mcp）

服务：`idalib-mcp`（ida-pro-mcp v2.0.0，headless）运行于 `127.0.0.1:13338`，会话 `core31-readonly` 已加载 `core.dll.i64`（auto-analysis + Hex-Rays 就绪）。示例调用（Streamable-HTTP / JSON-RPC）：

- 反编译：`decompile { addr: "0x59453b0" }`
- 函数名检索：`func_query { queries:[{ name_regex:"@DB_Attribute@@QBE_NXZ" }] }`
- 字符串检索：`entity_query { queries:[{ kind:"strings", regex:"(?i)regen" }] }`
- 交叉引用：`xrefs_to { addrs:"0x58d5290" }`

> 注：`core.dll.i64` 是打包库，被上述 headless 会话独占（锁住 `.id0/.id1/.nam`）。若要用 IDA GUI 打开同一库，需先让 MCP `idalib_close` 释放会话，或对副本操作。

---

## 10. 补充发现（第二轮深挖）

### 10.1 DDL 字段号 → 标志的映射（判据来自数据字典，非代码硬编码）

两个 `ReadData` 从 DABACON 字典按**数值字段号**把标志读进对象（bool 用 `sub_55BB6C3`，typed 用 `sub_55BB60C`/`internalGetField`）：

- `DB_Noun::ReadData` @ `0x58d6d20`：
  - `graphicsBehaviour` = 字段 **5099119** → `this+180`（int）
  - `geomset` = 字段 **859903**，`extrusion` = 字段 **663225**（bool）
- `DB_Attribute::ReadData` @ `0x58ce1f0`：
  - **`wnoevt` = 布尔字段 `299311034` → `this+184`**
  - **`wnoclm` = 布尔字段 `193514909` → `this+185`**
  - `change` = 字段 `76272573`

> 含义：**"这次属性改动要不要往下游走"是一条 DDL（数据字典）配置**——每个属性在字典里带 `wnoevt` 位，改字典即可改变某属性是否触发事件/重算，无需改代码。这也解释了为什么它是"声明式"而非 `if/else` 硬编码。

### 10.2 判据方法都是 core.dll 的导出符号 → 真正的几何消费者在外部模块

- `graphicsBehaviour / geomset / wnoevt / wnoclm / postSetAttribute / SubscribePostSetAttribute / attributeModified …` 的**唯一静态引用**都是来自 `0x5e14028` 的 data 交叉引用。
- 实证 `0x5e14028` 是 PE **导出地址表（EAT）**：读出的前 8 字节为 `B0 90 6A 00 C0 90 6A 00` → RVA `0x6A90B0`、`0x6A90C0`；`0x6A90B0 + imagebase 0x5170000 = 0x58190B0`，正是 `survey_binary` 列出的 export #1。并且 `0x5eec197` 存放导出名串 `?graphicsBehaviour@DB_Noun@@QBEHXZ`。
- **结论**：这些判据/订阅/事件方法是 core.dll 的**导出 API**，由**其它 DLL（设计/绘图/图形模块，如 des/draw 等）**在运行时调用与订阅。因此"哪些具体 ATT 被登记为影响几何"在 core.dll **静态不可见**（属运行时注册）；core.dll 只提供 `wnoevt` 这道 **DDL 级总闸** + 事件/变更基础设施。要拿到"影响几何的具体属性清单"，需转向消费方模块（下一步方向）或运行时观测订阅表。

### 10.3 受影响元素的扩散：克隆 / 分布式属性

`postSetAttribute` 里的 `sub_5992C80`（`0x5992c80`）决定"改一个属性影响哪些元素"：

- 若属性 `DB_Attribute::isCloneable`：`DB_Clone::getRelatedElements`（`0x59ac380`）——
  - 分布式属性绑定元素（`DB_DistAtt::isBoundElement`）→ `getBoundElementsToModify`；
  - 否则 `DB_Clone::getClones` 取所有克隆副本。
- 否则：只影响该元素本身。

即：**改"可克隆"属性会把 post-set-attribute 事件扩散到所有克隆/绑定副本**，这些副本的几何也随之被标记需要重算。这对增量重算的"波及范围"很关键——Rust 侧若支持 copy/clone/镜像，需要一并把关联副本纳入重算集合。

### 10.4 `DB_ComparisonSession`：目录依赖不是只看本元素，而是递归比较引用闭包

第二轮对 comparison-session 路径的反编译补齐了 `CATR`/`SPRE` 的作用边界。它们不是 `wnoevt` 总闸的替代品，而是在“已经要判断某元素/目录是否变化”时扩展**依赖闭包**：

| 函数 | 地址(core) | 已确认逻辑 |
|---|---|---|
| `DB_ComparisonSession::isModified(element, attr, qualifier, out)` | `0x5a4a7e0` | 通用按属性类型/标量或列表比较新旧会话值；`CATMOD`、`GEOM` 有专门分支 |
| `DB_ComparisonSession::isModified(element, out)` | `0x5a4a600` | `hasElementChangedSince` 后再查 `attributesChangedSince` / `rulesChangedSince`，排除“只有会话号前进、无实际属性/规则差异” |
| `DB_ComparisonSession::isDependeeModified` | `0x5a4a2f0` | 递归检查本体、`MEMB`，以及 `ATTLIS` 中所有 element-reference(type=5)；跳过 `OWNER` 和 UDA |
| `DB_ComparisonSession::dbSesModified` | `0x5a49940` | 非 CATA 元素显式沿 `SPRE`、再沿 `CATR` 进入目录；进入 CATA 后递归成员和引用属性，并按 DB/session 记忆避免重复遍历 |

`isModified(CATMOD)` 的实际分支是：

```c
target = getAtt(SPRE);
if (!target.exists)
    target = getAtt(CATR);

if (isModified(the_chosen_ref_attribute))
    return true;
return target != NULL && isDependeeModified(target);
```

因此即使设计元素自己的 `CATR`/`SPRE` 没变，只要它们指向的目录元素、目录成员、规则或进一步引用发生变化，聚合属性 `CATMOD` 仍会报告 modified。`isModified(GEOM)` 则直接读取派生判据 `GEODIF`。这给 Rust 侧一个明确约束：**只按“本 refno 的 modified_attrs”过滤不够；目录引用必须反向扩散到所有实例，且目录侧要计算传递依赖闭包。**

### 10.5 下一步可深挖方向

1. **其它消费方模块**：Core3D 主链已定位；还可对 `AfiModeling` / `PanelModelling` / `FunctionalModelling` / `CommonReferenceModelling` 的订阅回调做同样的属性常量扫描，补齐专业模块特例。
2. **`graphicsBehaviour` 枚举取值**：其 int 语义定义在 DDL 字典（字段 5099119），可从字典/DDL 侧或消费方对该值的比较逻辑还原。
3. **运行时权威导出**：从活 E3D 会话导出 `wnoevt`/`isPseudo`/`casc`，验证 `PRTREF`、`TYPEX` 等边界属性；Rust 侧把目录反向引用、克隆/分布式副本纳入波及闭包。

---

## 11. 消费方（`Core3D.dll`）：属性变更 → 3D 模型/图形重建

承接 §5.2——core.dll 只广播/记账，真正把变更转成几何重建的是 **`Core3D.dll`**（14.5 MB 原生 C++，已 `idalib_open` 为 session `core3d`）。它 import 了 core.dll 的 `SubscribePostSetAttribute` 等导出符号（导入扫描实证：Core3D.dll / AfiModeling.dll / PanelModelling.dll / FunctionalModelling.dll / CommonReferenceModelling.dll 都是订阅方；`Aveva.Core.Database.Implementation.dll` 是 .NET 封装）。

### 11.1 谁订阅：`DESDRA_SCPlugs`（设计/绘图变更插件）

`DESDRA_SCPlugs`（DES=design，DRA=draw）在 `Init`（`0x10409160`）里向 core.dll 注册：
- `DB_LegalityChecksPlugger`：Create/Modify/Delete/Move/CopyAttribute/SetXxxAttribute **Allowed**（合法性检查）
- `DB_ElementChangesPlugger`：`PostCreateElement`/`PostCopyAllElement`/`PostReorderElement`… 并实现 **`PostSetAttribute`/`PostSetName`/`PostSetRefAttribute`/`PostSetRefListAttribute`**
- `DB_ProjectEventsPlugger`、`DB_MDBPlugger`

即它是**全局订阅者**：接收所有"通过 `wnoevt` 闸门"的变更，再自行按 (noun, attr) 分派——而非逐属性订阅。

### 11.2 属性变更的分派：按 (nounHash, attrHash) 定点修正

`DESDRA_SCPlugs::PostSetAttribute`（`0x10409a60`）：

```c
db = DB_DB::findDB(el->dbno());
attrHash = DB_Attribute::hashValue(attr);
nounHash = DB_Noun::hashValue(el->hardType());
if (db && DB_DB::type(db)==7) sub_1005D702(el->asPointer(), &nounHash, &attrHash); // type7=DRAFT/2D
else                          sub_101F33A9(el->asPointer(), &nounHash, &attrHash); // 3D 设计
```

3D 入口 `sub_101F33A9`（模块字符串 `descases/VDESPT`）内是**硬编码的 (nounHash, attrHash) 特例**，命中即做对应几何量重算（向量运算 `VDIFF/MVMLTI/VUNIT/VSUM`、实数组 `DGETRA/DPUTRA`）。这些 hash 是 dabacon 名字哈希，**已用 §12 的解码器还原成真实名字**（`*a3`=noun，`*a4`=attr）：

| nounHash | noun | attrHash | attr | 该特例做什么 |
|---|---|---|---|---|
| `0xCC949` | `PLOO` | `0xA5056` | **`HEIG`** | 高度改动 → 重算 |
| `0xAFBC4` | `SJOI` | `0x9D04E` | `JFRE` | 方向/位置向量重算（VDIFF→矩阵乘→单位化→VSUM 写回） |
| `0xCA761` | `COCO` | `0xD3371` | `CTYP` | 数组重算 |

另外该函数还比较了 `0xCD240`=`PPRO`、`0xCD234`=`DPRO`、`0x8A1E7`=`DATA` 等（P-point / 设计点联动修正）。这些是"派生几何量随特定属性联动"的定点修正——**通用重建仍由 §11.3 的 PartialUpdateDesiMgr 负责**。

#### 11.2.1 引用属性专用入口：`SPCO.PRTREF` 有显式 post-set 级联

标量入口之外，`DESDRA_SCPlugs::PostSetRefAttribute`（`0x10409be0`）把 `(elementPointer, nounHash, attrHash, newRefPointer)` 交给 `descases/VDESPF`（`sub_101F2A27`）。该函数先调用 `structures/BAKREF`（`sub_102D4724`）维护通用反向引用，再执行一个明确特例：

```c
if (noun == SPCO && attr == PRTREF) {
    copy(new_ref, path=GPART, source=CATR, dest=CATR);
    copy(new_ref, path=GPART, source=DETR, dest=DETA);
    copy(new_ref, path=GPART, source=MATX, dest=MATX);
    copy(new_ref, path=GPART, source=CMPR, dest=CMPR);
    copy(new_ref, path=GPART, source=BLTR, dest=BLTR);
    copy(new_ref, path=GPART, source=TMPR, dest=TMPR);
}
```

原指令是六组 `GATRF1(newRef, GPART, source)` → `BPUTF(dest, currentElement)`。这证明 `PRTREF` 不是仅供显示的普通引用：在 `SPCO` 上修改它会同步当前元素的目录/明细/材料等派生属性。其影响是 **noun-scoped**，不能据此推成“所有 NOUN 的 PRTREF 都必然改几何”，但增量过滤若完全忽略它会漏掉真实 Core3D 级联。

#### 11.2.2 `GATCAT`/`GATCRF`：`SPRE` 选入口、`CATR` 进目录、`PRTREF` 为 TABITE 跳板

目录解析例程进一步给出了三者的分工：

- `catdblib/GATCAT`（`sub_1035C340`）默认以 **`SPRE`** 作为规格/目录入口；`NOZZ`/`ELCONN`/`EQUCOM` 改用 **`CATR`**，`TUBI` 会按 `TYPE` 选 `HSTU/LSTU/HSRO/LSRO`。
- 对 `TABITE`，`GATCAT` 先读取 **`PRTREF`**：请求 `SPRE` 时直接返回该目标，请求其它目录属性时以该目标继续 `GATCRF`。
- `catdblib/GATCRF`（`sub_1035D7D8`）若当前 `TYPE==TABITE` 同样先沿 **`PRTREF`** 跳转；请求 `GMRE/PTRE/GSTR/PSTR/DTRE/NGMR/CATR/CCORRE` 时再沿 **`CATR`** 进入目录对象。请求 `CATR` 本身时返回目标的 `REF`，其它请求则在 CATR 目标上读取。

这条链可以概括为：

```text
设计元素 --SPRE/按 noun 选择的引用--> 规格/目录入口
         --PRTREF (TABITE)----------> 被引用零件/表项
         --CATR---------------------> 实际目录组件及几何/设计表属性
```

哈希实证：`CATR=0xDBCF9`、`SPRE=0x9D165`、`PRTREF=0x557F908`。其中 PRTREF 的 Core3D 原始立即数命中 `VDESPF`，另在 `GATCAT/GATCRF` 的只读常量区作为路径属性使用。

### 11.3 通用增量重建：`PartialUpdateDesiMgr`（改了就把所在模型块按粒度排队）

```c
// PartialUpdateDesiMgr::ModelToUpdate  0x1047e590
if (DB_DB::type(el->getDB())==1) {          // 仅 DESIGN 库
    if (el->climb(NOUN_XGEOM).isNull() && !IsPending(el,state))
        GranularityExpansion(el, state);    // 计算重算粒度并入队
}
```
- `ChangedModelToUpdate`（`0x1047c200`）用 Fortran DB 历史查询（`HQLNIR/HGETIA`）遍历**变更元素集**，逐个 `ModelToUpdate`。
- `GranularityExpansion`（`0x1047d8c0`）决定"重算多大范围"：`IsPrimitive` 判是否几何图元 → `SignificantOwner` 上溯到"有意义的几何容器"（在该层重算而非单图元）→ `Members` 展开成员、`AbsentPrimitives` 处理被删图元、`AncestorDeletes` 处理祖先删除；`ModelState` 区分 added/modified/…。
- **结论：通用路径不逐属性判定——只要某 DESIGN 元素发生了（过 `wnoevt` 闸门的）变更，就把它所属的几何块按粒度排队重算**；属性级的精细联动由 §11.2 的 (noun,attr) 特例补充。

### 11.4 图形层落地：`DES_DrawListManager` / `GFX_GraphicsManager`

- `DES_DrawListManager::hasTopLevelGraphicsChanged`（`0x1052c850`）：用 **`DB_Element::attributesChangedBetween(会话区间)`** 取"两会话间变了哪些属性"判定顶层图形是否变化（会话号=sesno 增量）。
- `DES_DrawListManager::updateGraphics`（`0x1052d330`）：`UpdateChangeList` 后遍历 draw list，重建变化的渲染批次。
- `GFX_GraphicsManager::Update/DoDbUpdate/IsInterestedInUpdate`（`0x10797060`…）：把更新应用到图形场景。

### 11.5 端到端总结（两模块合起来回答"如何分辨 ATT 改动是否要重算模型"）

1. **core.dll**：`putAtt → postSetAttribute → wnoevt 闸门 → 广播 + DB_UserChanges 记账`（NOUN 有无几何 = `geomset/graphicsBehaviour`）。
2. **Core3D.dll `DESDRA_SCPlugs`**：全局收下所有过闸变更 → 按 (nounHash, attrHash) 定点修正（`sub_101F33A9`），并由 `PartialUpdateDesiMgr` 把所在 DESIGN 几何块按粒度（`SignificantOwner`/`XGEOM`）排队重算。
3. **图形层**：`DES_DrawListManager`/`GFX_GraphicsManager` 用会话增量 `attributesChangedBetween` 判定并重建 draw list / 场景。

> **对 plant-model-gen 的最终启示**：「**是否重算**」= core.dll 的 `wnoevt`（属性级）× `geomset`（NOUN 级）；「**重算多大范围**」= Core3D 的 `SignificantOwner`/`XGEOM` 粒度。Rust 侧增量重算宜：(a) 用 `wnoevt` 语义过滤无意义属性；(b) 命中后不必逐属性精算，按"有意义几何容器（significant owner）"为粒度重算该块；(c) 波及克隆/分布式副本（§10.3）。

### 11.6 Core3D.dll 关键符号

| 符号 | 地址(core3d) | 作用 |
|---|---|---|
| `DESDRA_SCPlugs::Init` | `0x10409160` | 向 core.dll 注册所有变更/合法性订阅 |
| `DESDRA_SCPlugs::PostSetAttribute` | `0x10409a60` | 属性变更入口，按 (nounHash,attrHash) 分派 |
| `DESDRA_SCPlugs::PostSetRefAttribute` | `0x10409be0` | 引用属性变更入口，传入新引用目标 |
| `sub_101F33A9` / `sub_1005D702` | `0x101f33a9` / `0x1005d702` | 3D / DRAFT 的 (noun,attr) 特例几何修正（descases/VDESPT） |
| `sub_101F2A27` (`VDESPF`) | `0x101f2a27` | 通用反向引用维护 + `SPCO.PRTREF` 派生属性级联 |
| `sub_1035C340` / `sub_1035D7D8` | `0x1035c340` / `0x1035d7d8` | `GATCAT/GATCRF` 目录引用解析（SPRE/PRTREF/CATR） |
| `PartialUpdateDesiMgr::ChangedModelToUpdate` | `0x1047c200` | 遍历变更元素集入队 |
| `PartialUpdateDesiMgr::ModelToUpdate` | `0x1047e590` | 仅 DESIGN 库；XGEOM 判定后入队 |
| `PartialUpdateDesiMgr::GranularityExpansion` | `0x1047d8c0` | 计算重算粒度（IsPrimitive/SignificantOwner/Members/AncestorDeletes） |
| `DES_DrawListManager::hasTopLevelGraphicsChanged` | `0x1052c850` | 用 `attributesChangedBetween` 判定顶层图形变化 |
| `DES_DrawListManager::updateGraphics` | `0x1052d330` | 重建 draw list |
| `GFX_GraphicsManager::Update` / `DoDbUpdate` | `0x10797060` / `0x107962e0` | 应用到图形场景 |

> 复现：`idalib_open {input_path:"D:\\AVEVA\\Everything3D3.1\\Core3D.dll", session_id:"core3d"}` 后，用 `decompile {addr, database:"core3d"}` 查看以上函数。

---

## 12. 附录：dabacon 名字哈希（DEHASH）解码器——把 (nounHash, attrHash) 还原成名字

§11.2 的 (noun,attr) 是 dabacon **名字哈希**（`DB_Noun::hashValue`=`*(this+92)`、`DB_Attribute::hashValue`=`*(this+4)`，均为字典里预存的哈希）。core.dll 里 `DB_Attribute::hashName`/`DB_Noun::hashName` → `DB_FortranInterface::hashValueToString`(`0x58dc160`) → `dehashVal`(`0x58dbcc0`) → Fortran `DEHASH`(`0x525e9fc`) 负责**哈希→名字**。

### 12.1 算法（从 `DEHASH` 反汇编还原）

三段分支（按 hash 大小）：
- `hash ≤ 0x81BF2`(=27⁴+1=531442)：短名/特殊分支（本文涉及的常量都不落此段，未展开）
- `0x81BF2 ≤ hash ≤ 0x171FAD39`(=387951929)：**主分支**——`x = hash − 531441`（=27⁴），随后取 6 位 **27 进制**（小端）：`d = x%27`，`d==0`→填充/空格，`d∈1..26`→`chr(0x40+d)`（即 `A..Z`），`x//=27`；名字取到"最后一个非零位"。
- `hash > 0x171FAD39`：UDA/UDET，名字是另存的字符串（`DB_Attribute::name`/`DB_Noun::fullName`）。

即内建 noun/attr 的名字 = **26 字母 + 空位** 的 27 进制打包整数（≤6 字符）。Python 复刻：

```python
def dehash(h: int) -> str:
    if h < 0x81BF2 or h > 0x171FAD39:
        return f"<special:{h}>"          # 短名分支 / UDA，见上
    x = h - 531441                       # 27^4
    chars, last = [], 0
    for i in range(1, 7):                # 最多 6 字符，小端
        d = x % 27
        if d:
            chars.append(chr(0x40 + d))  # 1..26 -> A..Z
            last = i
        else:
            chars.append(' ')            # 空位
        x //= 27
    return ''.join(chars[:last])
```

**验证**（已解出且为真实 PDMS 名）：`0xA5056→HEIG`、`0x9E770→SIZE`、`0x853B1→POS`、`0x8502A→DIR`、`0xF139C→VIEW`、`0x9AB88→SHEE`、`0xB73DE→LOCK`、`0xD9485→OVER`、`0x1501AC41→GRIDNX`、`0xFCF3790→ASMBLR`、`0x4B7E481→MSTYLE`。

### 12.2 DRAFT/2D 路径 `sub_1005D702`（DB type 7）解出的属性（部分）

（这些是 2D 出图/DRAFT 视图相关属性——改动触发 DRAFT 重绘；对 3D `plant-model-gen` 一般不相关，仅供参考）

`LOCK DDNM IDLN IDNM ASMBLR GRIDNM SHEE SHTMPL OVER BACK LALB ISOLB SYLB SIZE APPT ADIR DPPT DPBA DPOI POS NPPT BAIN PKEY PKDI VIEW RCOD XYPS DIR THPO FRPO PERS VSCA ADEG ONPO VRAT IDLI ADDE REME VSEC SPLA FPLA PPLA WPOS TAGR TMRF GLAB SLAB LAYE AXESYM AXSPRI LVIS SYTM SORF DTER FTER MSTYLE GAPS …`

### 12.3 用法

要解任何一个 dabacon 哈希（例如从别的模块里看到的比较常量），把整数丢进 `dehash()` 即可；反过来要算某名字的哈希：`h = 531441 + Σ (ord(c)-0x40) * 27**(i)`（i 从 0 起，`A=1..Z=26`）。这样即可在 `plant-model-gen` 里自建「属性名 ↔ dabacon 哈希」双向表，配合 §7 的属性显著性表使用。

---

## 13. 哪些属性影响模型生成（权威判定规则 + 几何输入属性清单）★

这是本次分析的重点结论：**"改哪个属性需要重算模型"** 有两层答案——(1) core.dll 的权威门控规则；(2) 几何生成实际读取的"几何输入属性"清单（实测自 `rs-core` + `plant-model-gen` 代码）。

### 13.1 判定规则（core.dll 权威语义）

一次属性改动触发模型重算，当且仅当：
1. 该属性 **`wnoevt == false`**（会发事件，§4.2/§10.1），**且**
2. 元素的 NOUN 是几何类型（**`geomset`/`graphicsBehaviour`** ↔ Rust `is_geo_noun`，§4.1）。

命中后，core.dll 广播 → Core3D `PartialUpdateDesiMgr` 把该元素所属**几何块（significant-owner 粒度）整体排队重算——通用路径不区分具体哪个属性**（§11.3）。波及范围：owner 的摆放改动波及其几何子元素；catalogue（`CATR`/`SPRE`，以及 `SPCO/TABITE` 上的 `PRTREF`）改动波及所有引用它的实例；可克隆属性波及克隆/分布式副本（§10.3）。

> 严格按 PDMS 语义：**任何 `wnoevt=false` 的属性改在几何元素上都会触发重算**。但很多这类属性并不改几何（只改外观/元数据）——`plant-model-gen` 可以更精细，**仅在"几何输入属性"变化时才重算**。清单见 §13.2。

### 13.2 几何/模型输入属性清单（代码读取点 + Core3D 引用级联）

证据来源：`resolve.rs::cata_context_from_session` / `query_gm_param*` 及全仓 `get_*("ATTR")` 的 577 处读取点聚合，再并入 §11.2 的 Core3D 标量/引用回调与目录解析证据。按类别（改动 → 需重算几何或模型派生数据）：

| 类别 | 属性 | 说明 |
|---|---|---|
| **A. 摆放/变换** | `POS` `POSL` `POSS` `POSE` `NPOS` `CPOS` `ORI` `YDIR` `ZDIR` `PAXI` `PZAXI` `PLAX` `ARRI` `LEAV` `BANG` | 位姿/朝向/管件到-离点/弯角，改→位姿变（X 轴由 ORI+Y/Z 派生，无独立属性） |
| **B. 目录/规格选型** | `CATR` `SPRE` `PRTREF` `CREF` `HREF` `TREF` `PSPE` `NGMR` `GTYP` `CTYP` | 改→换了元件/目录解析入口，几何或派生目录数据全变（影响最大）；`PRTREF` 有 `SPCO` post-set 与 `TABITE` 解析特例（§11.2.1/§11.2.2） |
| **C. 设计参数** | `DESP` `DELP` `PARA` `RINS` `OPDI` `UNIPAR` | 参数化尺寸（喂给目录几何表达式） |
| **D. 图元/目录尺寸** | `HEIG` `ANGL` `RADI` `DIAM` `PRAD` `PWID` `PHEI` `PDIA` `PBDM` `PTDM` `PDIS` `PBDI` `PTDI` `PXTS` `PYTS` `PXBS` `PYBS` `PXLE` `PX` `PY` `PZ` `DX` `DY` | 高/角/径/宽 + P-point 尺寸 |
| **E. 管路/布线** | `ARRI` `LEAV` `ZDIS` `ROUT` `DRNS` `DRNE` `CURD` `CURTYP` `DETR` | 到-离/坡降/路由/曲率 |
| **F. 定位/对齐** | `JUSL` `SJUS` `JLIN` `JFRE` | 对齐/justification；`JFRE`（§14.3 Core3D VDESPT 特例，与 noun `SJOI` 联动） |
| **G. 设计表/覆盖** | `DTRE` `DKEY` `DPRO` `PPRO` `PTYP` `PSTR` `PKEY` `PKDI` | 设计表默认值/属性覆盖 |

### 13.3 明确"不影响几何"的属性（只改这些 → 可跳过重算，这是优化点）

可明确按“几何/模型输入”跳过的主要是 `NAME` `DESC` `REFNO` `DBNUM` `NUMBDB` `RTEX` `CLAI`(claim/锁) `SKEY` `PURP` `FUNCTION`，以及所有已确认 `wnoevt=true` 的属性。

原先把 `OWNER/TYPE/NOUN/LEVE/NAPP/STYP` 放在本节过于激进：`OWNER` 改变世界变换/层级，`TYPE/NOUN/STYP` 改变生成分派，`LEVE/NAPP` 改变参与或显示语义，均应在“宁多勿漏”的生成器策略中保留。`OBST` 不改网格形状，但影响碰撞/参与范围，也属于模型输出策略而非纯元数据。

`TYPEX`（`0xCC6B3F`）在 core 只有两处立即数命中：`DB_Element::checkForUnknownAtts`（`0x5928cf0`）把它列入可容忍的内部/扩展属性；`RCF_Output::outPutSpecificAtt`（`sub_5A99370`，比较点 `0x5a99525`）遇到它直接跳到成功返回，不走普通属性序列化。Core3D 的立即数、符号和 import 扫描均未找到静态消费者。故它当前应标为 **internal/serialization-suppressed，不能仅凭名字并入几何白名单**；最终仍需活会话读取它的 `wnoevt`/pseudo/type 元数据确认。

### 13.4 给 plant-model-gen 增量重算的落地建议

1. 建一张 `GEOM_AFFECTING_ATTS`（≈ §13.2 全体并集，用 dabacon 哈希存，配合你们已实现的 dehash）。
2. 增量判定：某几何 refno 的"被改属性集合 ∩ `GEOM_AFFECTING_ATTS` ≠ ∅" → 重算该元素几何；否则**跳过**（纯元数据/外观改动，省掉无谓重算）。
3. 波及：owner 的 A 类（摆放）改动 → 重算其几何子树；`CATR`/`SPRE`/noun-scoped `PRTREF` 改动或其目标目录闭包变化 → 反向重算所有实例；可克隆属性 → 波及副本。
4. 粒度：以 **significant owner**（几何容器，如 `EQUI`/子设备/`BRAN`）为重算单位，而非单图元（对齐 Core3D `GranularityExpansion`）。

当前 `model_impact.rs::attribute_affects_model` 已含 `CATR/SPRE`，但尚缺本轮确认的 `PRTREF`。现有 API 只有 attr、没有 noun；短期按“宁多勿漏”可把 `PRTREF` 加入全局 allowlist，长期改成 `(noun, attr, effect)` 表，把 `SPCO/TABITE` 特例及 `dependency-cascade` 与 `direct-mesh` 区分开。

> 提示：§13.2 是"读取即用到 + Core3D 引用级联"的经验并集，覆盖设计件/图元/管路主路径；若要 100% 对齐 PDMS，仍以 §13.1 的 `wnoevt` 为最终权威（该标志是每属性的字典位，可从 dabacon 字典批量导出与本清单核对，见 §14）。

---

## 14. 与权威属性字典交叉校验（SurrealDB `att_meta`，702 个属性）

对该项目（`AvevaMarineSample`，ns=`1516`）运行中的 SurrealDB（`127.0.0.1:8020`，SurrealDB 3.3）`att_meta` 表做交叉校验。`att_meta` = **702 个属性**，字段 `id`(=`att_meta:<名>`)、`hash`(dabacon 哈希)、`meta_cn_name`。

### 14.1 §13.2 校验结果：全部命中（附 dabacon 哈希），仅剔除 2 个派生/误报

§13.2 的几何输入属性**全部是真实 dabacon 属性**（在 702 属性字典里命中，并拿到各自 hash），仅 2 个剔除：
- `XDIR`：X 轴由 `ORI`+`YDIR`/`ZDIR` 派生，字典无此独立属性（`YDIR`=`0xD9E0D`、`ZDIR`=`0xD9E0E` 存在，`XDIR` 不在）。
- `RAD`：真实属性是 `RADI`(`0xADB7D`)，`RAD` 系聚合误报。

命中示例（可直接建 `GEOM_AFFECTING_ATTS` 哈希表）：`POS`=`0x853B1`、`ORI`=`0x83787`、`CATR`=`0xDBCF9`、`SPRE`=`0x9D165`、`PRTREF`=`0x557F908`、`DESP`=`0xD20C7`、`PARA`=`0x89C41`、`HEIG`=`0xA5056`、`DIAM`=`0xC0748`、`JUSL`=`0xBEEF1`、`PDIA`=`0x882F1`、`ARRI`=`0xB0515`、`LEAV`=`0xEBADF`…（全部 hash 可从 `att_meta` 直接 `SELECT name, hash` 取。）

### 14.2 关于 `wnoevt=false` 全量清单（重要结论）

- **`wnoevt` 不在已同步的 SurrealDB 里**：`att_meta` 只有 `hash`+`meta_cn_name`（702 条），**无 `wnoevt`/事件标志**；`dicvir.dat`(0.27MB) 是版本戳、非字典。
- **`wnoevt` 是 E3D 内核 dabacon 字典的每属性标志**（运行时 `DB_Attribute` 偏移 184 / dabacon 字段 `299311034`，§10.1）。内建属性字典编译在内核里、不随模型库同步 → **静态无法直接导出**。
- **要拿全量 `wnoevt=false` 清单，需其一**：
  1. **活的 E3D 会话导出（权威）**：遍历 702 属性、逐个读其字典 `wnoevt` 标志 dump 出来；
  2. **扩展字典导入工具**：把字段 `299311034` 一并落进 `att_meta`（加一列 `wnoevt`），此后 `SELECT name FROM att_meta WHERE wnoevt=false` 即得清单；
  3. **等价可行替代（可立即做）**：用已加载的 `Core3D` 会话枚举其设计/几何代码引用的全部属性哈希（解码为名），与 §13.2 对齐——直接反映"3D 生成实际消费哪些属性"，比原始 `wnoevt` 超集更贴合"影响模型生成"。

> 交叉校验结论：**§13.2 清单在权威属性字典中 100% 命中（除 2 个派生项），可放心作为 `GEOM_AFFECTING_ATTS` 的基础**；`wnoevt` 精确门控需按 14.2 其一补齐。

### 14.3 消费方（Core3D）反向交叉校验：§13.2 基本完备

第一轮用已加载的 `Core3D` 会话，把标量属性分派函数（`sub_101F33A9` 3D-VDESPT、`sub_1005D702` DRAFT）里的**全部比较常量**解码，并用 `att_meta`（702）判定"是属性还是 noun"：

- 这些常量**大多是 NOUN 哈希**（分派按 (noun, attr) 键控）：3D 侧 `PLOO`/`SJOI`/`COCO`/`DATA`；DRAFT 侧 `SHEE`/`VIEW`/`LAYE`/`OVER`/`GRIDNM`/`MSTYLE`… 均为元素类型（noun），非属性。
- 真正是**属性**且被 Core3D 设计代码引用的：`HEIG`/`PPRO`/`DPRO`/`POS`/`PKEY`/`PKDI`（已在 §13.2）+ 3 个新增：
  - **`CTYP`**（组件类型，与 noun `COCO` 联动）、**`JFRE`**（与 noun `SJOI` 联动）→ **几何相关，已补入 §13.2（B/F）**。
  - `LOCK`（DRAFT 锁定/状态标志）→ **非几何**（不纳入）。
- 该结论只覆盖**标量 `PostSetAttribute`**。第二轮继续扫描 `PostSetRefAttribute` 与目录解析器，新增确认：
  - `CATR`/`SPRE`：`GATCAT/GATCRF` 的主目录解析链，同时被 core comparison-session 用于递归目录依赖；
  - **`PRTREF`**：`SPCO.PRTREF` 在 `VDESPF` 有显式派生属性回写，`TABITE` 在 `GATCAT/GATCRF` 以它作为目录跳板；已补入 §13.2（B）；
  - `TYPEX`：core 仅用于 unknown-att 容忍，并在 RCF 输出时显式跳过；Core3D 无静态消费者，暂不纳入。
- **修正后的结论**：加入 `PRTREF` 后，§13.2 对当前已见 3D 标量特例、引用级联和目录解析路径覆盖完整；通用重建仍按 significant-owner 整块重算，少数 `(noun,attr)` 特例负责派生修正与依赖扩散。
