@echo off
echo ========================================
echo 验证布尔运算实体方位显示修复
echo ========================================

echo.
echo 1. 检查代码修改...
findstr /C:"bool_set.contains" src\fast_model\export_model\export_common.rs >nul
if %errorlevel% == 0 (
    echo [OK] export_common.rs 已正确修改
) else (
    echo [ERROR] export_common.rs 修改失败
)

findstr /C:"bool_set.contains" src\fast_model\export_model\export_prepack_lod.rs >nul
if %errorlevel% == 0 (
    echo [OK] export_prepack_lod.rs 已正确修改
) else (
    echo [ERROR] export_prepack_lod.rs 修改失败
)

echo.
echo 2. 运行测试命令...
echo 命令: cargo run --bin aios-database -- --debug-model 17496_106028 --regen-model --export-obj
echo.

REM 运行测试命令
cargo run --bin aios-database -- --debug-model 17496_106028 --regen-model --export-obj

echo.
echo 3. 检查输出文件...
if exist "output\17496_106028.obj" (
    echo [OK] OBJ 文件已生成
    echo.
    echo 前10个顶点坐标:
    findstr "^v " "output\17496_106028.obj" | head -10
) else (
    echo [ERROR] OBJ 文件未生成
)

echo.
echo ========================================
echo 验证完成
pause
