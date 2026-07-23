# core.dll 中「NOUN 的 ATT 修改是否需要重算模型」的判定机制

> 逆向对象：`D:\AVEVA\Everything3D3.1\core.dll`（AVEVA Everything3D 3.1 的 DABACON 数据库层）
> 分析工具：IDA Professional 9.2 + `ida-pro-mcp`（headless idalib，会话 `core31-readonly`）
> 模块信息：32 位 PE，imagebase `0x5170000`，image_size `0x4113000`，MD5 `52a236db644b0e11950c9cda7b93dd34`，
> 34,308 个函数 / 52,255 条字符串；本文所有地址均为 IDA 内已重定基地址。
> 结论与代码片段均来自反编译（Hex-Rays）实证，域含义处会标注「PDMS 语义」。

---

## 0. 结论速览（TL;DR）

> **2026-07-23 纠偏**：本文旧版把“是否重算”概括为 `wnoevt × geomset`。
> 继续下钻 `Core3D.dll::EVALAT/IDCHNG` 后确认，该结论把数据库事件门与设计变化门
> 混成了一层。以下以新链路为准。

完整判定分三层：

1. **core.dll 事件层**：`DB_Attribute::wnoevt()` 是事件总闸。置位后不走普通属性
   广播，也不写当前 `DB_UserChanges::attributeModified`。
2. **Core3D 设计变化层**：全局订阅者收到过闸变化后，`EVALAT → IDCHNG` 读取属性
   字典 **`DCHC`**；code 0 不进 `QCHGLS`，非零再按 noun、owner、component/ref
   特例选择或扩散目标。
3. **目标与粒度层**：`geomset/graphicsBehaviour` 是 NOUN 几何元数据，但不是与
   `wnoevt` 组成的严格与门；`SignificantOwner + Members`、引用反向边和 draw-list
   依赖共同决定最终重建对象及块级粒度。

一句话：**`wnoevt` 管“是否发数据库事件”，`DCHC/EVALAT` 管“是否形成设计变化及
如何传播”，noun/引用/`SignificantOwner` 管“重算谁、重算多大”。**

core.dll 的 `DB_UserChanges` 与 Core3D 的 legacy change list `QCHGLS`
（`ref[2] + changeCode`）是两套相关但不同的数据，不能互相代称。

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
    if ( !DB_Attribute::wnoevt(a3) )          // ★ 事件门：no-event 时不做普通广播/attributeModified
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

- `wnoevt` 是**普通属性事件的总闸**，不是完整的“影响模型”判据。
  `xrefs_to 0x58d5290` 证实它只被三个发事件的分发器检查：
  - `PostSetAttribute`（标量属性）`0x591e5b0`
  - `PostSetRefAttribute`（引用属性）`0x591e720`
  - `PostSetRefListAttribute`（引用列表属性）`0x591e780`
- 两种订阅：
  - 全局订阅 `SubscribePostSetAttribute(handler)` `0x581f730`
  - **按属性订阅** `SubscribePostSetAttribute(const DB_Attribute*, handler)` `0x581f750`
    （红黑树按属性指针建索引）。
- Core3D 的 `DESDRA_SCPlugs` 实际注册为**全局订阅者**，接收所有过 `wnoevt` 的变化，
  再由 `DCHC/EVALAT` 分类；不能再把“按属性订阅表”描述成 Core3D 的模型影响白名单。
- 因此允许 `wnoevt=false, DCHC=0`：业务订阅者/数据库变化仍可看到该属性变化，但它
  不进入 Core3D 的 `QCHGLS`。

---

## 4. 分层使用的字典字段

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

### 4.2 ATT 级事件与依赖标志（`DB_Attribute`）

`func_query "@DB_Attribute@@QBE_NXZ"` 列出的布尔字典标志（挑选与事件/依赖相关者）：

| 方法 | 地址 | 语义（PDMS） | 在当前链路中的作用 |
|---|---|---|---|
| **`wnoevt()`** | `0x58d5290` | will-no-event，改动**不发普通属性事件**（**DDL 布尔字段 299311034** → off 184） | ★core 事件总闸；不是 Core3D 模型影响码 |
| **`wnoclm()`** | `0x58d5270` | will-no-claim，改动**无需 claim 元素**（**DDL 布尔字段 193514909** → off 185） | 影响是否把元素纳入工作集/session 记账（`sub_5991BE0` 的 claim 路径） |
| `change()` | `0x58cf270` | 属性可变 | 字典字段 76272573 |
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

### 4.3 Core3D 使用的设计变化码 `DCHC` / `PLCF`

`DB_Attribute::dchc @0x58cf550` 返回属性对象 off 92 的整数，底层字典字段为
**`DCHC=596407`**；`DB_Attribute::plcf @0x58d2830` 对应
**`PLCF=652066`**。Core3D `IDCHNG @0x1022e302` 通过 core 导出
`ATAINT @0x58de220` 读取它们：

```c
changeCode = ATAINT(attrHash, DCHC);
if (lookup_error) changeCode = 0;
else if (ATAINT(attrHash, PLCF) == 1) plcDeletePending = true;
return changeCode;
```

- `DCHC=0`：不把该属性变化写入 `QCHGLS`；
- `DCHC=1..4`：作为 `EVALAT` 的起点，再按 noun、owner、component/ref 特例重定向
  或扩散；相同 ref 由 `EVALST` 保留较大 code；
- `REDRAW`（hash `331445106`）强制 code 4，`INTUBE`（`73767168`）强制 code 1；
- `PLCF=1` 置 pending，`UPDATD` 开头调用 `PLCDEL`。

因此，`DCHC/EVALAT` 才是 Rust 模型影响分类器应近似的层。

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

