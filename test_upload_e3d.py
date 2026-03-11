#!/usr/bin/env python3
"""E3D 文件上传和解析测试脚本"""

import sys
import time
import requests
from pathlib import Path

BASE_URL = "http://localhost:8080"

def upload_e3d(file_path: str, project_name: str = "test_project"):
    """上传 E3D 文件"""
    print(f"📤 上传文件: {file_path}")
    
    if not Path(file_path).exists():
        print(f"❌ 文件不存在: {file_path}")
        sys.exit(1)
    
    with open(file_path, 'rb') as f:
        files = {'file': f}
        data = {'project_name': project_name}
        
        response = requests.post(
            f"{BASE_URL}/api/upload/e3d",
            files=files,
            data=data,
            timeout=300
        )
    
    result = response.json()
    print(f"响应: {result}")
    
    if not result.get('success'):
        print(f"❌ 上传失败: {result.get('message')}")
        sys.exit(1)
    
    task_id = result.get('task_id')
    print(f"✅ 上传成功，任务ID: {task_id}\n")
    return task_id

def poll_task_status(task_id: str, max_attempts: int = 60):
    """轮询任务状态"""
    print(f"🔄 查询解析状态 (任务ID: {task_id})...")
    
    for attempt in range(1, max_attempts + 1):
        time.sleep(2)
        
        response = requests.get(f"{BASE_URL}/api/upload/task/{task_id}")
        result = response.json()
        
        if not result.get('success'):
            print(f"❌ 查询失败: {result.get('error_message')}")
            sys.exit(1)
        
        task = result.get('task', {})
        status = task.get('status')
        progress = task.get('progress', 0)
        message = task.get('message', '')
        
        print(f"[{attempt}/{max_attempts}] 状态: {status} | 进度: {progress:.1f}% | {message}")
        
        if status == 'completed':
            print(f"\n✅ 解析完成！")
            return True
        elif status == 'failed':
            print(f"\n❌ 解析失败: {message}")
            sys.exit(1)
    
    print(f"\n⏱️  超时：解析未在预期时间内完成")
    sys.exit(1)

def test_query_api():
    """测试数据查询 API"""
    print("\n🔍 测试数据查询 API...")
    
    # 查询 World Root
    print("查询 World Root...")
    response = requests.get(f"{BASE_URL}/api/e3d/world-root")
    print(f"响应: {response.json()}\n")

def main():
    if len(sys.argv) < 2:
        print("用法: python test_upload_e3d.py <e3d_file_path> [project_name]")
        print("示例: python test_upload_e3d.py test_data/sample.e3d my_project")
        sys.exit(1)
    
    file_path = sys.argv[1]
    project_name = sys.argv[2] if len(sys.argv) > 2 else "test_project"
    
    print("=" * 50)
    print("E3D 远程上传解析测试")
    print("=" * 50)
    print(f"服务器: {BASE_URL}")
    print(f"文件: {file_path}")
    print(f"项目: {project_name}\n")
    
    # 1. 上传文件
    task_id = upload_e3d(file_path, project_name)
    
    # 2. 轮询状态
    poll_task_status(task_id)
    
    # 3. 测试查询
    test_query_api()
    
    print("=" * 50)
    print("✅ 测试完成")
    print("=" * 50)

if __name__ == "__main__":
    main()
