#!/usr/bin/env python3
"""
使用 docker run 启动 Lsky-Pro
"""

import paramiko
import time

HOST = "123.57.182.243"
PORT = 22
USERNAME = "root"
PASSWORD = "Happytest123_"

def run_cmd(client, cmd, timeout=120):
    print(f"[CMD] {cmd[:100]}..." if len(cmd) > 100 else f"[CMD] {cmd}")
    stdin, stdout, stderr = client.exec_command(cmd, timeout=timeout)
    out = stdout.read().decode('utf-8', errors='ignore')
    err = stderr.read().decode('utf-8', errors='ignore')
    code = stdout.channel.recv_exit_status()
    if out.strip():
        print(out[:500])
    if err.strip():
        print(f"[INFO] {err[:300]}")
    return out, err, code

def main():
    print(f"连接到服务器 {HOST}...")
    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    client.connect(HOST, port=PORT, username=USERNAME, password=PASSWORD, timeout=30)
    print("连接成功!\n")

    # 1. 清理旧容器
    print("=" * 60)
    print("1. 清理旧容器")
    print("=" * 60)
    run_cmd(client, "docker rm -f lsky-pro 2>/dev/null || true")

    # 2. 准备配置
    print("\n" + "=" * 60)
    print("2. 准备配置")
    print("=" * 60)

    run_cmd(client, "mkdir -p /opt/lsky-pro/data")

    # 3. 直接使用 docker run 启动
    print("\n" + "=" * 60)
    print("3. 启动容器")
    print("=" * 60)

    docker_run_cmd = '''docker run -d \\
        --name lsky-pro \\
        --restart unless-stopped \\
        -p 8089:8089 \\
        -v /opt/lsky-pro/data:/var/www/html/storage \\
        -e TZ=Asia/Shanghai \\
        halcyonazure/lsky-pro-docker:latest'''

    run_cmd(client, docker_run_cmd)
    time.sleep(5)

    # 4. 检查状态
    print("\n" + "=" * 60)
    print("4. 检查容器状态")
    print("=" * 60)
    run_cmd(client, "docker ps --filter name=lsky-pro")

    # 5. 检查日志
    print("\n" + "=" * 60)
    print("5. 检查日志")
    print("=" * 60)
    run_cmd(client, "docker logs lsky-pro --tail 20")

    # 6. ���试访问
    print("\n" + "=" * 60)
    print("6. 测试访问")
    print("=" * 60)
    run_cmd(client, "curl -s -o /dev/null -w '%{http_code}' http://localhost:8089/")

    print("\n" + "=" * 60)
    print("容器已启动!")
    print("=" * 60)
    print()
    print(f"请访问: http://{HOST}:8089")
    print()
    print("安装向导将引导你完成:")
    print("1. 环境检测 (点击'下一步')")
    print("2. 数据库配置 (选择 SQLite)")
    print("3. 管理员设置:")
    print("   - 邮箱: admin@example.com")
    print("   - 密码: Admin123456")

    client.close()

if __name__ == "__main__":
    main()