> **重要范围界定**：在 core.dll 内搜索 `geometry/rebuildModel/makeGeom/...` 命名的函数
> **为空**。core.dll 作为 DABACON 数据库层，只负责属性写入后的事件门控、广播、
> `DB_UserChanges` 记账和 sesno 查询；“该变化是否形成设计/图形更新”由消费方
> Core3D 的 `DCHC/EVALAT/QCHGLS` 决定。`DB_UserChanges` 不能直接等同于
> Core3D 的设计变化队列。

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
| `DB_Attribute::wnoevt()` | 无实时 Plugger 等价层 | Rust 直接采集 sesno 数据变化；`wnoevt` 只对应 core 事件边界，不对应模型白名单 |
| `IDCHNG(DCHC) + EVALAT` | `classify_attribute_model_impact` + `classify_modified_element` | 最接近的模型影响分类；Rust 为 trigger/neutral/unknown-fallback，未保留 1..4 code |
| `DB_UserChanges` + `elementsChangedSince/Between` | `EleOperationData` / `ModifiedElement` / `PdmsSesnoElementChange` | 原始变化及属性差异 |
| `QCHGLS(ref, changeCode)` | `IncrGeoUpdateLog` → `GenerationTargets` | 筛选后的模型目标/欠账；code、传播原因和部分依赖边会丢失 |
| `geomset/graphicsBehaviour` + EVALAT noun 分支 | `insert_change_by_noun` / `targets_from_candidates` | Rust noun 名单主要承担直接目标执行路由，不是与 wnoevt 组成的严格影响门 |
| session 号（sesno）增量 | `sesno_version_anchor`（见 `AGENTS.md` specs/022） | 增量起点/锚点 |
| owner/member 结构影响 | `apply_critical_model_expansion` | 已补旧/新 owner 与 children 差集，仍非通用 SignificantOwner/Members |
| `PartialUpdateDesiMgr::SignificantOwner/Members` | loop 容器→owner 上溯 ≤6 层 | 仍缺通用块级粒度 |
| 同步变化批次 | data anchor → `model_gen_debt` → 连续追平 → model-gen anchor | Rust 的持久化容错扩展 |

### 可落地的改进建议

1. **模型影响表对齐 `DCHC/EVALAT`，不是 `wnoevt`**：保留当前未知属性/UDA
   `unknown_fallback`，只有明确 known-neutral 才跳过模型欠账。
2. **从 bool/三态升级为 effect**：区分 data-only、transform-only、direct-geometry、
   dependency-cascade、structural-membership、unknown；若取得 DCHC，保留原始 code。
3. **补齐目标闭包与粒度**：CATR/SPRE/SCOM 反向实例、克隆/绑定副本、
   SignificantOwner + Members。
4. **noun 名单只做最终执行路由**：非直接几何 noun 仍可通过引用/owner 传播产出目标，
   不能在影响分类阶段直接丢弃。

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
| `DB_Attribute::dchc` / `plcf` | `0x58cf550` / `0x58d2830` | Core3D 读取的设计变化码 / plot-clash 标记 |
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

另有 `DCHC=596407`（设计变化 code）与 `PLCF=652066`（plot/clash 标记）。

> 含义：属性的**事件可见性**由 `wnoevt` 数据字典位声明；属性进入 Core3D 设计变化
> 队列的起始 code 由 `DCHC` 声明。两者都是数据驱动，但用途不同。

### 10.2 判据方法都是 core.dll 的导出符号 → 真正的几何消费者在外部模块

- `graphicsBehaviour / geomset / wnoevt / wnoclm / postSetAttribute / SubscribePostSetAttribute / attributeModified …` 的**唯一静态引用**都是来自 `0x5e14028` 的 data 交叉引用。
- 实证 `0x5e14028` 是 PE **导出地址表（EAT）**：读出的前 8 字节为 `B0 90 6A 00 C0 90 6A 00` → RVA `0x6A90B0`、`0x6A90C0`；`0x6A90B0 + imagebase 0x5170000 = 0x58190B0`，正是 `survey_binary` 列出的 export #1。并且 `0x5eec197` 存放导出名串 `?graphicsBehaviour@DB_Noun@@QBEHXZ`。
- **结论**：这些字典访问器/订阅/事件方法是 core.dll 的**导出 API**，由其它 DLL
  在运行时调用。Core3D 不是靠逐属性注册表筛选几何属性，而是全局订阅后通过
  `ATAINT(DCHC/PLCF)`、`EVALAT` 及 noun/ref 特例分类。core.dll 提供事件与字典基础设施，
  Core3D 决定设计变化。

### 10.3 受影响元素的扩散：克隆 / 分布式属性

`postSetAttribute` 里的 `sub_5992C80`（`0x5992c80`）决定"改一个属性影响哪些元素"：

- 若属性 `DB_Attribute::isCloneable`：`DB_Clone::getRelatedElements`（`0x59ac380`）——
  - 分布式属性绑定元素（`DB_DistAtt::isBoundElement`）→ `getBoundElementsToModify`；
  - 否则 `DB_Clone::getClones` 取所有克隆副本。
- 否则：只影响该元素本身。

即：**改“可克隆”属性会把 post-set-attribute 事件扩散到所有克隆/绑定副本**。
各副本是否进入 `QCHGLS` 仍由其后的 `DCHC/EVALAT` 决定；对有模型影响的属性，
Rust 需要把关联副本纳入目标闭包。

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
3. **运行时权威导出**：从活 E3D 会话同时导出
   `wnoevt/DCHC/PLCF/isPseudo/casc`；`wnoevt` 用于事件兼容，`DCHC/EVALAT`
   用于模型影响校验。Rust 侧再补目录反向引用、克隆/分布式副本闭包。

---

## 11. 消费方（`Core3D.dll`）：属性变更 → 3D 模型/图形重建

