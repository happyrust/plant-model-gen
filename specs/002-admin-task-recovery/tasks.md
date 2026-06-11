# Tasks

- [x] Add `recover_stale_tasks_on_startup` (UPDATE Pending/Running → Failed + error).
- [x] Wire it into `create_admin_task_routes` after table init; log recovered count.
- [ ] Manually verify: kill -9 during Running task → restart → site unblocked, retry works.
- [x] Format changed Rust files.
- [x] Update CHANGELOG.
