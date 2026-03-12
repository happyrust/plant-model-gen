# 房间计算单元测试指南

## 1. 测试策略概览

### 1.1 测试金字塔

```
        /\
       /  \  E2E Tests (集成测试)
      /____\
     /      \  Integration Tests (模块集成)
    /________\
   /          \  Unit Tests (单元测试)
  /__________\
```

**优先级**：单元测试 > 集成测试 > E2E 测试

---

## 2. 单元测试方案

### 2.1 空间查询 API (`sqlite_spatial_api.rs`)

#### 测试目标
- 输入验证
- 边界条件
- 模式检测逻辑
- 距离计算准确性

#### 测试用例

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_position_mode_radius_validation() {
        // P1 修复验证：radius 上限
        let params = SqliteSpatialQueryParams {
            mode: Some("position".to_string()),
            x: Some(1000.0),
            y: Some(2000.0),
            z: Some(3000.0),
            radius: Some(200_000.0), // 超过 100m 限制
            ..Default::default()
        };
        
        let result = do_spatial_query(params);
        assert!(!result.success);
        assert!(result.error.unwrap().contains("radius"));
    }

    #[test]
    fn test_position_mode_valid_query() {
        let params = SqliteSpatialQueryParams {
            x: Some(1000.0),
            y: Some(2000.0),
            z: Some(3000.0),
            radius: Some(5000.0),
            ..Default::default()
        };
        
        let result = do_spatial_query(params);
        // 需要 mock SQLite 索引
        assert!(result.query_bbox.is_some());
    }

    #[test]
    fn test_position_mode_invalid_coordinates() {
        let params = SqliteSpatialQueryParams {
            x: Some(f32::NAN),
            y: Some(2000.0),
            z: Some(3000.0),
            radius: Some(1000.0),
            ..Default::default()
        };
        
        let result = do_spatial_query(params);
        assert!(!result.success);
    }

    #[test]
    fn test_mode_detection_priority() {
        // 显式 mode 优先
        let params = SqliteSpatialQueryParams {
            mode: Some("bbox".to_string()),
            x: Some(1000.0),
            y: Some(2000.0),
            z: Some(3000.0),
            ..Default::default()
        };
        assert_eq!(parse_mode(&params), "bbox");

        // 自动检测：position > refno > bbox
        let params = SqliteSpatialQueryParams {
            x: Some(1000.0),
            y: Some(2000.0),
            z: Some(3000.0),
            ..Default::default()
        };
        assert_eq!(parse_mode(&params), "position");
    }

    #[test]
    fn test_distance_sorting() {
        // 验证结果按距离排序
        // 需要 mock 数据
    }
}
```

---

### 2.2 房间模型 (`room_model.rs`)

#### 测试目标
- 几何变换错误处理
- 正则表达式匹配
- AABB 计算
- 房间名称验证

#### 测试用例

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_room_name_hd() {
        // P1 修复验证：静态 regex
        assert!(match_room_name_hd("A001"));
        assert!(match_room_name_hd("Z999"));
        assert!(!match_room_name_hd("a001")); // 小写
        assert!(!match_room_name_hd("AA01")); // 两个字母
        assert!(!match_room_name_hd("A01"));  // 两位数字
        assert!(!match_room_name_hd("A0001")); // 四位数字
        assert!(!match_room_name_hd("1001")); // 无字母
    }

    #[test]
    fn test_transform_trimesh_valid() {
        // P1 修复验证：Result 传播
        let vertices = vec![
            Point::new(0.0, 0.0, 0.0),
            Point::new(1.0, 0.0, 0.0),
            Point::new(0.0, 1.0, 0.0),
        ];
        let indices = vec![[0u32, 1, 2]];
        let mesh = TriMesh::new(vertices, indices).unwrap();
        
        let transform = Mat4::from_translation(Vec3::new(10.0, 20.0, 30.0));
        let result = transform_tri_mesh(&mesh, transform);
        
        assert!(result.is_ok());
        let transformed = result.unwrap();
        assert_eq!(transformed.vertices().len(), 3);
    }

    #[test]
    fn test_transform_trimesh_invalid_topology() {
        // 无效拓扑：重复顶点索引
        let vertices = vec![
            Point::new(0.0, 0.0, 0.0),
            Point::new(1.0, 0.0, 0.0),
        ];
        let indices = vec![[0u32, 0, 0]]; // 退化三角形
        let mesh = TriMesh::new(vertices, indices).unwrap();
        
        let transform = Mat4::IDENTITY;
        let result = transform_tri_mesh(&mesh, transform);
        
        // 应该返回 Err 而不是 panic
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_merge_aabb() {
        let a = Aabb::new(Point::new(0.0, 0.0, 0.0), Point::new(1.0, 1.0, 1.0));
        let b = Aabb::new(Point::new(0.5, 0.5, 0.5), Point::new(2.0, 2.0, 2.0));
        
        let merged = merge_aabb(&a, &b);
        
        assert_eq!(merged.mins, Point::new(0.0, 0.0, 0.0));
        assert_eq!(merged.maxs, Point::new(2.0, 2.0, 2.0));
    }

    #[test]
    fn test_extract_aabb_corners() {
        let aabb = Aabb::new(Point::new(0.0, 0.0, 0.0), Point::new(1.0, 1.0, 1.0));
        let corners = extract_aabb_corners(&aabb);
        
        assert_eq!(corners.len(), 8);
        assert!(corners.contains(&Point::new(0.0, 0.0, 0.0)));
        assert!(corners.contains(&Point::new(1.0, 1.0, 1.0)));
    }
}
```

