# 验证布尔运算实体方位显示修复
Write-Host "========================================"
Write-Host "验证布尔运算实体方位显示修复"
Write-Host "========================================"

# 检查代码修改
Write-Host "`n1. 检查代码修改..."
$file1 = "src\fast_model\export_model\export_common.rs"
$file2 = "src\fast_model\export_model\export_prepack_lod.rs"

if (Select-String -Path $file1 -Pattern "bool_set.contains" -Quiet) {
    Write-Host "✓ export_common.rs 已正确修改"
} else {
    Write-Host "✗ export_common.rs 修改失败"
}

if (Select-String -Path $file2 -Pattern "bool_set.contains" -Quiet) {
    Write-Host "✓ export_prepack_lod.rs 已正确修改"
} else {
    Write-Host "✗ export_prepack_lod.rs 修改失败"
}

# 尝试编译
Write-Host "`n2. 尝试编译..."
$compileResult = cargo check 2>&1
if ($LASTEXITCODE -eq 0) {
    Write-Host "✓ 编译成功"
} else {
    Write-Host "✗ 编译失败:"
    Write-Host $compileResult
}

# 运行测试命令
Write-Host "`n3. 运行测试命令..."
Write-Host "执行: cargo run --bin aios-database -- --debug-model 17496_106028 --regen-model --export-obj --verbose"
$testResult = cargo run --bin aios-database -- --debug-model 17496_106028 --regen-model --export-obj --verbose 2>&1
Write-Host $testResult

# 检查输出文件
Write-Host "`n4. 检查输出文件..."
$objFile = "output\17496_106028.obj"
if (Test-Path $objFile) {
    Write-Host "✓ OBJ 文件已生成: $objFile"
    
    # 显示前20个顶点
    Write-Host "`n前20个顶点坐标:"
    $vertices = Select-String -Path $objFile -Pattern "^v " | Select-Object -First 20
    $vertices | ForEach-Object { Write-Host $_.Line }
    
    # 统计顶点数量
    $vertexCount = (Select-String -Path $objFile -Pattern "^v ").Count
    Write-Host "`n总顶点数: $vertexCount"
} else {
    Write-Host "✗ OBJ 文件未生成"
}

Write-Host "`n========================================"
Write-Host "验证完成"
