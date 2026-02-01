#!/bin/bash
# Lsky-Pro 一键部署脚本
# 在服务器上执行: curl -sSL https://raw.githubusercontent.com/... | bash
# 或者直接复制此脚本到服务器执行

set -e

echo "=========================================="
echo "  Lsky-Pro 图床一键部署"
echo "=========================================="

# 创建目录
INSTALL_DIR="/opt/lsky-pro"
mkdir -p $INSTALL_DIR
cd $INSTALL_DIR

# 检查 Docker
if ! command -v docker &> /dev/null; then
    echo "[INFO] Docker 未安装，正在安装..."
    curl -fsSL https://get.docker.com | sh
    systemctl start docker
    systemctl enable docker
fi

# 检查 docker-compose
if ! command -v docker-compose &> /dev/null; then
    echo "[INFO] docker-compose 未安装，正在安装..."
    curl -L "https://github.com/docker/compose/releases/latest/download/docker-compose-$(uname -s)-$(uname -m)" -o /usr/local/bin/docker-compose
    chmod +x /usr/local/bin/docker-compose
fi

echo "[INFO] Docker 版本: $(docker --version)"
echo "[INFO] Docker-Compose 版本: $(docker-compose --version)"

# 创建 docker-compose.yml
cat > docker-compose.yml << 'EOF'
version: '3.8'

services:
  lsky-pro:
    image: halcyonazure/lsky-pro-docker:latest
    container_name: lsky-pro
    restart: unless-stopped
    ports:
      - "8089:8089"
    volumes:
      - ./data:/var/www/html/storage
    environment:
      - TZ=Asia/Shanghai
EOF

echo "[INFO] 配置文件已创建"

# 拉取镜像并启动
echo "[INFO] 拉取 Lsky-Pro 镜像..."
docker-compose pull

echo "[INFO] 启动 Lsky-Pro..."
docker-compose up -d

# 等待服务启动
echo "[INFO] 等待服务启动..."
sleep 5

# 检查状态
if docker ps | grep -q lsky-pro; then
    echo ""
    echo "=========================================="
    echo "  部署成功!"
    echo "=========================================="
    echo ""
    echo "访问地址: http://$(curl -s ifconfig.me):8089"
    echo ""
    echo "下一步:"
    echo "1. 访问上面的地址完成初始化"
    echo "2. 设置管理员账号"
    echo "3. 在后台 -> 设置 -> 接口 -> 生成 API Token"
    echo ""
else
    echo "[ERROR] 启动失败，请检查日志:"
    docker-compose logs
fi
