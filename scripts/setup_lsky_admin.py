#!/usr/bin/env python3
"""
通过 SSH 在容器内完成 Lsky-Pro 安装
"""

import paramiko
import time

HOST = "123.57.182.243"
PORT = 22
USERNAME = "root"
PASSWORD = "Happytest123_"

ADMIN_EMAIL = "admin@example.com"
ADMIN_PASSWORD = "Admin123456"

def run_cmd(client, cmd, timeout=60):
    """执行命令"""
    print(f"[CMD] {cmd}")
    stdin, stdout, stderr = client.exec_command(cmd, timeout=timeout)
    out = stdout.read().decode('utf-8', errors='ignore')
    err = stderr.read().decode('utf-8', errors='ignore')
    code = stdout.channel.recv_exit_status()
    if out:
        print(out)
    if err and code != 0:
        print(f"[ERR] {err}")
    return out, err, code

def main():
    print(f"连接到服务器 {HOST}...")
    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    client.connect(HOST, port=PORT, username=USERNAME, password=PASSWORD, timeout=30)
    print("连接成功!\n")

    # 1. 检查容器状态
    print("=" * 60)
    print("1. 检查容器状态")
    print("=" * 60)
    run_cmd(client, "docker ps --filter name=lsky-pro")

    # 2. 创建必要的目录和文件
    print("\n" + "=" * 60)
    print("2. 初始化存储目录")
    print("=" * 60)
    run_cmd(client, "docker exec lsky-pro mkdir -p /var/www/html/storage/app/public")
    run_cmd(client, "docker exec lsky-pro mkdir -p /var/www/html/storage/database")
    run_cmd(client, "docker exec lsky-pro touch /var/www/html/storage/database/database.sqlite")
    run_cmd(client, "docker exec lsky-pro chown -R www-data:www-data /var/www/html/storage")
    run_cmd(client, "docker exec lsky-pro chmod -R 775 /var/www/html/storage")

    # 3. 检查 .env 文件
    print("\n" + "=" * 60)
    print("3. 配置 .env 文件")
    print("=" * 60)

    env_content = f'''APP_NAME="Lsky Pro"
APP_ENV=production
APP_DEBUG=false
APP_URL=http://123.57.182.243:8089

LOG_CHANNEL=daily
LOG_LEVEL=error

DB_CONNECTION=sqlite
DB_DATABASE=/var/www/html/storage/database/database.sqlite

CACHE_DRIVER=file
FILESYSTEM_DISK=local
SESSION_DRIVER=file
SESSION_LIFETIME=120
'''

    run_cmd(client, f'''docker exec lsky-pro bash -c "cat > /var/www/html/.env << 'ENVEOF'
{env_content}
ENVEOF"''')

    # 4. 生成 APP_KEY
    print("\n" + "=" * 60)
    print("4. 生成应用密钥")
    print("=" * 60)
    run_cmd(client, "docker exec lsky-pro php artisan key:generate --force")

    # 5. 运行数据库迁移
    print("\n" + "=" * 60)
    print("5. 运行数据库迁移")
    print("=" * 60)
    run_cmd(client, "docker exec lsky-pro php artisan migrate --force", timeout=120)

    # 6. 创建管理员用户（使用 tinker）
    print("\n" + "=" * 60)
    print("6. 创建管理员用户")
    print("=" * 60)

    # 使用 PHP artisan tinker 创建用户
    create_user_cmd = f'''docker exec lsky-pro php artisan tinker --execute="
\\App\\Models\\User::create([
    'name' => 'Admin',
    'email' => '{ADMIN_EMAIL}',
    'password' => bcrypt('{ADMIN_PASSWORD}'),
    'is_adminer' => true,
    'capacity' => 0,
    'status' => 1,
]);
echo 'User created successfully';
"'''
    out, err, code = run_cmd(client, create_user_cmd)

    if code != 0:
        # 尝试另一种方式
        print("\n尝试使用 db:seed 或直接 SQL...")
        sql_cmd = f'''docker exec lsky-pro php -r "
\\$db = new PDO('sqlite:/var/www/html/storage/database/database.sqlite');
\\$hash = password_hash('{ADMIN_PASSWORD}', PASSWORD_BCRYPT);
\\$db->exec(\\"INSERT OR IGNORE INTO users (name, email, password, is_adminer, capacity, status, created_at, updated_at) VALUES ('Admin', '{ADMIN_EMAIL}', '\\$hash', 1, 0, 1, datetime('now'), datetime('now'))\\");
echo 'Done';
"'''
        run_cmd(client, sql_cmd)

    # 7. 清除缓存
    print("\n" + "=" * 60)
    print("7. 清除缓存")
    print("=" * 60)
    run_cmd(client, "docker exec lsky-pro php artisan config:clear")
    run_cmd(client, "docker exec lsky-pro php artisan cache:clear")
    run_cmd(client, "docker exec lsky-pro php artisan view:clear")

    # 8. 重启容器
    print("\n" + "=" * 60)
    print("8. 重启容器")
    print("=" * 60)
    run_cmd(client, "docker restart lsky-pro")
    time.sleep(5)

    # 9. 检查状态
    print("\n" + "=" * 60)
    print("9. 检查最终状态")
    print("=" * 60)
    run_cmd(client, "docker ps --filter name=lsky-pro")

    print("\n" + "=" * 60)
    print("  配置完成!")
    print("=" * 60)
    print()
    print(f"访问地址: http://{HOST}:8089")
    print(f"管理员邮箱: {ADMIN_EMAIL}")
    print(f"管理员密码: {ADMIN_PASSWORD}")
    print()
    print("如果仍显示安装向导，请手动点击完成安装步骤。")

    client.close()

if __name__ == "__main__":
    main()
