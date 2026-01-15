# 房间计算使用指南

## 概述

房间计算功能用于构建房间与构件之间的空间关系，支持通过 CLI 命令和 API 两种方式调用。

## CLI 命令

### 基本用法

```bash
# 使用默认配置运行房间计算
cargo run --release --features sqlite-index -- --room-compute

# 指定房间关键词
cargo run --release --features sqlite-index -- --room-compute --room-keywords "-RM,-ROOM"

# 指定数据库编号
cargo run --release --features sqlite-index -- --room-compute --room-db-nums 1112,1113

# 强制重建所有房间关系
cargo run --release --features sqlite-index -- --room-compute --room-force-rebuild

# 组合使用
cargo run --release --features sqlite-index -- \
  --room-compute \
  --room-keywords "-RM,-ROOM" \
  --room-db-nums 1112 \
  --room-force-rebuild \
  --verbose
```

### CLI 参数说明

| 参数 | 说明 | 默认值 |
|------|------|--------|
| `--room-compute` | 启用房间计算模式 | - |
| `--room-keywords` | 房间关键词（逗号分隔） | 配置文件中的 `room_key_word` |
| `--room-db-nums` | 数据库编号（逗号分隔） | 全部 |
| `--room-force-rebuild` | 强制重建所有关系 | false |
| `--verbose` | 详细输出 | false |

## API 接口

### 同步房间计算

直接执行房间计算，等待完成后返回结果。

**端点**: `POST /api/room/compute`

**请求体**:
```json
{
  "room_keywords": ["-RM", "-ROOM"],
  "db_nums": [1112, 1113],
  "force_rebuild": false
}
```

**响应**:
```json
{
  "success": true,
  "message": "房间计算完成，处理了 150 个房间",
  "total_rooms": 150,
  "total_panels": 450,
  "total_components": 12000,
  "build_time_ms": 5230,
  "cache_hit_rate": 0.85
}
```

### 异步房间计算

创建异步任务，通过任务 ID 查询进度。

**端点**: `POST /api/room/rebuild-relations`

**请求体**:
```json
{
  "room_numbers": ["RM-001", "RM-002"],
  "force_rebuild": true
}
```

**响应**:
```json
{
  "success": true,
  "task_id": "550e8400-e29b-41d4-a716-446655440000",
  "message": "房间关系重建任务已创建"
}
```

### 查询任务状态

**端点**: `GET /api/room/tasks/{task_id}`

## 配置文件

在 `DbOption.toml` 中配置房间计算相关参数：

```toml
# 房间关键词（用于识别房间面板）
room_key_word = ["-RM", "-ROOM", "-房间"]

# 网格文件路径
meshes_path = "assets/meshes"
```

## 输出说明

房间计算完成后会输出以下统计信息：

- **处理房间数**: 成功处理的房间数量
- **处理面板数**: 处理的房间面板数量
- **处理构件数**: 建立关系的构件数量
- **构建耗时**: 计算耗时（毫秒）
- **缓存命中率**: 几何缓存命中率
- **内存使用**: 峰值内存使用量

## 注意事项

1. 房间计算需要 `sqlite-index` 特性支持
2. 首次运行建议使用 `--room-force-rebuild` 确保数据完整
3. 大型项目建议分批处理，使用 `--room-db-nums` 指定数据库
4. 计算过程中会自动使用几何缓存提升性能