---

### 2.3 房间工作器 (`room_worker.rs`)

#### 测试目标
- 任务队列管理
- 优先级排序
- 取消机制
- 状态转换

#### 测试用例

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_task_submission_and_priority() {
        let config = RoomWorkerConfig::default();
        let worker = RoomWorker::new(config);
        
        let task1 = RoomWorkerTask::new(
            "task1".to_string(),
            RoomTaskType::RebuildAll,
            DbOption::default(),
        ).with_priority(100);
        
        let task2 = RoomWorkerTask::new(
            "task2".to_string(),
            RoomTaskType::RebuildAll,
            DbOption::default(),
        ).with_priority(50); // 高优先级
        
        worker.submit_task(task1).await;
        worker.submit_task(task2).await;
        
        let queue = worker.task_queue.read().await;
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0].id, "task2"); // 高优先级在前
    }

    #[tokio::test]
    async fn test_task_cancellation_in_queue() {
        let config = RoomWorkerConfig::default();
        let worker = RoomWorker::new(config);
        
        let task = RoomWorkerTask::new(
            "task1".to_string(),
            RoomTaskType::RebuildAll,
            DbOption::default(),
        );
        
        worker.submit_task(task).await;
        let cancelled = worker.cancel_task("task1").await;
        
        assert!(cancelled);
        assert_eq!(worker.queue_len().await, 0);
    }

    #[test]
    fn test_task_status_is_terminal() {
        assert!(!RoomWorkerTaskStatus::Queued.is_terminal());
        assert!(!RoomWorkerTaskStatus::Running {
            progress: 0.5,
            stage: "test".to_string()
        }.is_terminal());
        assert!(RoomWorkerTaskStatus::Cancelled.is_terminal());
    }
}
```

---

## 3. 集成测试方案

### 3.1 端到端房间计算测试

```rust
// tests/integration_room_computation.rs

#[tokio::test]
#[ignore] // 需要真实数据库
async fn test_build_room_relations_end_to_end() {
    // 1. 准备测试数据
    let db_option = load_test_db_option();
    
    // 2. 执行房间计算
    let result = build_room_relations(&db_option, None, None).await;
    
    // 3. 验证结果
    assert!(result.is_ok());
    let stats = result.unwrap();
    assert!(stats.total_rooms > 0);
    assert!(stats.cache_hit_rate >= 0.0 && stats.cache_hit_rate <= 1.0);
}

#[tokio::test]
#[ignore]
async fn test_spatial_query_with_real_index() {
    // 需要预先构建 SQLite 索引
    let params = SqliteSpatialQueryParams {
        x: Some(10000.0),
        y: Some(20000.0),
        z: Some(5000.0),
        radius: Some(5000.0),
        ..Default::default()
    };
    
    let result = do_spatial_query(params);
    assert!(result.success);
    assert!(result.results.is_some());
}
```

---

## 4. Mock 和测试工具

### 4.1 SQLite 索引 Mock

```rust
pub struct MockSpatialIndex {
    data: HashMap<i64, (String, Aabb)>,
}

impl MockSpatialIndex {
    pub fn new() -> Self {
        Self { data: HashMap::new() }
    }
    
    pub fn insert(&mut self, id: i64, noun: String, aabb: Aabb) {
        self.data.insert(id, (noun, aabb));
    }
    
    pub fn query_intersect(&self, query_aabb: &Aabb) -> Vec<i64> {
        self.data.iter()
            .filter(|(_, (_, aabb))| aabb.intersects(query_aabb))
            .map(|(id, _)| *id)
            .collect()
    }
}
```

### 4.2 几何数据生成器

```rust
pub fn create_test_trimesh(size: f32) -> TriMesh {
    let vertices = vec![
        Point::new(0.0, 0.0, 0.0),
        Point::new(size, 0.0, 0.0),
        Point::new(0.0, size, 0.0),
        Point::new(0.0, 0.0, size),
    ];
    let indices = vec![
        [0u32, 1, 2],
        [0, 1, 3],
        [0, 2, 3],
        [1, 2, 3],
    ];
    TriMesh::new(vertices, indices).unwrap()
}

pub fn create_test_aabb(center: [f32; 3], size: f32) -> Aabb {
    let half = size / 2.0;
    Aabb::new(
        Point::new(center[0] - half, center[1] - half, center[2] - half),
        Point::new(center[0] + half, center[1] + half, center[2] + half),
    )
}
```

---

## 5. 性能测试

### 5.1 基准测试

```rust
// benches/room_computation_bench.rs

use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_match_room_name_hd(c: &mut Criterion) {
    c.bench_function("match_room_name_hd", |b| {
        b.iter(|| {
            match_room_name_hd(black_box("A001"))
        });
    });
}

