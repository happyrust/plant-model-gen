#!/usr/bin/env python3
"""
渲染 OBJ 模型并生成截图
"""

import os
import sys
import numpy as np
import trimesh
from pathlib import Path

# 要渲染的 OBJ 文件列表
OBJ_FILES = [
    "17496_161470.obj",
    "17496_161478.obj",
    "17496_161482.obj",
    "17496_161486.obj",
    "17496_161513.obj",
    "17496_161522.obj",
    "17496_161727.obj",
    "Copy-of--CCV-S-2-H-1103.obj",
    "Copy-of--CCV-S-2-L-1104.obj",
]

def render_obj_to_png(obj_path: Path, output_path: Path, resolution=(1920, 1080)):
    """使用 trimesh 渲染 OBJ 文件为 PNG"""
    try:
        # 加载模型
        mesh = trimesh.load(str(obj_path), force='mesh')

        if mesh.is_empty:
            print(f"  [WARN] 模型为空: {obj_path.name}")
            return False

        # 获取模型信息
        bounds = mesh.bounds
        extents = mesh.extents
        center = mesh.centroid

        print(f"  模型中心: {center}")
        print(f"  模型尺寸: {extents}")
        print(f"  顶点数: {len(mesh.vertices)}")
        print(f"  面数: {len(mesh.faces)}")

        # 创建场景
        scene = trimesh.Scene(mesh)

        # 尝试渲染
        try:
            # 使用 pyrender 或 pyglet 后端
            png_data = scene.save_image(resolution=resolution, visible=False)

            with open(str(output_path), 'wb') as f:
                f.write(png_data)

            print(f"  [OK] 截图已保存: {output_path}")
            return True
        except Exception as e:
            print(f"  [WARN] 渲染失败 ({type(e).__name__}): {e}")
            # 尝试备用方法 - 使用 matplotlib
            return render_with_matplotlib(mesh, output_path)

    except Exception as e:
        print(f"  [ERROR] 加载模型失败: {e}")
        return False

def render_with_matplotlib(mesh, output_path: Path):
    """使用 matplotlib 进行简单的线框渲染"""
    try:
        import matplotlib.pyplot as plt
        from mpl_toolkits.mplot3d import Axes3D
        from mpl_toolkits.mplot3d.art3d import Poly3DCollection

        fig = plt.figure(figsize=(16, 9), dpi=120)
        ax = fig.add_subplot(111, projection='3d')

        # 获取顶点和面
        vertices = mesh.vertices
        faces = mesh.faces

        # 限制面数以避免渲染过慢
        max_faces = 5000
        if len(faces) > max_faces:
            # 随机采样
            indices = np.random.choice(len(faces), max_faces, replace=False)
            faces = faces[indices]
            print(f"  [INFO] 面数过多，采样 {max_faces} 个面进行渲染")

        # 创建多边形集合
        polygons = vertices[faces]

        # 添加多边形
        collection = Poly3DCollection(polygons,
                                       alpha=0.8,
                                       facecolor='steelblue',
                                       edgecolor='darkblue',
                                       linewidth=0.1)
        ax.add_collection3d(collection)

        # 设置坐标轴范围
        bounds = mesh.bounds
        center = mesh.centroid
        max_extent = max(mesh.extents) / 2 * 1.2

        ax.set_xlim(center[0] - max_extent, center[0] + max_extent)
        ax.set_ylim(center[1] - max_extent, center[1] + max_extent)
        ax.set_zlim(center[2] - max_extent, center[2] + max_extent)

        # 设置视角
        ax.view_init(elev=30, azim=45)

        # 设置标题
        ax.set_title(output_path.stem, fontsize=14)
        ax.set_xlabel('X')
        ax.set_ylabel('Y')
        ax.set_zlabel('Z')

        # 保存
        plt.savefig(str(output_path), dpi=120, bbox_inches='tight',
                   facecolor='white', edgecolor='none')
        plt.close(fig)

        print(f"  [OK] 截图已保存 (matplotlib): {output_path}")
        return True

    except Exception as e:
        print(f"  [ERROR] matplotlib 渲染失败: {e}")
        return False

def main():
    # 设置路径
    script_dir = Path(__file__).parent
    project_dir = script_dir.parent
    output_dir = project_dir / "output"
    screenshot_dir = output_dir / "screenshots"

    # 创建截图目录
    screenshot_dir.mkdir(parents=True, exist_ok=True)
    print(f"截图保存目录: {screenshot_dir}")
    print()

    success_count = 0
    fail_count = 0

    for obj_name in OBJ_FILES:
        obj_path = output_dir / obj_name
        if not obj_path.exists():
            print(f"[SKIP] 文件不存在: {obj_name}")
            fail_count += 1
            continue

        print(f"[RENDER] {obj_name}")

        # 生成截图路径
        png_name = obj_path.stem + ".png"
        png_path = screenshot_dir / png_name

        if render_obj_to_png(obj_path, png_path):
            success_count += 1
        else:
            fail_count += 1

        print()

    print("=" * 50)
    print(f"渲染完成: 成功 {success_count}, 失败 {fail_count}")
    print(f"截图目录: {screenshot_dir}")

if __name__ == "__main__":
    main()
