#!/usr/bin/env python3
"""
使用 Playwright 自动完成 Lsky-Pro 安装向导
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
        page = browser.new_page()

        try:
            # Step 1: 访问安装页面
            print("\n" + "=" * 60)
            print("Step 1: 访问安装页面")
            print("=" * 60)

            page.goto(f"{BASE_URL}/install", timeout=30000)
            page.wait_for_load_state("networkidle")
            print(f"  当前页面: {page.url}")
            print(f"  标题: {page.title()}")

            # 截图查看当前状态
            page.screenshot(path="install_step1.png")
            print("  截图: install_step1.png")

            # Step 2: 点击下一步（环境检测通过后）
            print("\n" + "=" * 60)
            print("Step 2: 环境检测")
            print("=" * 60)

            # 等待并查找下一步按钮
            time.sleep(2)

            # 尝试找到并点击"下一步"按钮
            next_buttons = page.locator("button, a").filter(has_text="下一步")
            if next_buttons.count() > 0:
                print("  找到'下一步'按钮，点击...")
                next_buttons.first.click()
                time.sleep(2)
                page.screenshot(path="install_step2.png")
            else:
                # 尝试其他可能的按钮文本
                for text in ["Next", "继续", "Continue", "开始安装"]:
                    btn = page.locator("button, a").filter(has_text=text)
                    if btn.count() > 0:
                        print(f"  找到'{text}'按钮，点击...")
                        btn.first.click()
                        time.sleep(2)
                        break

            print(f"  当前页面: {page.url}")

            # Step 3: 数据库配置（选择 SQLite）
            print("\n" + "=" * 60)
            print("Step 3: 数据库配置")
            print("=" * 60)

            # 查找 SQLite 选项
            sqlite_option = page.locator("input[value='sqlite'], label:has-text('SQLite'), button:has-text('SQLite')")
            if sqlite_option.count() > 0:
                print("  找到 SQLite 选项，选择...")
                sqlite_option.first.click()
                time.sleep(1)

            # 点击下一步
            next_btn = page.locator("button, a").filter(has_text="下一步")
            if next_btn.count() > 0:
                next_btn.first.click()
                time.sleep(2)
                page.screenshot(path="install_step3.png")

            print(f"  当前页面: {page.url}")

            # Step 4: 管理员账户设置
            print("\n" + "=" * 60)
            print("Step 4: 管理员账户")
            print("=" * 60)

            # 填写邮箱
            email_input = page.locator("input[type='email'], input[name='email'], input[placeholder*='邮箱']")
            if email_input.count() > 0:
                print(f"  输入邮箱: {ADMIN_EMAIL}")
                email_input.first.fill(ADMIN_EMAIL)

            # 填写密码
            password_inputs = page.locator("input[type='password']")
            if password_inputs.count() >= 1:
                print(f"  输入密码...")
                password_inputs.first.fill(ADMIN_PASSWORD)
            if password_inputs.count() >= 2:
                print(f"  确认密码...")
                password_inputs.nth(1).fill(ADMIN_PASSWORD)

            # 提交
            submit_btn = page.locator("button[type='submit'], button:has-text('完成'), button:has-text('安装'), button:has-text('Submit')")
            if submit_btn.count() > 0:
                print("  提交安装...")
                submit_btn.first.click()
                time.sleep(5)

            page.screenshot(path="install_step4.png")
            print(f"  当前页面: {page.url}")

            # 检查是否安装成功
            print("\n" + "=" * 60)
            print("检查安装结果")
            print("=" * 60)

            page.goto(BASE_URL, timeout=30000)
            page.wait_for_load_state("networkidle")

            if "install" not in page.url.lower():
                print("  安装成功!")
                page.screenshot(path="install_success.png")
            else:
                print("  安装可能未完成")
                print(f"  当前页面: {page.url}")

            print("\n" + "=" * 60)
            print("配置信息")
            print("=" * 60)
            print(f"  访问地址: {BASE_URL}")
            print(f"  管理员邮箱: {ADMIN_EMAIL}")
            print(f"  管理员密码: {ADMIN_PASSWORD}")

        except Exception as e:
            print(f"错误: {e}")
            page.screenshot(path="install_error.png")
            import traceback
            traceback.print_exc()
        finally:
            browser.close()

if __name__ == "__main__":
    main()