承接 §5.2——core.dll 只广播/记账，真正把变更转成几何重建的是 **`Core3D.dll`**（14.5 MB 原生 C++，已 `idalib_open` 为 session `core3d`）。它 import 了 core.dll 的 `SubscribePostSetAttribute` 等导出符号（导入扫描实证：Core3D.dll / AfiModeling.dll / PanelModelling.dll / FunctionalModelling.dll / CommonReferenceModelling.dll 都是订阅方；`Aveva.Core.Database.Implementation.dll` 是 .NET 封装）。

### 11.1 谁订阅：`DESDRA_SCPlugs`（设计/绘图变更插件）

`DESDRA_SCPlugs`（DES=design，DRA=draw）在 `Init`（`0x10409160`）里向 core.dll 注册：
- `DB_LegalityChecksPlugger`：Create/Modify/Delete/Move/CopyAttribute/SetXxxAttribute **Allowed**（合法性检查）
- `DB_ElementChangesPlugger`：`PostCreateElement`/`PostCopyAllElement`/`PostReorderElement`… 并实现 **`PostSetAttribute`/`PostSetName`/`PostSetRefAttribute`/`PostSetRefListAttribute`**
- `DB_ProjectEventsPlugger`、`DB_MDBPlugger`

即它是**全局订阅者**：接收所有"通过 `wnoevt` 闸门"的变更，再自行按 (noun, attr) 分派——而非逐属性订阅。

### 11.2 属性变更的分派：`DCHC/EVALAT` 通用分类 + (noun, attr) 特例

`DESDRA_SCPlugs::PostSetAttribute`（`0x10409a60`）：

```c
db = DB_DB::findDB(el->dbno());
attrHash = DB_Attribute::hashValue(attr);
nounHash = DB_Noun::hashValue(el->hardType());
if (db && DB_DB::type(db)==7) sub_1005D702(el->asPointer(), &nounHash, &attrHash); // type7=DRAFT/2D
else                          sub_101F33A9(el->asPointer(), &nounHash, &attrHash); // 3D 设计
```

3D 入口 `sub_101F33A9`（模块字符串 `descases/VDESPT`）会先用 `ATAINT` 校验属性，
随后对所有可识别属性调用通用 `EVALAT @0x1022c679`；硬编码的
(nounHash, attrHash) 分支是**附加定点修正**，不是唯一模型入口。

#### 11.2.1 通用分类：`EVALAT → IDCHNG → QCHGLS`

`EVALAT` 调 `IDCHNG @0x1022e302`，后者通过 `ATAINT` 读取
`DCHC=596407` 与 `PLCF=652066`（§4.3）。当前控制流显式处理 code 0..4：

- `0`：不写 `QCHGLS`；
- `1/2`：先重定向到关联对象，再提升为 code 4；
- `3/4`：进入 component、point、owner、引用扩散分支；
- `REDRAW` 强制 code 4，`INTUBE` 强制 code 1。

`EVALCD @0x1022e020` 包装 `EVALST @0x1022e0a7` 写入 `QCHGLS`：

- 每项为两个 ref 整数 + 一个 change code；
- 相同 ref 去重；
- 新 code 较大时覆盖旧 code。

`sub_1022C3D7 @0x1022c3d7` 返回该全局 change-list handle
（`dword_10E98540`）。这证明 Core3D 在属性层先做通用 DCHC 分类，再进入块级重建。

#### 11.2.2 附加的 (nounHash, attrHash) 定点修正

VDESPT 中仍有少数硬编码分支，命中后做派生几何量修正（向量运算
`VDIFF/MVMLTI/VUNIT/VSUM`、实数组 `DGETRA/DPUTRA`）。这些 hash 是 dabacon 名字
哈希，已用 §12 解码器还原（`*a3`=noun，`*a4`=attr）：

| nounHash | noun | attrHash | attr | 该特例做什么 |
|---|---|---|---|---|
| `0xCC949` | `PLOO` | `0xA5056` | **`HEIG`** | 高度改动 → 重算 |
| `0xAFBC4` | `SJOI` | `0x9D04E` | `JFRE` | 方向/位置向量重算（VDIFF→矩阵乘→单位化→VSUM 写回） |
| `0xCA761` | `COCO` | `0xD3371` | `CTYP` | 数组重算 |

另外该函数还比较了 `0xCD240`=`PPRO`、`0xCD234`=`DPRO`、`0x8A1E7`=`DATA`
等（P-point / 设计点联动修正）。这些特例不取代 §11.2.1 的通用
`DCHC/EVALAT` 路径。

#### 11.2.3 引用属性专用入口：`SPCO.PRTREF` 有显式 post-set 级联

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

#### 11.2.4 `GATCAT`/`GATCRF`：`SPRE` 选入口、`CATR` 进目录、`PRTREF` 为 TABITE 跳板

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

### 11.3 `QCHGLS` 消费与 `PartialUpdateDesiMgr` 块级粒度

`Core3D!UPDATD @0x1022e5ac` 在图形子系统激活且当前不处于嵌套 update
（`UQGRAF(...) & 1 && !UQUCUR()`）时，按以下顺序执行：

1. `USCHGO(status)` 交付 core 当前变化；
2. `DES_DrawListManager::updateGraphics`，其入口先调 `UpdateChangeList`，把注册的
   draw-list 依赖以 code 4 扩入 `QCHGLS`；
3. `PartialUpdateDesiMgr::ChangedModelToUpdate` 消费扩展后的 `QCHGLS`；
4. `HLENIR` 清空 `QCHGLS`。

`ChangedModelToUpdate @0x1047c200` 从索引 1 开始按三元组步长读取两个 ref 整数，
对每项调用 `ModelToUpdate(element, 0)`。第三项 QCHGLS change code **不会**作为
`ModelState` 传入；QCHGLS code 与 PartialUpdate 的 state 是两套值。

