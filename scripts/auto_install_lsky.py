#!/usr/bin/env python3
"""
自动完成 Lsky-Pro 安装向导
"""

import requests
import re
import time

BASE_URL = "http://123.57.182.243:8089"

# 默认管理员配置
ADMIN_EMAIL = "admin@example.com"
ADMIN_PASSWORD = "Admin123456"

def get_csrf_token(session):
    """获取 CSRF Token"""
    resp = session.get(f"{BASE_URL}/install")
    if resp.status_code == 200:
        match = re.search(r'name="csrf-token"\s+content="([^"]+)"', resp.text)
        if match:
            return match.group(1)
        match = re.search(r'name="_token"\s+value="([^"]+)"', resp.text)
        if match:
            return match.group(1)
    return None

def complete_install():
    """完成安装流程"""
    session = requests.Session()

    # Step 1: 获取安装页面和 CSRF token
    print("Step 1: 获取安装页面...")
    resp = session.get(f"{BASE_URL}/install")
    print(f"  状态码: {resp.status_code}")

    # 尝试从 meta 标签获取 CSRF token
    csrf_match = re.search(r'content="([^"]+)"\s*name="csrf-token"|name="csrf-token"\s*content="([^"]+)"', resp.text)
    if csrf_match:
        csrf_token = csrf_match.group(1) or csrf_match.group(2)
        print(f"  CSRF Token: {csrf_token[:20]}...")
    else:
        print("  [ERROR] 无法获取 CSRF Token")
        # 尝试从 cookies 获取
        csrf_token = session.cookies.get('XSRF-TOKEN', '')
        if csrf_token:
            print(f"  从 Cookie 获取 CSRF Token: {csrf_token[:20]}...")

    # 设置请求头
    headers = {
        'X-CSRF-TOKEN': csrf_token,
        'X-Requested-With': 'XMLHttpRequest',
        'Accept': 'application/json',
        'Content-Type': 'application/x-www-form-urlencoded',
        'Referer': f'{BASE_URL}/install',
    }

    # Step 2: 提交环境检测
    print("\nStep 2: 提交环境检测...")
    resp = session.post(f"{BASE_URL}/install/checking",
                        headers=headers,
                        data={'_token': csrf_token})
    print(f"  状态码: {resp.status_code}")
    if resp.status_code == 200:
        try:
            data = resp.json()
            print(f"  响应: {data}")
        except:
            print(f"  响应: {resp.text[:200]}")

    # Step 3: 提交数据库配置 (使用 SQLite)
    print("\nStep 3: 提交数据库配置...")
    db_data = {
        '_token': csrf_token,
        'connection': 'sqlite',
    }
    resp = session.post(f"{BASE_URL}/install/database",
                        headers=headers,
                        data=db_data)
    print(f"  状态码: {resp.status_code}")
    if resp.status_code == 200:
        try:
            data = resp.json()
            print(f"  响应: {data}")
        except:
            print(f"  响应: {resp.text[:200]}")

    # Step 4: 提交管理员账户信息
    print("\nStep 4: 创建管理员账户...")
    admin_data = {
        '_token': csrf_token,
        'email': ADMIN_EMAIL,
        'password': ADMIN_PASSWORD,
        'password_confirmation': ADMIN_PASSWORD,
    }
    resp = session.post(f"{BASE_URL}/install/account",
                        headers=headers,
                        data=admin_data)
    print(f"  状态码: {resp.status_code}")
    if resp.status_code == 200:
        try:
            data = resp.json()
            print(f"  响应: {data}")
        except:
            print(f"  响应: {resp.text[:200]}")

    # 检查安装是否完成
    print("\n检查安装状态...")
    time.sleep(2)

    resp = session.get(BASE_URL)
    if "安装" not in resp.text and "install" not in resp.text.lower():
        print("\n" + "=" * 60)
        print("  安装成功!")
        print("=" * 60)
        print()
        print(f"访问地址: {BASE_URL}")
        print(f"管理员邮箱: {ADMIN_EMAIL}")
        print(f"管理员密码: {ADMIN_PASSWORD}")
    else:
        print("\n[INFO] 安装可能需要手动完成某些步骤")
        print(f"请访问: {BASE_URL}/install")

    # 尝试登录获取 Token
    print("\n尝试登录获取 API Token...")
    login_resp = session.post(f"{BASE_URL}/api/v1/tokens",
                              json={
                                  'email': ADMIN_EMAIL,
                                  'password': ADMIN_PASSWORD,
                              },
                              headers={'Accept': 'application/json'})
    print(f"  登录状态码: {login_resp.status_code}")
    if login_resp.status_code == 200:
        try:
            data = login_resp.json()
            if data.get('status'):
                token = data.get('data', {}).get('token', '')
                print(f"\n  API Token: {token}")
                print("\n  请保存此 Token 用于上传图片!")
        except:
            print(f"  响应: {login_resp.text[:500]}")
    else:
        print(f"  登录失败: {login_resp.text[:300]}")

if __name__ == "__main__":
    complete_install()
