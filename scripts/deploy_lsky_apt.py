#!/usr/bin/env python3
"""
通过 SSH 使用 apt 安装 Docker 并部署 Lsky-Pro
"""

import paramiko
import time
import sys

# 服务器配置
HOST = "123.57.182.243"
PORT = 22
USERNAME = "root"
PASSWORD = "Happytest123_"

def run_command(client, command, timeout=120):
    """执行单个命令并返回输出"""
    print(f"[CMD] {command}")
    stdin, stdout, stderr = client.exec_command(command, timeout=timeout)

    # 读取输出
    output = stdout.read().decode('utf-8', errors='ignore')
    error = stderr.read().decode('utf-8', errors='ignore')
    exit_code = stdout.channel.recv_exit_status()

    if output:
        print(output)
    if error and exit_code != 0:
        print(f"[STDERR] {error}")

    return output, error, exit_code

def main():
    print(f"连接到服务器 {HOST}...")

    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())

    try:
        client.connect(HOST, port=PORT, username=USERNAME, password=PASSWORD, timeout=30)
        print("连接成功!")
        print()

        # 1. 安装 Docker
        print("=" * 60)
        print("步骤 1: 安装 Docker")
        print("=" * 60)

        run_command(client, "apt-get update -qq", timeout=120)
        run_command(client, "apt-get install -y docker.io docker-compose", timeout=300)

        # 2. 启动 Docker 服务
        print()
        print("=" * 60)
        print("步骤 2: 启动 Docker 服务")
        print("=" * 60)

        run_command(client, "systemctl start docker")
        run_command(client, "systemctl enable docker")
        run_command(client, "docker --version")

        # 3. 创建配置文件
        print()
        print("=" * 60)
        print("步骤 3: 创建 Lsky-Pro 配置")
        print("=" * 60)

        run_command(client, "mkdir -p /opt/lsky-pro")

        compose_content = '''version: '3.8'

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
'''

        # 写入配置文件
        run_command(client, f"cat > /opt/lsky-pro/docker-compose.yml << 'EOF'\n{compose_content}EOF")
        run_command(client, "cat /opt/lsky-pro/docker-compose.yml")

        # 4. 拉取镜像并启动
        print()
        print("=" * 60)
        print("步骤 4: 拉取并启动 Lsky-Pro")
        print("=" * 60)

        run_command(client, "cd /opt/lsky-pro && docker-compose pull", timeout=300)
        run_command(client, "cd /opt/lsky-pro && docker-compose up -d", timeout=60)

        # 5. 等待并检查
        print()
        print("等待服务启动...")
        time.sleep(5)

        output, _, _ = run_command(client, "docker ps --filter name=lsky-pro")

        # 检查防火墙
        print()
        print("=" * 60)
        print("步骤 5: 检查防火墙")
        print("=" * 60)
        run_command(client, "ufw status 2>/dev/null || echo 'UFW not active'")

        print()
        print("=" * 60)
        print("  部署完成!")
        print("=" * 60)
        print()
        print("访问地址: http://123.57.182.243:8089")
        print()
        print("下一步:")
        print("1. 访问上面的地址完成初始化")
        print("2. 设置管理员账号密码")
        print("3. 在后台 -> 设置 -> 接口 -> 生成 API Token")

    except paramiko.AuthenticationException:
        print("认证失败，请检查用户名和密码")
        sys.exit(1)
    except Exception as e:
        print(f"错误: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)
    finally:
        client.close()

if __name__ == "__main__":
    main()
