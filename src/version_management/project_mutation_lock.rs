use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fs2::FileExt;
use serde::Serialize;

use crate::options::DbOptionExt;

#[derive(Debug, Serialize)]
struct LockOwner {
    pid: u32,
    started_at: String,
    command: String,
    project: String,
}

/// 进程级项目写入互斥锁。文件保留用于诊断，锁本身由 OS advisory lock 保证。
pub struct ProjectMutationLock {
    file: File,
    path: PathBuf,
}

#[derive(Clone, Copy)]
pub(crate) struct HeldProjectMutationLock<'a> {
    _lock: &'a ProjectMutationLock,
}

impl ProjectMutationLock {
    pub fn acquire(db_option: &DbOptionExt, command: impl Into<String>) -> Result<Self> {
        let path = lock_path(db_option);
        Self::acquire_at(&path, &db_option.inner.project_name, command.into())
    }

    pub fn acquire_for_current_command(db_option: &DbOptionExt) -> Result<Self> {
        Self::acquire(db_option, std::env::args().collect::<Vec<_>>().join(" "))
    }

    fn acquire_at(path: &Path, project: &str, command: String) -> Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("锁文件缺少父目录: {}", path.display()))?;
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建项目锁目录失败: {}", parent.display()))?;
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(path)
            .with_context(|| format!("打开项目写入锁失败: {}", path.display()))?;

        if let Err(error) = FileExt::try_lock_exclusive(&file) {
            let mut owner = String::new();
            let _ = file.seek(SeekFrom::Start(0));
            let _ = file.read_to_string(&mut owner);
            let owner = owner.trim();
            anyhow::bail!(
                "项目写入锁已被占用: {}（holder={}；error={}）。watch-incremental、incremental-sesno 与模型生成不可并发。",
                path.display(),
                if owner.is_empty() { "unknown" } else { owner },
                error
            );
        }

        let owner = LockOwner {
            pid: std::process::id(),
            started_at: chrono::Utc::now().to_rfc3339(),
            command,
            project: project.to_string(),
        };
        file.set_len(0)
            .with_context(|| format!("清空项目锁元数据失败: {}", path.display()))?;
        file.seek(SeekFrom::Start(0))
            .with_context(|| format!("定位项目锁文件失败: {}", path.display()))?;
        serde_json::to_writer_pretty(&mut file, &owner)
            .with_context(|| format!("写入项目锁元数据失败: {}", path.display()))?;
        file.write_all(b"\n")
            .with_context(|| format!("写入项目锁换行失败: {}", path.display()))?;
        file.sync_data()
            .with_context(|| format!("刷新项目锁元数据失败: {}", path.display()))?;

        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn held(&self) -> HeldProjectMutationLock<'_> {
        HeldProjectMutationLock { _lock: self }
    }
}

impl Drop for ProjectMutationLock {
    fn drop(&mut self) {
        if let Err(error) = FileExt::unlock(&self.file) {
            log::warn!(
                "释放项目写入锁失败(path={}): {}",
                self.path.display(),
                error
            );
        }
    }
}

pub fn lock_path(db_option: &DbOptionExt) -> PathBuf {
    db_option
        .output_root
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("output"))
        .join(&db_option.inner.project_name)
        .join("incremental.lock")
}