```c
// PartialUpdateDesiMgr::ModelToUpdate  0x1047e590
if (DB_DB::type(el->getDB())==1) {          // 仅 DESIGN 库
    if (el->climb(NOUN_XGEOM).isNull() && !IsPending(el,state))
        GranularityExpansion(el, state);    // 计算重算粒度并入队
}
```

- `GranularityExpansion`（`0x1047d8c0`）决定"重算多大范围"：`IsPrimitive` 判是否几何图元 → `SignificantOwner` 上溯到"有意义的几何容器"（在该层重算而非单图元）→ `Members` 展开成员、`AbsentPrimitives` 处理被删图元、`AncestorDeletes` 处理祖先删除；`ModelState` 区分 added/modified/…。
- `ModelToUpdate` 只接收 DESIGN DB（`DB_DB::type==1`），排除 `XGEOM` 祖先和已排队项。
- `ChangedModelToUpdate` 还有 enable、root-valid、suppress 三个对象状态门。
- 已确认 ModelState：changed=0、new=1、deleted=3；值 4 有删除式分支，但静态 caller
  尚未闭合。

**结论：属性级是否入队已经在 `DCHC/EVALAT` 完成；进入 QCHGLS 后，
PartialUpdate 再按 SignificantOwner/Members 扩为块级重建。**

### 11.4 图形层落地：`DES_DrawListManager` / `GFX_GraphicsManager`

- `DES_DrawListManager::hasTopLevelGraphicsChanged`（`0x1052c850`）：用 **`DB_Element::attributesChangedBetween(会话区间)`** 取"两会话间变了哪些属性"判定顶层图形是否变化（会话号=sesno 增量）。
- `DES_DrawListManager::updateGraphics`（`0x1052d330`）：先调
  `UpdateChangeList @0x1052b3c0`。后者按 QCHGLS 三元组读取直接变化 ref，经注册
  draw-list 依赖匹配，把扩出的设计 ref 通过 `EVALCD(ref,4)` 写回 QCHGLS；随后重建
  满足 global/标记且未 suppress 的渲染批次。
- `GFX_GraphicsManager::Update/DoDbUpdate/IsInterestedInUpdate`（`0x10797060`…）：把更新应用到图形场景。

### 11.5 端到端总结（两模块合起来回答"如何分辨 ATT 改动是否要重算模型"）

1. **core.dll**：`putAtt → postSetAttribute → wnoevt 事件闸门 → 广播 +
   DB_UserChanges 记账`。
2. **Core3D 影响分类**：`DESDRA_SCPlugs → VDESPT → EVALAT → IDCHNG(DCHC)`；
   code 0 停止，非零按 noun/owner/ref 传播并写 `QCHGLS`。引用专用入口同时维护
   `BAKREF`。
3. **目标扩展与粒度**：DrawList 先扩依赖，PartialUpdate 再把 QCHGLS ref 通过
   `SignificantOwner/Members` 变成块级重建队列。
4. **图形落地**：`DES_DrawListManager` / `GFX_GraphicsManager` 重建 draw list 与场景。

> **对 plant-model-gen 的最终启示**：模型影响分类应近似
> `DCHC/EVALAT`，而不是 `wnoevt`；命中后按 SignificantOwner/Members 归一粒度，
> 并补齐目录反向引用、draw-list 等价依赖及克隆/分布式副本闭包。noun 名单应是最终
> 生成器路由，不应充当影响分类的严格前置门。

### 11.6 Core3D.dll 关键符号

| 符号 | 地址(core3d) | 作用 |
|---|---|---|
| `DESDRA_SCPlugs::Init` | `0x10409160` | 向 core.dll 注册所有变更/合法性订阅 |
| `DESDRA_SCPlugs::PostSetAttribute` | `0x10409a60` | 属性变更入口，按 (nounHash,attrHash) 分派 |
| `DESDRA_SCPlugs::PostSetRefAttribute` | `0x10409be0` | 引用属性变更入口，传入新引用目标 |
| `VDESPT` / `EVALAT` / `IDCHNG` | `0x101f33a9` / `0x1022c679` / `0x1022e302` | 3D 属性入口、通用影响传播、读取 DCHC/PLCF |
| `EVALCD` / `EVALST` / `QCHGLS` | `0x1022e020` / `0x1022e0a7` / `0x1022c3d7` | 写 change-list、按较大 code 去重、取得全局 handle |
| `UPDATD` / `UPDATN` | `0x1022e5ac` / `0x1022e3e7` | 设计更新入口；两者消费链并不完全相同 |
| `sub_101F2A27` (`VDESPF`) | `0x101f2a27` | 通用反向引用维护 + `SPCO.PRTREF` 派生属性级联 |
| `sub_1035C340` / `sub_1035D7D8` | `0x1035c340` / `0x1035d7d8` | `GATCAT/GATCRF` 目录引用解析（SPRE/PRTREF/CATR） |
| `PartialUpdateDesiMgr::ChangedModelToUpdate` | `0x1047c200` | 遍历 QCHGLS 三元组并以 ModelState=0 入队 |
| `PartialUpdateDesiMgr::ModelToUpdate` | `0x1047e590` | 仅 DESIGN 库；XGEOM 判定后入队 |
| `PartialUpdateDesiMgr::GranularityExpansion` | `0x1047d8c0` | 计算重算粒度（IsPrimitive/SignificantOwner/Members/AncestorDeletes） |
| `DES_DrawListManager::hasTopLevelGraphicsChanged` | `0x1052c850` | 用 `attributesChangedBetween` 判定顶层图形变化 |
| `DES_DrawListManager::UpdateChangeList` | `0x1052b3c0` | 由 draw-list 依赖扩展 QCHGLS |
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

要解任何一个 dabacon 哈希（例如从别的模块里看到的比较常量），把整数丢进
`dehash()` 即可；反过来要算某名字的哈希：
`h = 531441 + Σ (ord(c)-0x40) * 27**(i)`（i 从 0 起，`A=1..Z=26`）。
这样可在 `plant-model-gen` 中建立「属性名 ↔ dabacon 哈希」双向表，配合
`DCHC` 原始 code 与 Rust effect 分类使用。

