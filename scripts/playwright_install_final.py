#!/usr/bin/env python3
"""
使用 Playwright 自动完成 Lsky-Pro 安装向导 (最终版)
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
            # Step 1: 环境检测
            print("\n" + "=" * 60)
            print("Step 1: 环境检测")
            print("=" * 60)

            page.goto(f"{BASE_URL}/install", timeout=30000)
            page.wait_for_load_state("networkidle")
            time.sleep(2)
            print(f"  页面标题: {page.title()}")

            # 滚动并点击"下一步"
            page.evaluate("window.scrollTo(0, document.body.scrollHeight)")
            time.sleep(1)

            result = page.evaluate('''() => {
                const buttons = document.querySelectorAll('button, a');
                for (const btn of buttons) {
                    const text = btn.innerText || btn.textContent;
                    if (text && text.includes('下一步')) {
                        btn.click();
                        return 'clicked: ' + text;
                    }
                }
                return 'not found';
            }''')
            print(f"  点击结果: {result}")

            time.sleep(3)

            # Step 2: 配置数据库和管理员账号
            print("\n" + "=" * 60)
            print("Step 2: 配置数据库和管理员账号")
            print("=" * 60)

            # 选择 SQLite
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
            print(f"  数据库选择: {select_result}")
            time.sleep(2)

            # 填写表单 - 注意：数据库路径留空，只填写邮箱和密码
            fill_result = page.evaluate(f'''() => {{
                let filled = [];
                const inputs = document.querySelectorAll('input');

                for (const input of inputs) {{
                    const name = input.name || '';
                    const placeholder = input.placeholder || '';
                    const type = input.type || '';

                    // 清空数据库路径字段
                    if (name.includes('database') || placeholder.includes('路径') || placeholder.includes('名称')) {{
                        input.value = '';
                        input.dispatchEvent(new Event('input', {{ bubbles: true }}));
                        filled.push('cleared db path');
                    }}

                    // ��写邮箱
                    if (type === 'email' || name.includes('email') || placeholder.includes('邮箱')) {{
                        input.value = "{ADMIN_EMAIL}";
                        input.dispatchEvent(new Event('input', {{ bubbles: true }}));
                        filled.push('email');
                    }}

                    // 填写密码
                    if (type === 'password') {{
                        input.value = "{ADMIN_PASSWORD}";
                        input.dispatchEvent(new Event('input', {{ bubbles: true }}));
                        filled.push('password');
                    }}
                }}

                return filled.join(', ');
            }}''')
            print(f"  表单填写: {fill_result}")

            time.sleep(1)
            page.screenshot(path="lsky_before_submit.png", full_page=True)

            # 点击"立即安装"按钮
            print("\n  点击'立即安装'按钮...")
            submit_result = page.evaluate('''() => {
                const buttons = document.querySelectorAll('button');
                for (const btn of buttons) {
                    const text = btn.innerText || btn.textContent;
                    if (text && (text.includes('立即安装') || text.includes('安装') || text.includes('Install'))) {
                        btn.click();
                        return 'clicked: ' + text;
                    }
                }
                return 'not found';
            }''')
            print(f"  提交结果: {submit_result}")

            # 等待安装完成
            print("  等待安装完成...")
            time.sleep(10)

            page.screenshot(path="lsky_after_submit.png", full_page=True)

            # 检查结果
            print("\n" + "=" * 60)
            print("检查安装结果")
            print("=" * 60)

            current_url = page.url
            print(f"  当前 URL: {current_url}")

            # 尝试访问首页
            page.goto(BASE_URL, timeout=30000)
            page.wait_for_load_state("networkidle")
            time.sleep(2)

            final_url = page.url
            print(f"  最终 URL: {final_url}")
            print(f"  页面标题: {page.title()}")

            page.screenshot(path="lsky_final.png", full_page=True)

            if "install" not in final_url.lower():
                print("\n  安装成功!")
                print("\n" + "=" * 60)
                print("配置信息")
                print("=" * 60)
                print(f"  访问地址: {BASE_URL}")
                print(f"  管理员邮箱: {ADMIN_EMAIL}")
                print(f"  管理员密码: {ADMIN_PASSWORD}")
                print()
                print("下一步: 登录后台获取 API Token")
                print("  后台 -> 设置 -> 接口 -> 生成 Token")
            else:
                print("\n  安装可能失败或需要手动完成")
                # 检查是否有错误信息
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
