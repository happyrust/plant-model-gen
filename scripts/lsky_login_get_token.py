#!/usr/bin/env python3
"""
登录 Lsky-Pro 并获取 API Token
"""

from playwright.sync_api import sync_playwright
import time
import re

BASE_URL = "http://123.57.182.243:8089"
ADMIN_EMAIL = "admin@example.com"
ADMIN_PASSWORD = "Admin123456"

def main():
    print("启动浏览器...")

    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        context = browser.new_context(viewport={'width': 1280, 'height': 900})
        page = context.new_page()

        try:
            # 访问登录页面
            print("\n访问登录页面...")
            page.goto(f"{BASE_URL}/login", timeout=30000)
            page.wait_for_load_state("networkidle")
            time.sleep(2)

            print(f"  当前 URL: {page.url}")
            print(f"  页面标题: {page.title()}")

            page.screenshot(path="lsky_login.png", full_page=True)

            # 填写登录表单
            print("\n填写登录表单...")

            email_input = page.locator("input[type='email']:visible, input[name='email']:visible")
            if email_input.count() > 0:
                email_input.fill(ADMIN_EMAIL)
                print(f"  已填写邮箱: {ADMIN_EMAIL}")

            pwd_input = page.locator("input[type='password']:visible")
            if pwd_input.count() > 0:
                pwd_input.fill(ADMIN_PASSWORD)
                print("  已填写密码")

            time.sleep(1)

            # 点击登录按钮
            login_btn = page.locator("button:visible:has-text('登录'), button:visible:has-text('Login')")
            if login_btn.count() > 0:
                login_btn.click()
                print("  已点击登录按钮")
            else:
                # 尝试提交表单
                page.locator("button[type='submit']:visible").first.click()
                print("  已点击提交按钮")

            time.sleep(5)

            print(f"\n  登录后 URL: {page.url}")
            page.screenshot(path="lsky_after_login.png", full_page=True)

            # 检查是否登录成功
            if "login" not in page.url.lower():
                print("  登录成功!")

                # 访问后台设置页面获取 Token
                print("\n访问设置页面...")
                page.goto(f"{BASE_URL}/admin/settings", timeout=30000)
                time.sleep(3)
                page.screenshot(path="lsky_settings.png", full_page=True)

                # 尝试访问接口设置
                page.goto(f"{BASE_URL}/admin/settings/api", timeout=30000)
                time.sleep(3)
                page.screenshot(path="lsky_api_settings.png", full_page=True)

                # 尝试通过 API 获取 Token
                print("\n尝试通过 API 获取 Token...")

                # 使用 requests 库通过 API 登录获取 token
                import requests
                login_resp = requests.post(
                    f"{BASE_URL}/api/v1/tokens",
                    json={
                        "email": ADMIN_EMAIL,
                        "password": ADMIN_PASSWORD
                    },
                    headers={"Accept": "application/json"}
                )
                print(f"  API 登录状态: {login_resp.status_code}")
                if login_resp.status_code == 200:
                    data = login_resp.json()
                    if data.get("status"):
                        token = data.get("data", {}).get("token", "")
                        print(f"\n  API Token: {token}")
                        print("\n保存 Token 到文件...")
                        with open("lsky_token.txt", "w") as f:
                            f.write(token)
                        print("  已保存到 lsky_token.txt")
                    else:
                        print(f"  响应: {data}")
                else:
                    print(f"  响应: {login_resp.text[:300]}")

            else:
                print("  登录失败")
                body_text = page.inner_text("body")[:500]
                print(f"  页面内容: {body_text[:300]}...")

        except Exception as e:
            print(f"错误: {e}")
            page.screenshot(path="lsky_error.png", full_page=True)
            import traceback
            traceback.print_exc()
        finally:
            browser.close()

if __name__ == "__main__":
    main()
