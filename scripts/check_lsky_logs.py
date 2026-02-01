#!/usr/bin/env python3
"""
检查 Lsky-Pro 容器日志
"""

import paramiko

HOST = "123.57.182.243"
PORT = 22
USERNAME = "root"
PASSWORD = "Happytest123_"

def run_cmd(client, cmd, timeout=60):
    stdin, stdout, stderr = client.exec_command(cmd, timeout=timeout)
    out = stdout.read().decode('utf-8', errors='ignore')
    err = stderr.read().decode('utf-8', errors='ignore')
    return out + err

def main():
    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    client.connect(HOST, port=PORT, username=USERNAME, password=PASSWORD, timeout=30)

    print("检查容器日志...")
    print("=" * 60)
    logs = run_cmd(client, "docker logs lsky-pro --tail 50 2>&1")
    print(logs)

    print("\n检查 .env 文件...")
    print("=" * 60)
    env = run_cmd(client, "docker exec lsky-pro cat /var/www/html/.env 2>&1")
    print(env)

    print("\n检查存储目录权限...")
    print("=" * 60)
    perms = run_cmd(client, "docker exec lsky-pro ls -la /var/www/html/storage/ 2>&1")
    print(perms)

    client.close()

if __name__ == "__main__":
    main()