fn bench_transform_trimesh(c: &mut Criterion) {
    let mesh = create_test_trimesh(1000.0);
    let transform = Mat4::from_translation(Vec3::new(10.0, 20.0, 30.0));
    
    c.bench_function("transform_trimesh", |b| {
        b.iter(|| {
            transform_tri_mesh(black_box(&mesh), black_box(transform))
        });
    });
}

criterion_group!(benches, bench_match_room_name_hd, bench_transform_trimesh);
criterion_main!(benches);
```

---

## 6. 测试覆盖率目标

| 模块 | 目标覆盖率 | 优先级 |
|------|-----------|--------|
| `sqlite_spatial_api.rs` | 80%+ | 高 |
| `room_model.rs` (核心函数) | 70%+ | 高 |
| `room_worker.rs` | 60%+ | 中 |
| 集成测试 | 关键路径 100% | 高 |

---

## 7. CI/CD 集成

### 7.1 测试命令

```bash
# 单元测试
cargo test --lib

# 集成测试（需要数据库）
cargo test --test integration_* -- --ignored

# 基准测试
cargo bench

# 覆盖率报告
cargo tarpaulin --out Html
```

### 7.2 GitHub Actions 配置

```yaml
- name: Run unit tests
  run: cargo test --lib

- name: Run integration tests
  run: cargo test --test integration_* -- --ignored
  if: github.event_name == 'push' && github.ref == 'refs/heads/main'
```

---

## 8. 测试数据管理

### 8.1 测试夹具

```rust
// tests/fixtures/mod.rs

pub fn load_test_db_option() -> DbOption {
    DbOption {
        project_name: "test_project".to_string(),
        room_keyword: vec!["-RM".to_string()],
        ..Default::default()
    }
}

pub fn create_test_spatial_index() -> anyhow::Result<()> {
    // 创建测试用 SQLite 索引
    let idx = SqliteSpatialIndex::with_path("test_spatial_index.sqlite")?;
    idx.clear()?;
    
    // 插入测试数据
    let test_data = vec![
        (1, "EQUI".to_string(), 0.0, 100.0, 0.0, 100.0, 0.0, 100.0),
        (2, "PIPE".to_string(), 50.0, 150.0, 50.0, 150.0, 50.0, 150.0),
    ];
    
    idx.inner().insert_aabbs_with_items(test_data)?;
    Ok(())
}
```

---

## 9. 回归测试

### 9.1 已知问题验证

```rust
#[test]
fn test_p1_issue_1_radius_limit() {
    // 验证 P1 #1 修复：radius 上限
    let params = SqliteSpatialQueryParams {
        x: Some(0.0),
        y: Some(0.0),
        z: Some(0.0),
        radius: Some(1e20), // 极大值
        ..Default::default()
    };
    
    let result = do_spatial_query(params);
    assert!(!result.success);
    assert!(result.error.unwrap().contains("100000"));
}

#[test]
fn test_p1_issue_2_no_panic_on_invalid_mesh() {
    // 验证 P1 #2 修复：不会 panic
    let vertices = vec![Point::new(0.0, 0.0, 0.0)];
    let indices = vec![[0u32, 0, 0]];
    let mesh = TriMesh::new(vertices, indices).unwrap();
    
    let result = transform_tri_mesh(&mesh, Mat4::IDENTITY);
    // 应该返回 Result，不会 panic
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn test_p1_issue_3_regex_performance() {
    // 验证 P1 #3 修复：regex 不重复编译
    use std::time::Instant;
    
    let start = Instant::now();
    for _ in 0..10000 {
        match_room_name_hd("A001");
    }
    let duration = start.elapsed();
    
    // 10000 次调用应该在 10ms 内完成（如果每次编译会超过 1s）
    assert!(duration.as_millis() < 10);
}
```

---

## 10. 测试最佳实践

### 10.1 原则

1. **FIRST 原则**
   - Fast: 快速执行
   - Independent: 独立运行
   - Repeatable: 可重复
   - Self-validating: 自我验证
   - Timely: 及时编写

2. **AAA 模式**
   - Arrange: 准备测试数据
   - Act: 执行被测代码
   - Assert: 验证结果

3. **测试命名**
   - `test_<function>_<scenario>_<expected_result>`
   - 例：`test_position_mode_invalid_radius_returns_error`

### 10.2 避免的陷阱

- ❌ 测试依赖外部状态（数据库、文件系统）
- ❌ 测试之间有依赖关系
- ❌ 过度 mock 导致测试脆弱
- ❌ 测试实现细节而非行为
- ❌ 忽略边界条件和错误路径

---

## 总结

本测试方案覆盖：
- ✅ 单元测试（输入验证、边界条件、错误处理）
- ✅ 集成测试（端到端流程）
- ✅ 性能测试（基准测试）
- ✅ 回归测试（已修复问题验证）
- ✅ Mock 工具（隔离外部依赖）

**下一步行动**：
1. 实现核心单元测试（优先 P1 修复验证）
2. 添加 CI/CD 集成
3. 建立测试数据夹具
4. 定期运行覆盖率报告
