#!/usr/bin/env python3
"""
配置 Docker 国内镜像源并部署 Lsky-Pro
"""

import paramiko
import time
import sys

HOST = "123.57.182.243"
PORT = 22
USERNAME = "root"
PASSWORD = "Happytest123_"

def run_command(client, command, timeout=300):
    """执行单个命令并返回输出"""
    print(f"[CMD] {command}")
    stdin, stdout, stderr = client.exec_command(command, timeout=timeout)

    output = stdout.read().decode('utf-8', errors='ignore')
    error = stderr.read().decode('utf-8', errors='ignore')
    exit_code = stdout.channel.recv_exit_status()

    if output:
        print(output)
    if error:
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

        # 1. 配置 Docker 国内镜像源
        print("=" * 60)
        print("步骤 1: 配置 Docker 国内镜像源")
        print("=" * 60)

        # 阿里云/腾讯云等国内镜像
        daemon_json = '''{
  "registry-mirrors": [
    "https://docker.1ms.run",
    "https://docker.xuanyuan.me",
    "https://docker.m.daocloud.io"
  ]
}'''

        run_command(client, "mkdir -p /etc/docker")
        run_command(client, f"cat > /etc/docker/daemon.json << 'EOF'\n{daemon_json}\nEOF")
        run_command(client, "cat /etc/docker/daemon.json")

        # 2. 重启 Docker 服务
        print()
        print("=" * 60)
        print("步骤 2: 重启 Docker 服务")
        print("=" * 60)

        run_command(client, "systemctl daemon-reload")
        run_command(client, "systemctl restart docker")
        time.sleep(3)
        run_command(client, "docker info | grep -A 5 'Registry Mirrors'")

        # 3. 拉取镜像
        print()
        print("=" * 60)
        print("步骤 3: 拉取 Lsky-Pro 镜像 (使用国内镜像源)")
        print("=" * 60)

        output, error, code = run_command(client, "cd /opt/lsky-pro && docker-compose pull", timeout=600)

        if code != 0:
            print("[WARN] docker-compose pull 失败，尝试直接 docker pull...")
            run_command(client, "docker pull halcyonazure/lsky-pro-docker:latest", timeout=600)

        # 4. 启动容器
        print()
        print("=" * 60)
        print("步骤 4: 启动 Lsky-Pro 容器")
        print("=" * 60)

        run_command(client, "cd /opt/lsky-pro && docker-compose up -d", timeout=120)
        time.sleep(5)

        # 5. 检查状态
        print()
        print("=" * 60)
        print("步骤 5: 检查容器状态")
        print("=" * 60)

        output, _, _ = run_command(client, "docker ps --filter name=lsky-pro --format 'table {{.Names}}\t{{.Status}}\t{{.Ports}}'")

        if "lsky-pro" in output and "Up" in output:
            print()
            print("=" * 60)
            print("  部署成功!")
            print("=" * 60)
            print()
            print("访问地址: http://123.57.182.243:8089")
            print()
            print("下一步:")
            print("1. 访问上面的地址完成初始化")
            print("2. 设置管理员账号密码")
            print("3. 在后台 -> 设置 -> 接口 -> 生成 API Token")
        else:
            print()
            print("[WARN] 容器可能未正常启动，检查日志:")
            run_command(client, "docker logs lsky-pro --tail 50")

    except Exception as e:
        print(f"错误: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)
    finally:
        client.close()

if __name__ == "__main__":
    main()
