@echo off
echo 测试布尔运算实体方位显示修复
echo ================================

cargo run --bin aios-database -- --debug-model 17496_106028 --regen-model --export-obj --verbose

if exist "output\17496_106028.obj" (
    echo OBJ 文件已生成
    echo 检查顶点坐标...
    findstr "^v " "output\17496_106028.obj" | head -20
) else (
    echo 错误：OBJ 文件未生成
)
