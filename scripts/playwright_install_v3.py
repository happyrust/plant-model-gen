#!/usr/bin/env python3
"""
使用 Playwright 自动完成 Lsky-Pro 安装向导 (改进版 - 处理滚动)
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
            # Step 1: 访问安装页面
            print("\n" + "=" * 60)
            print("Step 1: 环境检测")
            print("=" * 60)

            page.goto(f"{BASE_URL}/install", timeout=30000)
            page.wait_for_load_state("networkidle")
            time.sleep(2)
            print(f"  页面标题: {page.title()}")

            # 滚动到页面底部
            page.evaluate("window.scrollTo(0, document.body.scrollHeight)")
            time.sleep(1)

            page.screenshot(path="lsky_step1.png", full_page=True)
            print("  截图: lsky_step1.png")

            # 查找并点击"下一步"按钮 - 使用多种方式
            next_clicked = False

            # 方式1: 使用 JavaScript 查找并点击
            result = page.evaluate('''() => {
                const buttons = document.querySelectorAll('button, a, div[role="button"]');
                for (const btn of buttons) {
                    const text = btn.innerText || btn.textContent;
                    if (text && (text.includes('下一步') || text.includes('Next') || text.includes('继续'))) {
                        btn.click();
                        return 'clicked: ' + text;
                    }
                }
                // 尝试点击表单内的提交按钮
                const submit = document.querySelector('button[type="submit"], input[type="submit"]');
                if (submit) {
                    submit.click();
                    return 'clicked submit';
                }
                return 'not found';
            }''')
            print(f"  JS 点击结果: {result}")
            if 'clicked' in result:
                next_clicked = True

            time.sleep(3)
            page.screenshot(path="lsky_step2.png", full_page=True)
            print(f"  当前 URL: {page.url}")

            # Step 2: 数据库配置
            print("\n" + "=" * 60)
            print("Step 2: 数据库配置")
            print("=" * 60)

            # 检查页面内容
            body_text = page.inner_text("body")[:300]
            print(f"  页面内容: {body_text[:150]}...")

            # 查找数据库选择下拉框并选择 SQLite
            select_result = page.evaluate('''() => {
                const selects = document.querySelectorAll('select');
                for (const select of selects) {
                    const options = select.querySelectorAll('option');
                    for (const option of options) {
                        if (option.value === 'sqlite' || option.textContent.toLowerCase().includes('sqlite')) {
                            select.value = option.value;
                            select.dispatchEvent(new Event('change', { bubbles: true }));
                            return 'selected: ' + option.textContent;
                        }
                    }
                }
                return 'no select found';
            }''')
            print(f"  数据库选择结果: {select_result}")

            time.sleep(2)

            # 点击下一步
            page.evaluate("window.scrollTo(0, document.body.scrollHeight)")
            result = page.evaluate('''() => {
                const buttons = document.querySelectorAll('button, a');
                for (const btn of buttons) {
                    const text = btn.innerText || btn.textContent;
                    if (text && (text.includes('下一步') || text.includes('Next'))) {
                        btn.click();
                        return 'clicked';
                    }
                }
                return 'not found';
            }''')
            print(f"  下一步点击结果: {result}")

            time.sleep(3)
            page.screenshot(path="lsky_step3.png", full_page=True)

            # Step 3: 管理员账户
            print("\n" + "=" * 60)
            print("Step 3: 管理员账户设置")
            print("=" * 60)

            body_text = page.inner_text("body")[:300]
            print(f"  页面内容: {body_text[:150]}...")

            # 填写表单
            fill_result = page.evaluate(f'''() => {{
                let filled = [];

                // 填写邮箱
                const emailInputs = document.querySelectorAll('input[type="email"], input[name="email"], input[placeholder*="邮箱"]');
                if (emailInputs.length > 0) {{
                    emailInputs[0].value = "{ADMIN_EMAIL}";
                    emailInputs[0].dispatchEvent(new Event('input', {{ bubbles: true }}));
                    filled.push('email');
                }}

                // 填写名称
                const nameInputs = document.querySelectorAll('input[name="name"], input[placeholder*="名称"], input[placeholder*="用户名"]');
                if (nameInputs.length > 0) {{
                    nameInputs[0].value = "Admin";
                    nameInputs[0].dispatchEvent(new Event('input', {{ bubbles: true }}));
                    filled.push('name');
                }}

                // 填写密码
                const passwordInputs = document.querySelectorAll('input[type="password"]');
                if (passwordInputs.length >= 1) {{
                    passwordInputs[0].value = "{ADMIN_PASSWORD}";
                    passwordInputs[0].dispatchEvent(new Event('input', {{ bubbles: true }}));
                    filled.push('password');
                }}
                if (passwordInputs.length >= 2) {{
                    passwordInputs[1].value = "{ADMIN_PASSWORD}";
                    passwordInputs[1].dispatchEvent(new Event('input', {{ bubbles: true }}));
                    filled.push('confirm');
                }}

                return filled.join(', ');
            }}''')
            print(f"  表单填写结果: {fill_result}")

            time.sleep(1)
            page.screenshot(path="lsky_step4a.png", full_page=True)

            # 点击完成/安装按钮
            page.evaluate("window.scrollTo(0, document.body.scrollHeight)")
            submit_result = page.evaluate('''() => {
                const buttons = document.querySelectorAll('button, input[type="submit"]');
                for (const btn of buttons) {
                    const text = btn.innerText || btn.textContent || btn.value;
                    if (text && (text.includes('完成') || text.includes('安装') || text.includes('Submit') || text.includes('Finish'))) {
                        btn.click();
                        return 'clicked: ' + text;
                    }
                }
                // 尝试点击最后一个按钮
                if (buttons.length > 0) {
                    buttons[buttons.length - 1].click();
                    return 'clicked last button';
                }
                return 'not found';
            }''')
            print(f"  提交结果: {submit_result}")

            time.sleep(5)
            page.screenshot(path="lsky_step4b.png", full_page=True)

            # 检查结果
            print("\n" + "=" * 60)
            print("检查安装结果")
            print("=" * 60)

            page.goto(BASE_URL, timeout=30000)
            page.wait_for_load_state("networkidle")
            time.sleep(2)

            print(f"  最终 URL: {page.url}")
            print(f"  页面标题: {page.title()}")

            page.screenshot(path="lsky_final.png", full_page=True)

            if "install" not in page.url.lower():
                print("\n  安装成功!")
                print("\n" + "=" * 60)
                print("配置信息")
                print("=" * 60)
                print(f"  访问地址: {BASE_URL}")
                print(f"  管理员邮箱: {ADMIN_EMAIL}")
                print(f"  管理员密码: {ADMIN_PASSWORD}")
            else:
                print("\n  安装可能需要手动完成")
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
