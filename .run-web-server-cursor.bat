@echo off
call "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul
set AWS_LC_SYS_NO_ASM=1
set CARGO_INCREMENTAL=0
cd /d D:\work\plant-code\plant-model-gen
cargo run --bin web_server -- --config db_options/DbOption-cursor
