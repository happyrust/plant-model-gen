#!/usr/bin/env python3
"""
Lsky-Pro 安装 - 简化版，只处理可见元素
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

            page.locator("text=下一步").click()
            print("  已点击'下一步'")
            time.sleep(3)

            # Step 2: 配置数据库和账号
            print("\nStep 2: 配置数据库和账号")

            # 选择 SQLite
            page.locator("select").first.select_option("sqlite")
            print("  已选择 SQLite")
            time.sleep(2)

            # 只填写可见的邮箱字段
            email_input = page.locator("input[type='email']:visible")
            if email_input.count() > 0:
                email_input.fill(ADMIN_EMAIL)
                print(f"  已填写邮箱: {ADMIN_EMAIL}")

            # 填写可见的密码字段
            pwd_input = page.locator("input[type='password']:visible")
            if pwd_input.count() > 0:
                pwd_input.fill(ADMIN_PASSWORD)
                print("  已填写密码")

            time.sleep(1)
            page.screenshot(path="lsky_ready.png", full_page=True)

            # 点击"立即安装"按钮
            print("\n点击'立即安装'按钮...")

            # 查找页面上所有 button 和它们的文本
            buttons = page.locator("button:visible").all()
            print(f"  可见按钮数量: {len(buttons)}")
            for i, btn in enumerate(buttons):
                try:
                    text = btn.inner_text()
                    print(f"    [{i}] {text}")
                except:
                    pass

            # 点击包含"安装"文字的按钮
            install_btn = page.locator("button:visible:has-text('安装')")
            if install_btn.count() > 0:
                install_btn.first.click()
                print("  已点击'安装'按钮")
            else:
                # 尝试使用 CSS 选择器找蓝色按钮
                all_visible_btns = page.locator("button:visible").all()
                if all_visible_btns:
                    # 点击最后一个可见按钮
                    all_visible_btns[-1].click()
                    print("  已点击最后一个可见按钮")

            # 等待安装
            print("\n等待安装完成...")
            time.sleep(15)

            page.screenshot(path="lsky_result.png", full_page=True)

            # 检查结果
            print("\n检查安装结果...")
            current_url = page.url
            print(f"  当前 URL: {current_url}")

            page.goto(BASE_URL, timeout=30000)
            time.sleep(3)

            final_url = page.url
            print(f"  最终 URL: {final_url}")

            page.screenshot(path="lsky_final.png", full_page=True)

            if "install" not in final_url.lower():
                print("\n" + "=" * 50)
                print("安装成功!")
                print("=" * 50)
                print(f"  访问地址: {BASE_URL}")
                print(f"  管理员邮箱: {ADMIN_EMAIL}")
                print(f"  管理员密码: {ADMIN_PASSWORD}")
            else:
                print("\n安装可能需要手动完成")
                body_text = page.inner_text("body")[:300]
                print(f"  页面: {body_text[:200]}...")

        except Exception as e:
            print(f"错误: {e}")
            page.screenshot(path="lsky_error.png", full_page=True)
            import traceback
            traceback.print_exc()
        finally:
            browser.close()

if __name__ == "__main__":
    main()
