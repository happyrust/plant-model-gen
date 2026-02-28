# syntax=docker/dockerfile:1.4
#
# 构建命令（在 plant-code/ 目录下执行）：
#   DOCKER_BUILDKIT=1 docker build \
#     -f gen-model-fork/Dockerfile \
#     -t aios-web-server:latest \
#     .
#
# Build context 目录结构（parent: plant-code/）：
#   plant-code/
#   ├── gen-model-fork/     ← 主项目
#   ├── rs-core/            ← path 依赖 aios_core
#   └── pdms-io-fork/       ← path 依赖 pdms_io / parse_pdms_db
#
# 缓存策略：
#   - cargo-chef  → 将依赖编译独立成 Docker layer，源码改动不会重编依赖
#   - BuildKit cache mount → 缓存 crates.io 注册表 & git 拉取，避免每次重新下载
#   - lld 链接器  → 替换默认 ld，显著缩短链接时间

# ═════════════════════════════════════════════════════════════════════════════
# Base: cargo-chef 镜像 + 系统构建依赖
# ═════════════════════════════════════════════════════════════════════════════
FROM lukemathwalker/cargo-chef:latest-rust-1.83.0-slim AS chef

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    build-essential \
    cmake \
    g++ \
    git \
    lld \
    && rm -rf /var/lib/apt/lists/*

# 使用 lld 加速链接。
# 注意：不拷贝本地 .cargo/config.toml，其中的 [unstable]/cranelift 设置
# 仅适用于 nightly，会导致 stable Rust 构建失败。
ENV RUSTFLAGS="-C link-arg=-fuse-ld=lld"

# 统一 WORKDIR 根目录，使 path = "../rs-core" 相对路径在容器内与本地一致
WORKDIR /build

# ═════════════════════════════════════════════════════════════════════════════
# Stage 1 — planner: 分析完整依赖图，生成 recipe.json
#   recipe.json 只记录依赖清单（类似 Cargo.lock 的子集），不含应用源码。
#   只有 Cargo.toml / Cargo.lock 变化时此 layer 才会失效。
# ═════════════════════════════════════════════════════════════════════════════
FROM chef AS planner

COPY rs-core/        ./rs-core/
COPY pdms-io-fork/   ./pdms-io-fork/
COPY gen-model-fork/ ./gen-model-fork/

WORKDIR /build/gen-model-fork
RUN cargo chef prepare --recipe-path recipe.json

# ═════════════════════════════════════════════════════════════════════════════
# Stage 2 — cacher: 仅编译外部依赖（Docker layer 缓存的核心）
#
#   缓存命中条件：recipe.json 未变（即 Cargo.toml/Cargo.lock 未变）
#   BuildKit cache mount（id=cargo-*）跨构建持久化，避免重复下载：
#     - cargo-registry: crates.io 包缓存
#     - cargo-git:      git 仓库缓存（包含 surrealdb@gitee）
# ═════════════════════════════════════════════════════════════════════════════
FROM chef AS cacher

COPY --from=planner /build/gen-model-fork/recipe.json ./gen-model-fork/recipe.json

# cargo chef cook 需要 path 依赖的完整源码才能编译它们
COPY rs-core/      ./rs-core/
COPY pdms-io-fork/ ./pdms-io-fork/

WORKDIR /build/gen-model-fork

RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    cargo chef cook --release \
        --no-default-features \
        --features ws,sqlite-index,surreal-save,web_server \
        --recipe-path recipe.json

# ═════════════════════════════════════════════════════════════════════════════
# Stage 3 — builder: 仅编译应用代码
#   依赖 .rlib 已由 cacher 预编译并通过 COPY --from=cacher 注入，
#   此阶段只重新编译变化的应用代码，通常 < 1 分钟。
# ═════════════════════════════════════════════════════════════════════════════
FROM chef AS builder

COPY rs-core/        ./rs-core/
COPY pdms-io-fork/   ./pdms-io-fork/
COPY gen-model-fork/ ./gen-model-fork/

# 还原已编译的依赖产物（.rlib / .rmeta）
COPY --from=cacher /build/gen-model-fork/target ./gen-model-fork/target

WORKDIR /build/gen-model-fork

RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    cargo build --release --bin web_server \
        --no-default-features \
        --features ws,sqlite-index,surreal-save,web_server

# ═════════════════════════════════════════════════════════════════════════════
# Stage 4 — runtime: 最小运行镜像
# ═════════════════════════════════════════════════════════════════════════════
FROM debian:12-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    sqlite3 \
    && rm -rf /var/lib/apt/lists/*

RUN useradd -r -s /bin/false appuser

WORKDIR /app

COPY --from=builder /build/gen-model-fork/target/release/web_server /usr/local/bin/web_server
COPY --from=builder /build/gen-model-fork/db_options/DbOption.toml  /app/db_options/

# 可选静态资源目录（不存在时跳过，不阻断构建）
RUN --mount=from=builder,source=/build/gen-model-fork,target=/src \
    for d in assets data web-test; do \
        [ -d "/src/$d" ] && cp -r "/src/$d" /app/ || true; \
    done

RUN chown -R appuser:appuser /app && \
    chmod +x /usr/local/bin/web_server

USER appuser

EXPOSE 8080

HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1

CMD ["web_server"]