---

## 13. 哪些属性影响模型生成（分层判定 + 几何输入属性清单）★

“改哪个属性需要重算模型”必须分开回答：core 事件是否可见、Core3D 是否形成设计
变化、变化如何映射到目标/粒度，以及 Rust 生成器实际读取哪些输入属性。

### 13.1 Core/Core3D 的分层判定规则

实时属性路径可概括为：

1. `wnoevt=false`：core 才执行普通属性广播和 `DB_UserChanges::attributeModified`；
2. `EVALAT/IDCHNG` 取得非零 DCHC（或 REDRAW/INTUBE 强制 code），再按 noun、owner、
   component/ref 特例选择或扩散目标，写 `QCHGLS`；
3. DrawList 扩展注册依赖，PartialUpdate 对目标做 DESIGN DB、XGEOM、去重等检查；
4. `SignificantOwner + Members` 把图元归一为块级重建目标。

因此：

- `wnoevt=false` 只是**必要的实时事件条件**，不是“必然重算”；
- `DCHC=0` 可保留数据库事件/数据变化，但不进入 QCHGLS；
- `geomset/graphicsBehaviour` 描述 noun 的几何能力，却不是与 wnoevt 组成的严格与门；
  非直接几何 noun 仍可能经 owner/ref/BAKREF/ATTABK 传播到设计实例；
- `DB_UserChanges` 与 `QCHGLS` 没有发现一个“整批直接转换”的唯一桥接函数。

### 13.2 几何/模型输入属性清单（代码读取点 + Core3D 引用级联）

证据来源：`resolve.rs::cata_context_from_session` / `query_gm_param*` 及全仓 `get_*("ATTR")` 的 577 处读取点聚合，再并入 §11.2 的 Core3D 标量/引用回调与目录解析证据。按类别（改动 → 需重算几何或模型派生数据）：

| 类别 | 属性 | 说明 |
|---|---|---|
| **A. 摆放/变换** | `POS` `POSL` `POSS` `POSE` `NPOS` `CPOS` `ORI` `YDIR` `ZDIR` `PAXI` `PZAXI` `PLAX` `ARRI` `LEAV` `BANG` | 位姿/朝向/管件到-离点/弯角，改→位姿变（X 轴由 ORI+Y/Z 派生，无独立属性） |
| **B. 目录/规格选型** | `CATR` `SPRE` `PRTREF` `CREF` `HREF` `TREF` `PSPE` `NGMR` `GTYP` `CTYP` | 改→换了元件/目录解析入口，几何或派生目录数据全变（影响最大）；`PRTREF` 有 `SPCO` post-set 与 `TABITE` 解析特例（§11.2.3/§11.2.4） |
| **C. 设计参数** | `DESP` `DELP` `PARA` `RINS` `OPDI` `UNIPAR` | 参数化尺寸（喂给目录几何表达式） |
| **D. 图元/目录尺寸** | `HEIG` `ANGL` `RADI` `DIAM` `PRAD` `PWID` `PHEI` `PDIA` `PBDM` `PTDM` `PDIS` `PBDI` `PTDI` `PXTS` `PYTS` `PXBS` `PYBS` `PXLE` `PX` `PY` `PZ` `DX` `DY` | 高/角/径/宽 + P-point 尺寸 |
| **E. 管路/布线** | `ARRI` `LEAV` `ZDIS` `ROUT` `DRNS` `DRNE` `CURD` `CURTYP` `DETR` | 到-离/坡降/路由/曲率 |
| **F. 定位/对齐** | `JUSL` `SJUS` `JLIN` `JFRE` | 对齐/justification；`JFRE`（§14.3 Core3D VDESPT 特例，与 noun `SJOI` 联动） |
| **G. 设计表/覆盖** | `DTRE` `DKEY` `DPRO` `PPRO` `PTYP` `PSTR` `PKEY` `PKDI` | 设计表默认值/属性覆盖 |

### 13.3 明确"不影响几何"的属性（只改这些 → 可跳过重算，这是优化点）

按当前 Rust 生成器输入可明确作为 known-neutral 的是 `NAME` `DESC` `PURP`
`FUNCTION`；`REFNO` `DBNUM` `NUMBDB` `RTEX` `CLAI`(claim/锁) `SKEY` 等可列为
候选，但在加入 neutral 集前仍应以生成器读取点、DCHC 和动态轨迹验证。

`wnoevt=true` 只能说明实时普通属性事件被抑制，**不能被重新解释成
“原始 sesno diff 中可安全忽略的数据字段”或“DCHC 必为 0”**。Rust 当前对未知属性
和 UDA 走 `unknown_fallback` 是更安全的默认。

原先把 `OWNER/TYPE/NOUN/LEVE/NAPP/STYP` 放在本节过于激进：`OWNER` 改变世界变换/层级，`TYPE/NOUN/STYP` 改变生成分派，`LEVE/NAPP` 改变参与或显示语义，均应在“宁多勿漏”的生成器策略中保留。`OBST` 不改网格形状，但影响碰撞/参与范围，也属于模型输出策略而非纯元数据。

`TYPEX`（`0xCC6B3F`）在 core 只有两处立即数命中：`DB_Element::checkForUnknownAtts`（`0x5928cf0`）把它列入可容忍的内部/扩展属性；`RCF_Output::outPutSpecificAtt`（`sub_5A99370`，比较点 `0x5a99525`）遇到它直接跳到成功返回，不走普通属性序列化。Core3D 的立即数、符号和 import 扫描均未找到静态消费者。故它当前应标为 **internal/serialization-suppressed，不能仅凭名字并入几何白名单**；最终仍需活会话读取它的 `DCHC/wnoevt/pseudo/type` 元数据确认。

