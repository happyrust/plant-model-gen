# 批注交互流程说明

## 目标

说明校审/审理场景下，三维视图中的批注创建、编辑、意见录入与确认的完整交互链路，便于排查和回归验证。

## 参与模块

- `src/components/review/ReviewPanel.vue`：审理入口，负责开启批注模式
- `src/components/dock_panels/ViewerPanel.vue`：接管三维画布交互并分发到 tools
- `src/composables/useDtxTools.ts`：处理 mesh 点击、拖拽框选、批注创建
- `src/composables/useToolStore.ts`：保存批注、激活项、待编辑状态
- `src/components/tools/AnnotationPanel.vue`：批注列表、标题/描述编辑、首次弹窗编辑
- `src/components/review/ReviewCommentsPanel.vue`：按角色录入和查看意见
- `src/components/review/ReviewConfirmation.vue`：校审模式下待确认数据的确认入口

## 文字批注交互流程

1. 用户在 `ReviewPanel` 点击“待确认数据 > 批注 > +”
2. `startAnnotation()` 会：
   - 激活 `annotation` 面板
   - 将 `toolMode` 切换为 `annotation`
3. `ViewerPanel` 检测到工具模式非 `none` 后，将画布点击交给 `useDtxTools`
4. 用户点击模型 `mesh`
5. `useDtxTools.onCanvasPointerUp()` 在 `mode === 'annotation'` 时执行 `pickSurfacePoint()`
6. 命中模型表面后创建 `AnnotationRecord`，写入：
   - `entityId`
   - `worldPos`
   - `glyph`
   - `title`
   - `description`
   - `refno`
7. `useToolStore.addAnnotation()` 会：
   - 追加到 `annotations`
   - 设置 `activeAnnotationId`
   - 设置 `pendingTextAnnotationEditId`
8. `AnnotationPanel` 监听 `pendingTextAnnotationEditId`，弹出“编辑文字批注”对话框
9. 用户填写标题/描述并确认后，批注进入可继续编辑/添加意见的状态

## 其他批注类型

- 云线批注：`annotation_cloud`，通过拖拽框选生成
- 矩形批注：`annotation_rect`，通过两点对角拖拽生成
- OBB 批注：`annotation_obb`，通过框选对象集合生成

三类批注创建后均进入 `AnnotationPanel` 统一管理；其中 OBB 也使用待编辑状态触发首次编辑弹窗。

## 意见录入流程

1. 用户在 `AnnotationPanel` 中选中一个批注
2. 面板下方“意见管理”区域根据当前选中的批注类型和 ID 加载评论
3. `ReviewCommentsPanel` 按角色分三栏展示：
   - 设计
   - 校对
   - 审核
4. 当前登录用户只能在其角色对应栏目添加意见
5. 添加、编辑、删除意见均通过 `useToolStore` 的评论函数完成

## 校审确认流程

1. 校审模式开启且存在待确认批注/测量时，`ReviewConfirmation` 显示浮层
2. 用户点击“确认完成”后，当前待确认的批注和测量写入 `ConfirmedRecord`
3. `toolStore.clearAll()` 清空待确认数据，当前轮交互结束

## 本次修复点

### 问题

此前存在“先创建批注，后挂载批注面板”的时序问题：

- `pendingTextAnnotationEditId` 已在 store 中写入
- `AnnotationPanel` 此时尚未挂载
- 面板挂载后 `watch` 不会处理已有值
- 结果是点击 mesh 后批注实际已创建，但首次编辑弹窗不出现，用户感知为“无法使用”

### 修复

1. `ReviewPanel.startAnnotation()` 先激活 `annotation` 面板
2. `AnnotationPanel` 对待编辑状态的监听改为 `immediate: true`

这样无论面板是在创建前还是创建后挂载，都能进入首次编辑状态。

## 回归验证建议

### 核心链路

1. 打开校审面板
2. 点击“批注 +”
3. 确认批注面板被激活
4. 点击模型 mesh
5. 确认出现“编辑文字批注”弹窗
6. 填写标题/描述并保存
7. 在意见管理中按当前角色提交一条意见

### 边界场景

- 批注面板原本关闭时，首次创建是否仍能弹窗
- 已有待确认批注时，右下角确认浮层是否正常显示
- 切换到云线/矩形/OBB 批注后，原有文字批注流程不受影响
- 不同角色用户是否只能在对应栏目提交意见