#!/usr/bin/env python3
"""
修复 Lsky-Pro 配置并完成安装
"""

import paramiko
import time

HOST = "123.57.182.243"
PORT = 22
USERNAME = "root"
PASSWORD = "Happytest123_"

def run_cmd(client, cmd, timeout=120):
    print(f"[CMD] {cmd[:80]}..." if len(cmd) > 80 else f"[CMD] {cmd}")
    stdin, stdout, stderr = client.exec_command(cmd, timeout=timeout)
    out = stdout.read().decode('utf-8', errors='ignore')
    err = stderr.read().decode('utf-8', errors='ignore')
    code = stdout.channel.recv_exit_status()
    if out.strip():
        for line in out.strip().split('\n')[:10]:
            print(f"  {line}")
    if err.strip() and code != 0:
        print(f"[ERR] {err[:200]}")
    return out, err, code

def main():
    print(f"连接到服务器 {HOST}...")
    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    client.connect(HOST, port=PORT, username=USERNAME, password=PASSWORD, timeout=30)
    print("连接成功!\n")

    # 1. 停止容器
    print("=" * 60)
    print("1. 停止容器")
    print("=" * 60)
    run_cmd(client, "docker stop lsky-pro")

    # 2. 创建持久化的 .env 文件到 data 目录
    print("\n" + "=" * 60)
    print("2. 创建持久化配置")
    print("=" * 60)

    # 生成随机 APP_KEY (base64 格式，32字节)
    out, _, _ = run_cmd(client, "openssl rand -base64 32")
    app_key = out.strip()

    env_content = f'''APP_NAME="Lsky Pro"
APP_ENV=production
APP_KEY=base64:{app_key}
APP_DEBUG=true
APP_URL=http://123.57.182.243:8089

LOG_CHANNEL=daily
LOG_LEVEL=debug

DB_CONNECTION=sqlite
DB_DATABASE=/var/www/html/storage/database/database.sqlite

CACHE_DRIVER=file
FILESYSTEM_DISK=local
SESSION_DRIVER=file
SESSION_LIFETIME=120
'''

    # 写入到持久化目录
    run_cmd(client, f'''cat > /opt/lsky-pro/data/.env << 'EOF'
{env_content}
EOF''')

    # 3. 创建数据库文件
    print("\n" + "=" * 60)
    print("3. 创建数据库文件")
    print("=" * 60)
    run_cmd(client, "mkdir -p /opt/lsky-pro/data/database")
    run_cmd(client, "touch /opt/lsky-pro/data/database/database.sqlite")
    run_cmd(client, "chmod 666 /opt/lsky-pro/data/database/database.sqlite")

    # 4. 更新 docker-compose 挂载 .env 文件
    print("\n" + "=" * 60)
    print("4. 更新 Docker Compose 配置")
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
      - ./data/.env:/var/www/html/.env
    environment:
      - TZ=Asia/Shanghai
'''

    run_cmd(client, f'''cat > /opt/lsky-pro/docker-compose.yml << 'EOF'
{compose_content}
EOF''')

    # 5. 启动容器
    print("\n" + "=" * 60)
    print("5. 启动容器")
    print("=" * 60)
    run_cmd(client, "cd /opt/lsky-pro && docker-compose up -d")
    time.sleep(5)

    # 6. 运行数据库迁移
    print("\n" + "=" * 60)
    print("6. 运行数据库迁移")
    print("=" * 60)
    run_cmd(client, "docker exec lsky-pro php artisan migrate --force", timeout=120)

    # 7. 创建管理员用户
    print("\n" + "=" * 60)
    print("7. 创建管理员用户")
    print("=" * 60)

    # 先创建默认用户组
    run_cmd(client, '''docker exec lsky-pro php artisan tinker --execute="
\\App\\Models\\Group::firstOrCreate(['id' => 1], [
    'name' => 'Default',
    'is_default' => true,
    'configs' => json_encode([])
]);
echo 'Group created';
"''')

    # 创建管理员
    run_cmd(client, '''docker exec lsky-pro php artisan tinker --execute="
\\App\\Models\\User::updateOrCreate(
    ['email' => 'admin@example.com'],
    [
        'name' => 'Admin',
        'password' => bcrypt('Admin123456'),
        'is_adminer' => true,
        'group_id' => 1,
        'capacity' => 0,
        'status' => 1,
        'email_verified_at' => now(),
    ]
);
echo 'Admin created';
"''')

    # 8. 检查状态
    print("\n" + "=" * 60)
    print("8. 检查状态")
    print("=" * 60)
    run_cmd(client, "docker ps --filter name=lsky-pro")

    # 9. 检查日志
    print("\n" + "=" * 60)
    print("9. 检查最新日志")
    print("=" * 60)
    run_cmd(client, "docker logs lsky-pro --tail 10")

    print("\n" + "=" * 60)
    print("完成!")
    print("=" * 60)
    print()
    print(f"访问地址: http://{HOST}:8089")
    print(f"管理员邮箱: admin@example.com")
    print(f"管理员密码: Admin123456")

    client.close()

if __name__ == "__main__":
    main()
