# Facts — 全库 ref0/dbnum 预扫描索引 + 精确按需解析关联库

- 站点级独立 SQLite `db_index.sqlite`（`runtime/admin_sites/<site>/`）存在，含 `db_file_index(dbnum, db_type, file_name, file_path, project, latest_sesno, fingerprint, scanned_at)` 与 `ref0_owner(ref0, dbnum)` 两表；同一站点的多个工程共享同一份索引。
- 预扫描以 index-only 方式工作：仅读 db 文件头取 dbnum/db_type（`parse_file_basic_info`）+ 遍历 B+树索引页取 owned ref0（`PdmsIO::collect_refno_locs`），不解析元素记录与属性。
- 全局 `ref0→dbnum` 覆盖站点内全部 db 文件，包括尚未导入 SurrealDB 的元件库/字典库/规格库。
- 给定一个设计库，系统能输出它精确依赖的外部 dbnum 列表（解析设计库时收集其外向 refno，经全局 `ref0→dbnum` 反查得到），结果不包含未被引用的库。
- 依赖推导为递归传递闭包：设计库→直接依赖→间接依赖逐层展开直到无新增，带去重与环检测。
- `auto_parse_related_dbnums` 开启时，`included_db_files` 由精确依赖结果驱动，替换原 `RELATED_DEPENDENCY_DB_TYPES` 按 CATA/DICT 类型全量纳入的粗粒度逻辑（`managed_project_sites.rs:1393`）。
- 不论 `auto_parse_related_dbnums` 开关是否开启，SYST + DICT + GLOB/GLB（工程存在则）恒被纳入解析。
- 预扫描在站点解析前自动触发；按 `latest_sesno + 文件 mtime/size` 指纹仅增量重扫发生变更的 db，未变更的库复用缓存。
- 提供 CLI 子命令可触发索引重建/强制全量重扫。
- admin 提供『重建索引』按钮，触发与 CLI 相同的重建逻辑。（手动验证）
- `db_meta_info.json` 仍作为解析期产物保留，预扫描索引不破坏既有 `ref0_to_dbnum` 使用方（`db_meta_manager` 等），无功能回归。
