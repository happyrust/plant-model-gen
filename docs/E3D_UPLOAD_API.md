# E3D 远程上传和解析 API 文档

## 概述

本 API 提供远程上传 E3D 文件并自动触发数据解析和模型生成的完整功能。

## API 端点

### 1. 上传 E3D 文件

**端点**: `POST /api/upload/e3d`

**请求格式**: `multipart/form-data`

**参数**:
- `file` (必需): E3D 文件
- `project_name` (可选): 项目名称，默认使用配置文件中的项目名

**响应示例**:
```json
{
  "success": true,
  "task_id": "550e8400-e29b-41d4-a716-446655440000",
  "message": "文件上传成功，开始解析"
}
```

### 2. 查询任务状态

**端点**: `GET /api/upload/task/{task_id}`

**响应示例**:
```json
{
  "success": true,
  "task": {
    "task_id": "550e8400-e29b-41d4-a716-446655440000",
    "filename": "sample.e3d",
    "status": "parsing",
    "progress": 45.0,
    "message": "正在解析 E3D 文件",
    "created_at": "2026-03-11T15:55:51+08:00",
    "project_name": "test_project"
  },
  "error_message": null
}
```

**任务状态**:
- `uploading`: 文件上传中
- `parsing`: 数据解析中
- `completed`: 解析完成
- `failed`: 解析失败

## 使用示例

### cURL 示例

```bash
# 1. 上传文件
curl -X POST http://localhost:8080/api/upload/e3d \
  -F "file=@/path/to/your/file.e3d" \
  -F "project_name=my_project"

# 2. 查询状态
curl http://localhost:8080/api/upload/task/{task_id}
```

### Python 示例

```python
import requests

# 上传文件
with open('sample.e3d', 'rb') as f:
    response = requests.post(
        'http://localhost:8080/api/upload/e3d',
        files={'file': f},
        data={'project_name': 'my_project'}
    )
    task_id = response.json()['task_id']

# 查询状态
status = requests.get(f'http://localhost:8080/api/upload/task/{task_id}')
print(status.json())
```

## 测试脚本

项目提供了两个测试脚本：

### Bash 脚本
```bash
./test_upload_e3d.sh /path/to/file.e3d project_name
```

### Python 脚本
```bash
python test_upload_e3d.py /path/to/file.e3d project_name
```

## 完整测试流程

1. **启动 Web 服务器**
```bash
cargo run --bin web_server --features web_server
```

2. **上传 E3D 文件**
```bash
python test_upload_e3d.py test_data/sample.e3d my_project
```

3. **查询解析结果**
   - 脚本会自动轮询任务状态
   - 解析完成后自动测试数据查询 API

4. **验证数据**
```bash
# 查询 World Root
curl http://localhost:8080/api/e3d/world-root

# 查询节点信息
curl http://localhost:8080/api/e3d/node/{refno}
```

## 文件存储

- 上传的文件存储在 `uploads/` 目录
- 解析后的数据存储在 SurrealDB
- 生成的模型文件在 `output/{project_name}/` 目录

## 错误处理

常见错误及解决方案：

1. **文件上传失败**
   - 检查文件大小限制
   - 确认文件格式正确

2. **解析失败**
   - 查看任务状态中的错误信息
   - 检查 SurrealDB 连接状态
   - 确认配置文件正确

3. **超时**
   - 大文件解析需要更长时间
   - 可调整测试脚本中的 `max_attempts` 参数

## 后续扩展

可基于此 API 实现：
- Web 前端上传界面
- 批量文件上传
- 解析进度实时推送（WebSocket/SSE）
- 模型导出 API 集成