### 13.4 给 plant-model-gen 增量重算的落地建议

1. 继续用 §13.2 作为 `GEOM_AFFECTING_ATTS` 的保守基线，但模型真相源定义为
   `DCHC/EVALAT`；未知属性/UDA 必须保守触发，不能按“不在白名单”直接跳过。
2. 将 `AffectsModel/KnownNeutral/Unknown` 逐步提升为 effect：
   data-only、transform-only、direct-geometry、dependency-cascade、
   structural-membership、unknown，并在取得 DCHC 后保留原始 code。
3. 波及：owner/成员变化保留旧新两侧；`CATR/SPRE`/noun-scoped `PRTREF`
   及目录闭包变化反向更新所有实例；可克隆属性扩散到副本。
4. 粒度：以 significant owner（如 `EQUI`/子设备/`BRAN`）为重算单位，
   展开 Members/AbsentPrimitives/AncestorDeletes，而非只算单图元。
5. noun 名单只负责把最终目标路由到具体生成器，不能提前丢弃目录/规格等依赖源。

`model_impact.rs::attribute_affects_model` 已含 `CATR/SPRE`，并已按本轮“宁多勿漏”
把 `PRTREF` 加入全局 allowlist（2026-07-23）。现有 API 只有 attr、没有 noun；长期改成
`(noun, attr, effect, raw_dchc)`，把 `SPCO/TABITE` 特例及 `dependency-cascade` 与
`direct-mesh` 区分开。

> 提示：§13.2 是“读取即用到 + Core3D 引用级联”的经验并集。若要逼近 Core3D，
> 应以 `DCHC/EVALAT` 动态轨迹为模型影响依据；`wnoevt` 仅用于复刻事件边界。

---

## 14. 与权威属性字典交叉校验（SurrealDB `att_meta`，702 个属性）

对该项目（`AvevaMarineSample`，ns=`1516`）运行中的 SurrealDB（`127.0.0.1:8020`，SurrealDB 3.3）`att_meta` 表做交叉校验。`att_meta` = **702 个属性**，字段 `id`(=`att_meta:<名>`)、`hash`(dabacon 哈希)、`meta_cn_name`。

### 14.1 §13.2 校验结果：全部命中（附 dabacon 哈希），仅剔除 2 个派生/误报

§13.2 的几何输入属性**全部是真实 dabacon 属性**（在 702 属性字典里命中，并拿到各自 hash），仅 2 个剔除：
- `XDIR`：X 轴由 `ORI`+`YDIR`/`ZDIR` 派生，字典无此独立属性（`YDIR`=`0xD9E0D`、`ZDIR`=`0xD9E0E` 存在，`XDIR` 不在）。
- `RAD`：真实属性是 `RADI`(`0xADB7D`)，`RAD` 系聚合误报。

命中示例（可直接建 `GEOM_AFFECTING_ATTS` 哈希表）：`POS`=`0x853B1`、`ORI`=`0x83787`、`CATR`=`0xDBCF9`、`SPRE`=`0x9D165`、`PRTREF`=`0x557F908`、`DESP`=`0xD20C7`、`PARA`=`0x89C41`、`HEIG`=`0xA5056`、`DIAM`=`0xC0748`、`JUSL`=`0xBEEF1`、`PDIA`=`0x882F1`、`ARRI`=`0xB0515`、`LEAV`=`0xEBADF`…（全部 hash 可从 `att_meta` 直接 `SELECT name, hash` 取。）

### 14.2 关于 `wnoevt` / `DCHC` 全量清单（重要结论）

- **`wnoevt` 与 `DCHC` 都不在已同步的 SurrealDB 里**：`att_meta` 只有
  `hash`+`meta_cn_name`（702 条），无事件位或设计变化 code；
  `dicvir.dat`(0.27MB) 是版本戳、非字典。
- **`wnoevt` 是 E3D 内核 dabacon 字典的每属性标志**（运行时 `DB_Attribute` 偏移 184 / dabacon 字段 `299311034`，§10.1）。内建属性字典编译在内核里、不随模型库同步 → **静态无法直接导出**。
- **`DCHC` 是 Core3D 使用的每属性设计变化 code**（字段 `596407`，由
  `IDCHNG` 读取）；它比 `wnoevt=false` 清单更接近“是否进入模型/图形变化队列”。
- **要拿全量权威数据，需其一**：
  1. **活 E3D 会话导出**：遍历 702 属性，同时 dump
     `wnoevt/DCHC/PLCF/isPseudo/casc`；
  2. **扩展字典导入工具**：把字段 `299311034/596407/652066` 落入
     `att_meta.wnoevt/dchc/plcf`；
  3. **动态消费轨迹**：记录 `(noun, attr, DCHC, QCHGLS ref/code)`，覆盖 EVALAT
     的强制 code 与引用/owner 重定向；
  4. **静态替代基线**：枚举 Core3D 实际引用的属性哈希并与 §13.2 对齐。

> 交叉校验结论：§13.2 清单在属性名/哈希字典中 100% 命中（除 2 个派生项），可作为
> 保守生成器基线；它没有因此自动获得 DCHC 真相源地位。`wnoevt` 用于事件兼容，
> `DCHC + EVALAT/QCHGLS` 用于模型影响验证。

---

## 15. DCHC 变化码 1..4 的语义与 EVALAT 传播规则（2026-07-24 深挖，填补 §9「未知」）★

> 本节直接反编译 `Core3D.dll` 的 `IDCHNG / EVALAT / EVALCD / EVALST / FNDTOP / ADSTCH`
> （session `core3d-retrace`），把此前标注「DCHC=1..4 官方枚举名未知」的关口补成
> **可操作语义**。结论：DCHC 的整数值本身在二进制里没有枚举名（DDL 数据里只是整数），
> 但它在 `EVALAT` 里被当作**「路由/作用域选择器」**消费，各值的行为已完整还原。

