#!/bin/bash
echo "测试布尔运算实体方位显示修复"
echo "================================"

# 运行命令
cargo run --bin aios-database -- --debug-model 17496_106028 --regen-model --export-obj --verbose

# 检查输出文件
if [ -f "output/17496_106028.obj" ]; then
    echo "OBJ 文件已生成"
    # 检查文件内容中的顶点坐标
    head -100 output/17496_106028.obj | grep "^v "
else
    echo "错误：OBJ 文件未生成"
fi
