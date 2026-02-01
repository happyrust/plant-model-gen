#!/usr/bin/env python3
"""
通过 SSH 自动部署 Lsky-Pro 到远程服务器
"""

import paramiko
import time
import sys

# 服务器配置
HOST = "123.57.182.243"
PORT = 22
USERNAME = "root"
PASSWORD = "Happytest123_"

# 部署命令
DEPLOY_COMMANDS = """
# 创建安装目录
mkdir -p /opt/lsky-pro
cd /opt/lsky-pro

# 检查 Docker
if ! command -v docker &> /dev/null; then
    echo "[INFO] Docker 未安装，正在安装..."
    curl -fsSL https://get.docker.com | sh
    systemctl start docker
    systemctl enable docker
fi

# 检查 docker-compose
if ! command -v docker-compose &> /dev/null; then
    echo "[INFO] docker-compose 未安装，尝试使用 docker compose..."
fi

echo "[INFO] Docker 版本:"
docker --version

# 创建 docker-compose.yml
cat > /opt/lsky-pro/docker-compose.yml << 'EOFCOMPOSE'
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
EOFCOMPOSE

echo "[INFO] 配置文件已创建"
cat /opt/lsky-pro/docker-compose.yml

# 拉取镜像并启动
cd /opt/lsky-pro
echo "[INFO] 拉取 Lsky-Pro 镜像..."
docker-compose pull || docker compose pull

echo "[INFO] 启动 Lsky-Pro..."
docker-compose up -d || docker compose up -d

# 等待服务启动
echo "[INFO] 等待服务启动..."
sleep 5

# 检查状态
docker ps | grep lsky-pro

echo ""
echo "=========================================="
echo "  部署完成!"
echo "=========================================="
echo "访问地址: http://123.57.182.243:8089"
"""

def main():
    print(f"连接到服务器 {HOST}...")

    # 创建 SSH 客户端
    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())

    try:
        # 连接
        client.connect(HOST, port=PORT, username=USERNAME, password=PASSWORD, timeout=30)
        print("连接成功!")
        print()

        # 执行部署命令
        print("开始部署 Lsky-Pro...")
        print("=" * 60)

        # 使用 invoke_shell 来执行多行命令
        channel = client.invoke_shell()
        time.sleep(1)

        # 发送命令
        for line in DEPLOY_COMMANDS.strip().split('\n'):
            if line.strip() and not line.strip().startswith('#'):
                channel.send(line + '\n')
                time.sleep(0.5)

        # 发送结束标记
        channel.send('echo "DEPLOY_COMPLETE"\n')

        # 读取输出
        output = ""
        timeout = 300  # 5分钟超时
        start_time = time.time()

        while True:
            if channel.recv_ready():
                chunk = channel.recv(4096).decode('utf-8', errors='ignore')
                output += chunk
                print(chunk, end='', flush=True)

                if "DEPLOY_COMPLETE" in output:
                    break

            if time.time() - start_time > timeout:
                print("\n[WARN] 超时，但部署可能仍在进行...")
                break

            time.sleep(0.5)

        print()
        print("=" * 60)
        print("部署脚本执行完成!")
        print()
        print("下一步:")
        print("1. 访问 http://123.57.182.243:8089")
        print("2. 完成初始化设置")
        print("3. 在后台获取 API Token")

    except paramiko.AuthenticationException:
        print("认证失败，请检查用户名和密码")
        sys.exit(1)
    except paramiko.SSHException as e:
        print(f"SSH 连接错误: {e}")
        sys.exit(1)
    except Exception as e:
        print(f"错误: {e}")
        sys.exit(1)
    finally:
        client.close()

if __name__ == "__main__":
    main()
