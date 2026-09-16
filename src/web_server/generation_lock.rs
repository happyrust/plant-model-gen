//! web 侧模型生成的项目写入互斥。
//!
//! 模型生成会改写进程级全局状态（pe 层级快照、transform cache、cache-miss
//! 报告）以及 inst_relate / geo_relate / tubi_relate，与 CLI 的
//! `watch-incremental`、`incremental-sesno` 属于同一互斥域。
//!
//! 锁只能加在 web 入口，**不能**下沉到 `gen_all_geos_data` 内部：CLI 的
//! `run_generate_model` / `run_regen_model` / `run_increment` /
//! `catch_up_model_generation` 进来时已经持有同一把锁，而
//! `try_lock_exclusive` 对同一进程的第二个文件句柄同样会失败，下沉即自锁。

use crate::options::DbOptionExt;
use crate::version_management::project_mutation_lock::{
    ProjectMutationLock, is_mutation_contention_error,
};

/// 为一个 web 生成入口抢项目写入锁；`endpoint` 会写进锁文件的 holder 字段。
///
/// 不要改用 `acquire_for_current_command`：它记录的是 web server 自己的命令
/// 行，锁文件里看不出是哪个接口占的。
pub fn acquire_web_generation_lock(
    db_option: &DbOptionExt,
    endpoint: &str,
) -> anyhow::Result<ProjectMutationLock> {
    ProjectMutationLock::acquire(db_option, format!("web:{endpoint}"))
}

/// 抢锁失败是否属于「另有写者正在跑」的正常让路，而不是真实故障。
pub fn is_generation_busy(error: &anyhow::Error) -> bool {
    is_mutation_contention_error(&error.to_string())
}
