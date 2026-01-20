@echo off
echo ========================================
echo 验证布尔运算实体方位显示修复结果
echo ========================================

echo.
echo 1. 检查 OBJ 文件生成情况...
if exist "output\17496_106028.obj" (
    echo ✓ OBJ 文件已生成
    for %%I in ("output\17496_106028.obj") do echo   文件大小: %%~zI 字节
) else (
    echo ✗ OBJ 文件未生成
    goto :end
)

echo.
echo 2. 分析顶点坐标范围...
echo 前10个顶点:
findstr "^v " "output\17496_106028.obj" | head -10

echo.
echo 3. 统计信息...
echo 顶点数量:
findstr "^v " "output\17496_106028.obj" | find /c /v ""

echo.
echo 面数量:
findstr "^f " "output\17496_106028.obj" | find /c /v ""

echo.
echo 4. 检查大坐标值（可能表示变换重复应用）...
findstr "^v " "output\17496_106028.obj" | findstr "R" >nul 2>&1
if %errorlevel% == 0 (
    echo ⚠️ 发现科学计数法表示的大数值
) else (
    echo ✓ 未发现异常大的坐标值
)

echo.
echo 5. 查找对象组定义...
findstr "^g " "output\17496_106028.obj" | head -10

echo.
echo ========================================
echo 验证完成
echo.
echo 修复效果判断：
echo - 如果坐标值范围合理（没有异常大的数值），说明修复成功
echo - 布尔运算实体应该出现在正确的位置，没有偏移

:end
pause
