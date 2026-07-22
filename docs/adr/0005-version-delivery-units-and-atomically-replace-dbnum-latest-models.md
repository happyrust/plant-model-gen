---
status: accepted
date: 2026-07-22
depends_on: ADR-0002
---

# 对最小交付单元追踪 sesno，并原子替换其他模型的 dbnum 最新汇总

BRAN、HANG、EQUI、WALL、FLOOR 根确定的最小交付单元继续以 `(dbnum, unit_refno, sesno)` 记录模型提交和 DuckLake 不可变导出物；其他模型类型不建立模型历史，仍按 dbnum 生成最新汇总模型。新的汇总模型只有在全部文件写入并验证完成后才通过原子替换 latest manifest 对 plant3d-web 可见，失败导出不得覆盖上一份可用模型；该 latest 指针不是 release、model_version_id 或另一套版本身份。
