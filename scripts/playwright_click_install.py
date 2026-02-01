#!/usr/bin/env python3
"""
Lsky-Pro 安装 - 直接点击按钮
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
        context = browser.new_context(viewport={'width': 1280, 'height': 900})
        page = context.new_page()

        try:
            # Step 1: 环境检测 - 点击下一步
            print("\nStep 1: 环境检测")
            page.goto(f"{BASE_URL}/install", timeout=30000)
            page.wait_for_load_state("networkidle")
            time.sleep(2)

            # 使用 locator 直接点击下一步
            page.locator("text=下一步").click()
            print("  已点击'下一步'")
            time.sleep(3)

            # Step 2: 配置数据库和账号
            print("\nStep 2: 配置数据库和账号")

            # 选择 SQLite - 使用 select 元素
            page.locator("select").first.select_option("sqlite")
            print("  已选择 SQLite")
            time.sleep(2)

            # 清空数据库路径
            db_path_input = page.locator("input").first
            db_path_input.fill("")
            print("  已清空数据库路径")

            # 填写邮箱
            page.locator("input[type='email']").fill(ADMIN_EMAIL)
            print(f"  已填写邮箱: {ADMIN_EMAIL}")

            # 填写密码
            page.locator("input[type='password']").fill(ADMIN_PASSWORD)
            print("  已填写密码")

            time.sleep(1)
            page.screenshot(path="lsky_ready.png", full_page=True)

            # 点击"立即安装"按钮
            print("\n点击'立即安装'按钮...")

            # 使用 text 定位器
            install_btn = page.locator("text=立即安装")
            print(f"  找到按钮数量: {install_btn.count()}")

            if install_btn.count() > 0:
                install_btn.click()
                print("  已点击'立即安装'")
            else:
                # 备用方案：点击页面上的蓝色按钮
                blue_btn = page.locator("button.bg-blue-500, button.bg-primary, button[class*='blue']")
                if blue_btn.count() > 0:
                    blue_btn.click()
                    print("  已点击蓝色按钮")
                else:
                    # 最后方案：点击所有 button
                    all_btns = page.locator("button").all()
                    print(f"  找到 {len(all_btns)} 个按钮")
                    for btn in all_btns:
                        text = btn.inner_text()
                        print(f"    - {text}")
                    if all_btns:
                        all_btns[-1].click()
                        print("  已点击最后一个按钮")

            # 等待安装
            print("\n等待安装完成...")
            time.sleep(10)

            page.screenshot(path="lsky_result.png", full_page=True)

            # 检查结果
            print("\n检查安装结果...")
            page.goto(BASE_URL, timeout=30000)
            time.sleep(3)

            final_url = page.url
            print(f"  最终 URL: {final_url}")

            page.screenshot(path="lsky_final.png", full_page=True)

            if "install" not in final_url.lower():
                print("\n安装成功!")
                print(f"  访问地址: {BASE_URL}")
                print(f"  管理员邮箱: {ADMIN_EMAIL}")
                print(f"  管理员密码: {ADMIN_PASSWORD}")
            else:
                print("\n安装可能需要手动完成")
                print(f"  请访问: {BASE_URL}/install")

        except Exception as e:
            print(f"错误: {e}")
            page.screenshot(path="lsky_error.png", full_page=True)
            import traceback
            traceback.print_exc()
        finally:
            browser.close()

if __name__ == "__main__":
    main()