### 15.1 code 从哪来：`IDCHNG` + `EVALAT` 的强制规则

`EVALAT @0x1022c679` 起始处决定变化码 `v16`（`a4=&attrHash`，`a3=&nounHash`，`a2=elementRef`）：

```c
if      (*a4 == 331445106 /*REDRAW*/) v16 = 4;   // 强制 code 4
else if (*a4 ==  73767168 /*INTUBE*/) v16 = 1;   // 强制 code 1
else                                  v16 = IDCHNG(a4);  // 读 DCHC
if (v16) { /* 仅当非 0 才进入路由/传播 */ }
```

`IDCHNG @0x1022e302` 只做字典查询（实证反编译）：

```c
int IDCHNG(attrHash){
    v = ATAINT(attrHash, DCHC/*596407*/);   // 读设计变化码
    if (lookup_error) return 0;             // 查不到 → 0（等价 NoChange）
    if (ATAINT(attrHash, PLCF/*652066*/)==1) dword_10E98500 = -1; // plot/clash 删除挂起
    return v;                               // 原样返回 DCHC 整数
}
```

即 **DCHC 原值 = EVALAT 的起始 code**，仅 `REDRAW→4`、`INTUBE→1` 两个属性会绕过字典强制取值。整个 EVALAT 只显式比较 `1/2/3/4`，其余非零值会落到通用传播尾部。

### 15.2 各 code 的作用域路由（EVALAT 控制流实证）

| code | EVALAT 行为（实证） | 语义（PDMS） | 最终写入 QCHGLS 的 code |
|---|---|---|---|
| **0** | `if(v16)` 不成立，EVALAT 直接返回 | **NoChange**：该属性改动**不进** QCHGLS（数据-only） | —（不写） |
| **1** | `DGOTO(el)`；INTUBE 另做 `CRETUR+NATTA`；`DGETF(REF=535968)→v48`；`v16=4`；`EVALCD(v48,4)` | **重定向到关联/被引用元素**（改在引用目标上生效，而非属性持有者本身） | 4 |
| **2** | `v16=4`；`EVALCD(self,4)` | **自身重建**（只重算该元素） | 4 |
| **3** | 进入 component/point/owner/ref 通用传播体（与 4 同一段 `(v16==3||v16==4)`） | **传播**：自身 + 组件/点/owner/引用依赖 | 3 或 4（多数分支置 4） |
| **4** | 与 3 完全相同的传播体；且是 REDRAW 级强制值、1/2/3 归一后的终值 | **强制全量传播/重绘** | 4 |

要点：

- **DCHC 是「路由」而非「严重度标量」**：1=去关联对象、2=去自身、3/4=自身+依赖闭包。它只在 EVALAT 内部决定**哪些 ref 被写入 QCHGLS**。
- **3 与 4 在 EVALAT 里行为等价**（同一 `(v16==3||v16==4)` 分支）；4 只是 REDRAW 级/归一后的规范终值，并在去重时胜出（见 §15.3）。
- 下游 `PartialUpdateDesiMgr::ChangedModelToUpdate` 传 `ModelState=0`（§11.3），**不消费 QCHGLS 里存的 code 值**——DCHC 的数值影响仅体现在「选了哪些 ref」，不体现在下游粒度状态。

### 15.3 code 如何落库：`EVALCD → EVALST`（QCHGLS 三元组 + 保留最大 code）

```c
// EVALCD @0x1022e020：仅是包装，写全局 QCHGLS 句柄 dword_10E98540
EVALCD(ref, &code){ EVALST(&QCHGLS/*dword_10E98540*/, ref, &code); }

// EVALST @0x1022e0a7：按 ref 去重、保留较大 code
EVALST(list, ref, &code){
    if (NULREF(ref)) return;
    for (i=1; i<len(list); i+=3)              // 步长 3：ref_hi, ref_lo, code
        if (EQREF(ref, list[i])) {            // ref 已存在
            if (list[i+2] < code) list[i+2]=code;  // 只在更大时覆盖
            return;
        }
    append(list, ref_hi, ref_lo, code);       // 不存在 → 追加三元组
}
```

证实 §4.3/§5.3 的旧结论：**QCHGLS = `(ref[2], changeCode)` 列表，按 ref 去重、保留最大 code**。

### 15.4 EVALAT 的 (noun) 专例传播（DEHASH 全部解出）

`v16` 非零后，EVALAT 按 `*a3`(nounHash) 走一批硬编码专例；除标注外均把命中目标以 code 4 写入 QCHGLS：

| 专例分支 | 机制 | 命中 noun（DEHASH 还原） |
|---|---|---|
| owner 上溯 `FNDTOP`(`sub_10380E38`) | 上卷到显著 owner，强制 code 4 | `FLRLAY STRTWR WLOPEN HRGATE RAIL KICKPL HRPOST BPOPEN AIDLIN AIDARC AIDCIR AIDPOI AIDTEX` |
| 成员遍历 `CGETEL/XTREE` | 展开子成员并入队 | `GRIDPL GRIDCY GRIDEL GRIDFA`（子项 `GRIDPL/GRIDCY`） |
| 引用遍历 `DFIND/DGETI` | 沿引用集扩散 | 容器 `CGRDCP CGRDLP`；命中项 `FPFITT ELFITT HVACFI INFITT`（各专业 fitting） |
| 引用遍历 | 同上 | `CTRAY`（命中 `HVACFI`）、`CLNPNG→CLNCGR→CLNTIL`（洁净室嵌套） |
| `GATREF` 反查 | 目录/引用反查入队 | `PLTGRD/INTFRM`(`CWBRAN/POINTR`)、`SUBCOM`(`EQUI/ELCONN`) |
| component 判定 `INCOMP/IPCOMP/IHCOMP` | 组件→自身 code 4 + 目录字段 | `PLTGRD INTFRM SUBCOM`；引用集 `PLOPEN DPCA DPCY DPSP DPSE`、`POHE`、`PANE/TMPL` |

