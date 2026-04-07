# 防重复确认保存：实现幂等 upsert 与前端状态分离

## 问题描述

在校审/提资流程中，确认按钮可以无限次点击，每次都会向数据库写入一条新的确认记录，导致：

1. **数据无限膨胀** - 同一任务同一操作员可以产生任意数量的重复记录
2. **历史混乱** - 无法区分哪些是"真正的新确认"，哪些是"重复点击"
3. **体验差** - 用户不知道是否已经保存成功，反复点击确认

### 复现步骤

1. 打开校审面板，添加批注或测量
2. 点击"确认当前数据"按钮
3. 不做任何修改，再次点击确认按钮
4. 重复步骤3多次
5. 查询数据库，发现同一任务同一操作员产生了多条完全相同的记录

### 期望行为

| 场景 | 期望结果 |
|------|----------|
| 首次确认 | 创建新记录，返回成功 |
| 重复确认（内容未变） | 返回已有记录，不新增，不更新时间戳 |
| 修改后再次确认 | 更新原有记录内容，刷新确认时间 |
| 删除所有批注后确认 | 允许确认空状态（视为有效变更） |

### 实际行为

每次点击都创建新记录，数据库中产生重复数据：

```
review_records 表（修复前）:
├─ record-uuid-1: task-123, operator-abc, note="检查完成", confirmed_at=10:00:00
├─ record-uuid-2: task-123, operator-abc, note="检查完成", confirmed_at=10:00:05  ← 重复！
├─ record-uuid-3: task-123, operator-abc, note="检查完成", confirmed_at=10:00:12  ← 又一条！
└─ ...
```

## 解决方案

### 核心思路

1. **稳定槽位键（Slot Key）** - 用 `form_id + current_node + operator_id` 确定唯一记录位置
2. **内容快照哈希** - 对确认内容计算 SHA256，用于检测是否真的发生变化
3. **幂等 Upsert** - 后端实现 `UPSERT` 语义：存在则更新，不存在则创建，内容相同则 no-op
4. **前端状态分离** - 区分"已确认状态"与"当前草稿"，用快照对比检测未保存变更

### 实现细节

#### 后端 (`review_api.rs`)

**稳定记录 ID 生成**（基于槽位键）：
```rust
fn build_confirmed_record_stable_id(slot_key: &str) -> String {
    let hash = sha256(slot_key);
    format!("slot-{}", hash)
}
```

**内容快照哈希**（用于检测变更）：
```rust
fn build_confirmed_record_snapshot_hash(
    record_type: &str,
    annotations: &[Value],
    cloud_annotations: &[Value],
    rect_annotations: &[Value],
    obb_annotations: &[Value],
    measurements: &[Value],
    note: &str,
) -> String {
    let normalized = json!({
        "type": record_type,
        "annotations": annotations,
        "cloud": cloud_annotations,
        "rect": rect_annotations,
        "obb": obb_annotations,
        "measurements": measurements,
        "note": note.trim(),
    });
    sha256(&normalized.to_string())
}
```

**Upsert 逻辑**：
```rust
// 1. 查询是否已有记录
let existing = query("SELECT * FROM review_records WHERE record::id(id) = $id").await?;

// 2. 存在且哈希相同 → no-op（仅刷新上下文字段）
if existing.snapshot_hash == new_snapshot_hash {
    return Ok(existing_record);  // 不更新 confirmed_at
}

// 3. 不存在或哈希不同 → UPSERT
UPSERT type::record('review_records', $id) CONTENT { ... } RETURN AFTER
```

#### 前端 (`ReviewPanel.vue` / `ReviewConfirmation.vue`)

**快照对比检测未保存变更**：
```typescript
const hasUnsavedChanges = computed(() => {
  return buildReviewConfirmSnapshotKey(currentDraftConfirmPayload.value)
    !== buildReviewConfirmSnapshotKey(confirmedSnapshotPayload.value);
});

const hasUnsavedPendingData = computed(() => hasUnsavedChanges.value);
```

**确认按钮状态**：
```typescript
:disabled="!hasUnsavedPendingData || confirmSaving"
```

**保存成功后更新快照**：
```typescript
if (saved) {
  confirmedSnapshotPayload.value = cloneDeep(currentDraftConfirmPayload.value);
  // ... 提示成功
}
```

### 数据表结构更新

新增字段：
- `slot_key: String` - 稳定槽位标识
- `snapshot_hash: String` - 内容快照哈希
- `form_id: String` - 表单ID（从任务解析）
- `current_node: String` - 当前流程节点（从任务解析）
- `operator_name: String` - 操作员姓名

## 验证结果

### 真实链路测试

在本地 `3110` 端口启动当前源码，执行验证脚本：

```bash
python3 debug_scripts/review_record_dedup/validate_review_record_dedup.py
```

**测试流程**：
1. 创建校审任务
2. 第一次保存（note="v1"）
3. 第二次保存（相同内容）
4. 第三次保存（修改 note="v2"）
5. 查询任务下所有记录

**结果**：

| 断言 | 状态 |
|------|------|
| 同内容重复提交 → 返回同一 record ID | ✅ 通过 |
| 同内容不刷新 confirmed_at | ✅ 通过 |
| 修改内容后提交 → 仍复用同一 record ID | ✅ 通过 |
| 修改内容后提交 → confirmed_at 更新 | ✅ 通过 |
| 任务下最终只有 1 条记录 | ✅ 通过 |

### 验证数据

```json
{
  "taskId": "task-0f8a1c50-b62c-479c-a3f2-ca2ce3a4ab75",
  "recordId": "slot-56ceba10926ce5da98285403ec46083d3c52d1ab9e4c082a6e9311ebdcf8a922",
  "firstConfirmedAt": 1775558871137,
  "secondConfirmedAt": 1775558871137,   // 相同，no-op
  "thirdConfirmedAt": 1775558873668,   // 变大，upsert
  "listCount": 1  // 只有一条记录！
}
```

## 涉及的改动

### 后端文件
- `src/web_api/review_api.rs`
  - 重构 `create_record` → 幂等 upsert 实现
  - 更新 `get_records_by_task` 返回新结构
  - 更新导出逻辑使用 `ReviewRecordRow`

### 前端文件
- `src/components/review/ReviewPanel.vue`
  - 新增 `confirmedSnapshotPayload` 跟踪已确认状态
  - 修改 `hasUnsavedPendingData` 基于快照对比
  - 保存成功后更新快照并提示

- `src/components/review/ReviewConfirmation.vue`
  - 同步确认逻辑
  - 修改 `clearAll` 为 no-op（避免保存后清空场景）
  - 修改 `isVisible` 基于未保存变更

### 调试脚本（新增）
- `debug_scripts/review_record_dedup/validate_review_record_dedup.py`

## 检查清单

- [x] 后端 `cargo check --bin web_server --features web_server` 通过
- [x] 前端 `npm run type-check` 通过
- [x] 真实链路验证：重复提交不新增记录
- [x] 真实链路验证：内容变更更新记录
- [x] 真实链路验证：任务下只有一条当前记录

## 修复后的数据库状态

```
review_records 表（修复后）:
└─ slot-{hash}: task-123, operator-abc, note="检查完成v2", confirmed_at=10:00:12
                                ↑
                        同一槽位始终只有一条记录
```

---

**相关 PR**: （待填写）
**测试脚本**: `debug_scripts/review_record_dedup/validate_review_record_dedup.py`
