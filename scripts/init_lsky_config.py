#!/usr/bin/env python3
"""
自动完成 Lsky-Pro 初始化配置
"""

import paramiko
import requests
import time
import sys

HOST = "123.57.182.243"
PORT = 22
USERNAME = "root"
PASSWORD = "Happytest123_"

LSKY_URL = f"http://{HOST}:8089"

def run_ssh_command(client, command, timeout=60):
    """执行 SSH 命令"""
    print(f"[SSH] {command}")
    stdin, stdout, stderr = client.exec_command(command, timeout=timeout)
    output = stdout.read().decode('utf-8', errors='ignore')
    error = stderr.read().decode('utf-8', errors='ignore')
    if output:
        print(output)
    if error:
        print(f"[STDERR] {error}")
    return output, error

def check_lsky_status():
    """检查 Lsky-Pro 是否可访问"""
    try:
        resp = requests.get(f"{LSKY_URL}/api/v1", timeout=10)
        return resp.status_code == 200
    except:
        return False

def main():
    print(f"连接到服务器 {HOST}...")

    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())

    try:
        client.connect(HOST, port=PORT, username=USERNAME, password=PASSWORD, timeout=30)
        print("SSH 连接成功!")
        print()

        # 检查容器状态
        print("=" * 60)
        print("检查 Lsky-Pro 容器状态")
        print("=" * 60)

        run_ssh_command(client, "docker ps --filter name=lsky-pro")

        # 进入容器执行 artisan 命令进行初始化
        print()
        print("=" * 60)
        print("执行 Lsky-Pro 初始化")
        print("=" * 60)

        # 创建 .env 文件配置
        env_config = '''APP_NAME=Lsky-Pro
APP_ENV=production
APP_KEY=
APP_DEBUG=false
APP_URL=http://123.57.182.243:8089

LOG_CHANNEL=daily

DB_CONNECTION=sqlite
DB_DATABASE=/var/www/html/storage/database/database.sqlite

CACHE_DRIVER=file
SESSION_DRIVER=file
SESSION_LIFETIME=120
'''

        # 写入 .env 文件
        run_ssh_command(client, f'''docker exec lsky-pro bash -c "cat > /var/www/html/.env << 'EOF'
{env_config}
EOF"''')

        # 生成应用密钥
        print()
        print("生成应用密钥...")
        run_ssh_command(client, "docker exec lsky-pro php artisan key:generate --force")

        # 创建数据库目录和文件
        print()
        print("初始化数据库...")
        run_ssh_command(client, "docker exec lsky-pro mkdir -p /var/www/html/storage/database")
        run_ssh_command(client, "docker exec lsky-pro touch /var/www/html/storage/database/database.sqlite")
        run_ssh_command(client, "docker exec lsky-pro chown -R www-data:www-data /var/www/html/storage")

        # 运行数据库迁移
        print()
        print("运行数据库迁移...")
        run_ssh_command(client, "docker exec lsky-pro php artisan migrate --force", timeout=120)

        # 创建管理员用户
        # 注意：Lsky-Pro 需要通过 Web 界面完成首次安装
        print()
        print("=" * 60)
        print("检查 Web 服务")
        print("=" * 60)

        # 检查 Web 是否可访问
        time.sleep(3)

        try:
            resp = requests.get(LSKY_URL, timeout=10)
            print(f"HTTP 状态码: {resp.status_code}")
            if "安装" in resp.text or "install" in resp.text.lower():
                print("Lsky-Pro 需要通过 Web 界面完成安装向导")
        except Exception as e:
            print(f"无法访问 Web 服务: {e}")

        print()
        print("=" * 60)
        print("  配置完成!")
        print("=" * 60)
        print()
        print(f"请访问: {LSKY_URL}")
        print()
        print("推荐的账户设置:")
        print("  - 管理员邮箱: admin@example.com")
        print("  - 管理员密码: Admin123456")
        print()

    except Exception as e:
        print(f"错误: {e}")
        import traceback
        traceback.print_exc()
    finally:
        client.close()

if __name__ == "__main__":
    main()
