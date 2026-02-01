#!/usr/bin/env python3
"""
上传截图到自部署的 Lsky-Pro 图床

使用方法:
1. 部署 Lsky-Pro (见 lsky-pro-docker-compose.yml)
2. 在 Lsky-Pro 后台获取 API Token
3. 设置环境变量:
   set LSKY_URL=http://your-server:8089
   set LSKY_TOKEN=your_api_token
4. 运行: python scripts/upload_screenshots_lsky.py

Lsky-Pro API 文档: https://docs.lsky.pro/docs/free/v2/api/
"""

import os
import sys
import json
import requests
from pathlib import Path

def get_config():
    """获取配置"""
    lsky_url = os.environ.get("LSKY_URL")
    lsky_token = os.environ.get("LSKY_TOKEN")

    if not lsky_url:
        print("错误: 请设置 LSKY_URL 环境变量 (例如: http://your-server:8089)")
        sys.exit(1)
    if not lsky_token:
        print("错误: 请设置 LSKY_TOKEN 环境变量")
        print("在 Lsky-Pro 后台 -> 接口 -> 获取 Token")
        sys.exit(1)

    return lsky_url.rstrip("/"), lsky_token

def upload_to_lsky(image_path: Path, base_url: str, token: str) -> dict:
    """上传单张图片到 Lsky-Pro"""
    upload_url = f"{base_url}/api/v1/upload"

    headers = {
        "Authorization": f"Bearer {token}",
        "Accept": "application/json",
    }

    with open(image_path, "rb") as f:
        files = {
            "file": (image_path.name, f, "image/png")
        }
        response = requests.post(upload_url, headers=headers, files=files, timeout=60)

    response.raise_for_status()
    return response.json()

def main():
    base_url, token = get_config()

    # 截图目录
    script_dir = Path(__file__).parent
    project_dir = script_dir.parent
    screenshot_dir = project_dir / "output" / "screenshots"

    if not screenshot_dir.exists():
        print(f"错误: 截图目录不存在: {screenshot_dir}")
        sys.exit(1)

    # 获取所有 PNG 文件
    png_files = sorted(screenshot_dir.glob("*.png"))
    if not png_files:
        print("错误: 没有找到 PNG 文件")
        sys.exit(1)

    print(f"Lsky-Pro 服务器: {base_url}")
    print(f"找到 {len(png_files)} 个截图文件")
    print()

    results = []
    for png_path in png_files:
        print(f"上传: {png_path.name} ... ", end="", flush=True)
        try:
            result = upload_to_lsky(png_path, base_url, token)
            if result.get("status"):
                data = result["data"]
                url = data.get("links", {}).get("url", data.get("url", ""))
                thumb = data.get("links", {}).get("thumbnail_url", "")
                results.append({
                    "name": png_path.name,
                    "url": url,
                    "thumb": thumb,
                    "key": data.get("key", ""),
                })
                print(f"成功")
                print(f"  URL: {url}")
            else:
                print(f"失败: {result.get('message', 'Unknown error')}")
        except requests.exceptions.RequestException as e:
            print(f"网络错误: {e}")
        except Exception as e:
            print(f"错误: {e}")

    print()
    print("=" * 60)
    print(f"上传完成: {len(results)}/{len(png_files)}")
    print()

    # 生成 Markdown 格式的图片列表
    if results:
        print("Markdown 格式:")
        print()
        for r in results:
            print(f"![{r['name']}]({r['url']})")
        print()

        # 保存结果到 JSON
        output_json = screenshot_dir / "lsky_upload_results.json"
        with open(output_json, "w", encoding="utf-8") as f:
            json.dump(results, f, indent=2, ensure_ascii=False)
        print(f"结果已保存到: {output_json}")

if __name__ == "__main__":
    main()
