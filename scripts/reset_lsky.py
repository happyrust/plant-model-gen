#!/usr/bin/env python3
"""
Lsky-Pro 完整安装 - 使用 MySQL 内置数据库或正确配置 SQLite
"""

import paramiko
import time

HOST = "123.57.182.243"
PORT = 22
USERNAME = "root"
PASSWORD = "Happytest123_"

ADMIN_EMAIL = "admin@example.com"
ADMIN_PASSWORD = "Admin123456"

def run_cmd(client, cmd, timeout=120):
    print(f"[CMD] {cmd[:100]}..." if len(cmd) > 100 else f"[CMD] {cmd}")
    stdin, stdout, stderr = client.exec_command(cmd, timeout=timeout)
    out = stdout.read().decode('utf-8', errors='ignore')
    err = stderr.read().decode('utf-8', errors='ignore')
    code = stdout.channel.recv_exit_status()
    if out.strip():
        print(out[:500] if len(out) > 500 else out)
    if err.strip() and code != 0:
        print(f"[ERR] {err[:300]}")
    return out, err, code

def main():
    print(f"连接到服务器 {HOST}...")
    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    client.connect(HOST, port=PORT, username=USERNAME, password=PASSWORD, timeout=30)
    print("连接成功!\n")

    # 1. 停止旧容器
    print("=" * 60)
    print("1. 停止旧容器")
    print("=" * 60)
    run_cmd(client, "cd /opt/lsky-pro && docker-compose down")

    # 2. 更新 docker-compose 使用内置 SQLite 配置
    print("\n" + "=" * 60)
    print("2. 更新配置")
    print("=" * 60)

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
      - INSTALL_RESET=true
'''

    run_cmd(client, f'''cat > /opt/lsky-pro/docker-compose.yml << 'EOF'
{compose_content}
EOF''')

    # 3. 清理旧数据重新安装
    print("\n" + "=" * 60)
    print("3. 清理旧数据")
    print("=" * 60)
    run_cmd(client, "rm -rf /opt/lsky-pro/data/*")

    # 4. 重新启动
    print("\n" + "=" * 60)
    print("4. 启动容器")
    print("=" * 60)
    run_cmd(client, "cd /opt/lsky-pro && docker-compose up -d")
    time.sleep(5)

    # 5. 检查状态
    print("\n" + "=" * 60)
    print("5. 检查容器状态")
    print("=" * 60)
    run_cmd(client, "docker ps --filter name=lsky-pro")

    print("\n" + "=" * 60)
    print("配置完成!")
    print("=" * 60)
    print()
    print("Lsky-Pro 需要通过 Web 界面完成安装向导。")
    print()
    print(f"请访问: http://{HOST}:8089")
    print()
    print("安装步骤:")
    print("1. 点击 '下一步' 通过环境检测")
    print("2. 数据库选择 SQLite (无需配置)")
    print("3. 设置管理员账户:")
    print(f"   - 邮箱: {ADMIN_EMAIL}")
    print(f"   - 密码: {ADMIN_PASSWORD}")
    print("4. 完成安装")
    print()
    print("安装完成后，在后台获取 API Token:")
    print("  后台 -> 设置 -> 接口 -> 生成 Token")

    client.close()

if __name__ == "__main__":
    main()
