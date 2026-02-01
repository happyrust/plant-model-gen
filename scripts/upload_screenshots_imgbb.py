#!/usr/bin/env python3
"""
上传截图到 ImgBB 图床

使用方法:
1. 访问 https://api.imgbb.com/ 获取免费 API Key
2. 设置环境变量: set IMGBB_API_KEY=your_api_key
3. 运行: python scripts/upload_screenshots_imgbb.py

ImgBB API 文档: https://api.imgbb.com/
- 端点: POST https://api.imgbb.com/1/upload
- 参数: key (API key), image (base64 或 URL)
- 限制: 单张 32MB
"""

import os
import sys
import base64
import json
import requests
from pathlib import Path

# ImgBB API 端点
IMGBB_API_URL = "https://api.imgbb.com/1/upload"

def upload_to_imgbb(image_path: Path, api_key: str) -> dict:
    """上传单张图片到 ImgBB"""
    with open(image_path, "rb") as f:
        image_data = base64.b64encode(f.read()).decode("utf-8")

    payload = {
        "key": api_key,
        "image": image_data,
        "name": image_path.stem,
    }

    response = requests.post(IMGBB_API_URL, data=payload, timeout=60)
    response.raise_for_status()
    return response.json()

def main():
    # 获取 API Key
    api_key = os.environ.get("IMGBB_API_KEY")
    if not api_key:
        print("错误: 请设置 IMGBB_API_KEY 环境变量")
        print("获取 API Key: https://api.imgbb.com/")
        sys.exit(1)

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

    print(f"找到 {len(png_files)} 个截图文件")
    print()

    results = []
    for png_path in png_files:
        print(f"上传: {png_path.name} ... ", end="", flush=True)
        try:
            result = upload_to_imgbb(png_path, api_key)
            if result.get("success"):
                url = result["data"]["url"]
                thumb_url = result["data"]["thumb"]["url"]
                delete_url = result["data"]["delete_url"]
                results.append({
                    "name": png_path.name,
                    "url": url,
                    "thumb": thumb_url,
                    "delete": delete_url,
                })
                print(f"成功")
                print(f"  URL: {url}")
            else:
                print(f"失败: {result}")
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
        output_json = screenshot_dir / "upload_results.json"
        with open(output_json, "w", encoding="utf-8") as f:
            json.dump(results, f, indent=2, ensure_ascii=False)
        print(f"结果已保存到: {output_json}")

if __name__ == "__main__":
    main()