`FNDTOP`（trace `desdblib/FNDTOP`）沿 owner 链上卷，边界 noun 为 `TUBI(710633)/BRAN(808220)/WORL(781187)/TMPL(779672)`——即 Core3D 的「显著几何容器」判定；`ADSTCH`（`sub_1022D774`，trace `change/ADSTCH`）是通用「关联结构变化」展开器（`CLIMBA`+`XTREE` 遍历关系字段再入队）。

### 15.5 官方枚举名的可得性

在 `core.dll` 中检索仅得到诊断串 `"DCHC = "`（`0x5d4c0b8`）与导出符号
`?dchc@DB_Attribute@@QBEHXZ`（`0x5ec8bdf`）；**没有 1..4 的枚举名字符串/枚举表**。
DCHC 的取值是 DDL 字典里的裸整数，故其**枚举名无法从二进制静态恢复**；§15.2 的
「路由语义」即为可得的权威还原。

### 15.6 对 plant-model-gen 的直接启示（升级 `AttributeModelImpact`）

当前 `attribute_affects_model` 是**扁平布尔白名单**，把 DCHC 的「路由维度」压平了。据 §15.2 可把三态升级为**带路由的 effect**（Rust 无法静态读每属性 DCHC，故保留 inclusive 兜底，但可表达路由）：

| DCHC | 建议 Rust effect | 目标选择 |
|---|---|---|
| 0 | `KnownNeutral`（data-only） | 不写欠账 |
| 1 | `RedirectToRelated`（owner/ref） | 入队**关联/owner**而非自身 |
| 2 | `SelfGeometry`（direct-geometry） | 入队自身 |
| 3/4 | `PropagateClosure`（dependency-cascade） | 自身 + 成员/引用/owner 闭包 |

价值点：现实现对 `INTUBE`/`CATR`/`SPRE`/克隆等**「改在 A、模型欠账应记在 B」**的情形，只把 A 自身入桶；对齐 DCHC 路由后应把欠账**重定向/扩散**到 owner、被引用实例与显著容器（呼应 §13.4、ADR-0009、ADR-0011）。

### 15.7 关键地址/字段速查（本轮新增）

| 符号 | 地址(core3d) | 作用 |
|---|---|---|
| `IDCHNG` | `0x1022e302` | 读 DCHC(596407)/PLCF(652066)，返回变化码 |
| `EVALAT` | `0x1022c679` | 按 code 1..4 路由 + noun 专例传播 |
| `EVALCD` | `0x1022e020` | 写全局 QCHGLS(`dword_10E98540`) 的包装 |
| `EVALST` | `0x1022e0a7` | QCHGLS 三元组去重 + 保留最大 code |
| `FNDTOP`(`sub_10380E38`) | `0x10380e38` | 显著 owner 上溯（边界 TUBI/BRAN/WORL/TMPL） |
| `ADSTCH`(`sub_1022D774`) | `0x1022d774` | 关联结构变化展开器 |
| `plcDeletePending` | `dword_10E98500` | PLCF==1 时置位，`UPDATD` 开头 `PLCDEL` |
| `QCHGLS` | `dword_10E98540` | 全局设计变化列表句柄 |

> DEHASH 本轮新验证：`DCHC=596407`、`PLCF=652066`、`REF=535968`、`REDRAW=331445106`、
> `INTUBE=73767168`、`TUBI=710633`、`BRAN=808220`、`WORL=781187`、`TMPL=779672`。

### 14.3 消费方（Core3D）反向交叉校验：§13.2 基本完备

通用路径已经确认 `VDESPT → EVALAT → IDCHNG(DCHC)`。此外，用已加载的
`Core3D` 会话把 VDESPT 与 DRAFT 分派中的**硬编码比较常量**全部解码，并用
`att_meta`（702）判定“是属性还是 noun”：

- 这些常量**大多是 NOUN 哈希**（分派按 (noun, attr) 键控）：3D 侧 `PLOO`/`SJOI`/`COCO`/`DATA`；DRAFT 侧 `SHEE`/`VIEW`/`LAYE`/`OVER`/`GRIDNM`/`MSTYLE`… 均为元素类型（noun），非属性。
- 真正是**属性**且被 Core3D 设计代码引用的：`HEIG`/`PPRO`/`DPRO`/`POS`/`PKEY`/`PKDI`（已在 §13.2）+ 3 个新增：
  - **`CTYP`**（组件类型，与 noun `COCO` 联动）、**`JFRE`**（与 noun `SJOI` 联动）→ **几何相关，已补入 §13.2（B/F）**。
  - `LOCK`（DRAFT 锁定/状态标志）→ **非几何**（不纳入）。
- 该结论只覆盖**标量 `PostSetAttribute`**。第二轮继续扫描 `PostSetRefAttribute` 与目录解析器，新增确认：
  - `CATR`/`SPRE`：`GATCAT/GATCRF` 的主目录解析链，同时被 core comparison-session 用于递归目录依赖；
  - **`PRTREF`**：`SPCO.PRTREF` 在 `VDESPF` 有显式派生属性回写，`TABITE` 在 `GATCAT/GATCRF` 以它作为目录跳板；已补入 §13.2（B）；
  - `TYPEX`：core 仅用于 unknown-att 容忍，并在 RCF 输出时显式跳过；Core3D 无静态消费者，暂不纳入。
- **修正后的结论**：加入 `PRTREF` 后，§13.2 对当前已见 3D 标量特例、引用级联
  和目录解析路径覆盖较完整；但通用入队依据仍是 DCHC/EVALAT，清单不能替代原始
  change code。进入 QCHGLS 后再按 significant-owner 整块重算。
