#!/usr/bin/env python3
"""
使用 Playwright 自动完成 Lsky-Pro 安装向导 (改进版)
"""

from playwright.sync_api import sync_playwright
import time

BASE_URL = "http://123.57.182.243:8089"
ADMIN_EMAIL = "admin@example.com"
ADMIN_PASSWORD = "Admin123456"

def main():
    print("启动浏览器...")

    with sync_playwright() as p:
        browser = p.chromium.launch(headless=True)
        context = browser.new_context(viewport={'width': 1280, 'height': 720})
        page = context.new_page()

        try:
            # Step 1: 访问安装页面
            print("\n" + "=" * 60)
            print("Step 1: 访问安装页面")
            print("=" * 60)

            page.goto(f"{BASE_URL}/install", timeout=30000)
            time.sleep(3)
            print(f"  页面标题: {page.title()}")

            # 查看页面内容
            body_text = page.locator("body").inner_text()[:500]
            print(f"  页面内容: {body_text[:200]}...")

            page.screenshot(path="lsky_install_1.png")

            # Step 2: 环境检测 - 点击下一步
            print("\n" + "=" * 60)
            print("Step 2: 环境检测")
            print("=" * 60)

            # 等待页面加载
            time.sleep(2)

            # 查找所有按钮
            buttons = page.locator("button").all()
            print(f"  找到 {len(buttons)} 个按钮")

            for i, btn in enumerate(buttons):
                try:
                    text = btn.inner_text()
                    print(f"    按钮 {i}: {text}")
                except:
                    pass

            # 点击下一步/Next 按钮
            try:
                next_btn = page.locator("button").filter(has_text="下一步")
                if next_btn.count() > 0:
                    print("  点击'下一步'按钮...")
                    next_btn.first.click()
                    time.sleep(3)
                else:
                    # 尝试点击第一个按钮
                    if len(buttons) > 0:
                        print("  点击第一个按钮...")
                        buttons[0].click()
                        time.sleep(3)
            except Exception as e:
                print(f"  点击失败: {e}")

            page.screenshot(path="lsky_install_2.png")
            print(f"  当前 URL: {page.url}")

            # Step 3: 数据库配置
            print("\n" + "=" * 60)
            print("Step 3: 数据库配置")
            print("=" * 60)

            # 查看页面内容
            body_text = page.locator("body").inner_text()[:500]
            print(f"  页面内容: {body_text[:200]}...")

            # 查找 SQLite 选项
            sqlite_elements = page.locator("text=SQLite").all()
            if sqlite_elements:
                print("  找到 SQLite 选项，点击...")
                sqlite_elements[0].click()
                time.sleep(1)

            # 查找所有 input 元素
            inputs = page.locator("input").all()
            print(f"  找到 {len(inputs)} 个输入框")

            # 点击下一步
            next_btn = page.locator("button").filter(has_text="下一步")
            if next_btn.count() > 0:
                print("  点击'下一步'按钮...")
                next_btn.first.click()
                time.sleep(3)

            page.screenshot(path="lsky_install_3.png")
            print(f"  当前 URL: {page.url}")

            # Step 4: 管理员账户
            print("\n" + "=" * 60)
            print("Step 4: 管理员账户设置")
            print("=" * 60)

            body_text = page.locator("body").inner_text()[:500]
            print(f"  页面内容: {body_text[:200]}...")

            # 查找邮箱输入框
            email_input = page.locator("input[type='email'], input[name='email'], input[placeholder*='邮箱'], input[placeholder*='email']")
            if email_input.count() > 0:
                print(f"  输入邮箱: {ADMIN_EMAIL}")
                email_input.first.fill(ADMIN_EMAIL)

            # 查找密码输入框
            password_inputs = page.locator("input[type='password']")
            print(f"  找到 {password_inputs.count()} 个密码框")
            if password_inputs.count() >= 1:
                print("  输入密码...")
                password_inputs.first.fill(ADMIN_PASSWORD)
            if password_inputs.count() >= 2:
                print("  确认密码...")
                password_inputs.nth(1).fill(ADMIN_PASSWORD)

            page.screenshot(path="lsky_install_4a.png")

            # 提交安装
            submit_btn = page.locator("button").filter(has_text="完成")
            if submit_btn.count() == 0:
                submit_btn = page.locator("button").filter(has_text="安装")
            if submit_btn.count() == 0:
                submit_btn = page.locator("button[type='submit']")

            if submit_btn.count() > 0:
                print("  提交安装...")
                submit_btn.first.click()
                time.sleep(5)

            page.screenshot(path="lsky_install_4b.png")
            print(f"  当前 URL: {page.url}")

            # 检查结果
            print("\n" + "=" * 60)
            print("检查安装结果")
            print("=" * 60)

            page.goto(BASE_URL, timeout=30000)
            time.sleep(3)
            print(f"  最终 URL: {page.url}")
            print(f"  页面标题: {page.title()}")

            page.screenshot(path="lsky_final.png")

            if "install" not in page.url.lower():
                print("\n  安装成功!")
            else:
                print("\n  安装可能需要手动完成")

            print("\n" + "=" * 60)
            print("配置信息")
            print("=" * 60)
            print(f"  访问地址: {BASE_URL}")
            print(f"  管理员邮箱: {ADMIN_EMAIL}")
            print(f"  管理员密码: {ADMIN_PASSWORD}")
            print()
            print("截图文件: lsky_install_*.png")

        except Exception as e:
            print(f"错误: {e}")
            page.screenshot(path="lsky_error.png")
            import traceback
            traceback.print_exc()
        finally:
            browser.close()

if __name__ == "__main__":
    main()
