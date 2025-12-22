---
description: 启动 gen-model-fork web_server 服务
---
# 启动 Web Server

## 编译并启动（Debug 模式）

```bash
cd /Volumes/DPC/work/plant-code/gen-model-fork
cargo build --bin web_server --features="web_server" && ./target/debug/web_server
```

## 编译并启动（Release 模式）

```bash
cd /Volumes/DPC/work/plant-code/gen-model-fork
cargo build --release --bin web_server --features="web_server" && ./target/release/web_server
```

## 服务器信息

- **访问地址**: http://localhost:8080
- **配置文件**: DbOption.toml
- **依赖服务**: SurrealDB (默认端口 8020)

## 常用 API

- `POST /api/model/show-by-refno` - 按需生成并显示模型
- `POST /api/model/generate-by-refno` - 按 refno 生成模型任务
- `GET /api/tasks` - 获取任务列表
