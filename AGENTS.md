---
description: 
alwaysApply: true
---

# Repository Guidelines
 不要使用 test 或者编译任何 test,  任何时候，针对 web_server 都要使用运行起来，然后使用 post 去测试，而不是使用 test
 针对 aios-database，使用 cli + json的方式去测试验证。

## 部署目标服务器 
- 服务器：`123.57.182.243`
- SSH 用户：`root`
- SSH 密码：仅通过环境变量 / CI Secrets 提供（禁止写入仓库）
