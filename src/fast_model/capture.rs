use std::path::{Path, PathBuf};
use std::sync::Mutex;

use aios_core::shape::pdms_shape::PlantMesh;
use aios_core::RefnoEnum;
use anyhow::{Context, Result, anyhow};
use glam::{Mat4, Vec2, Vec3, Vec4};
use image::{Rgba, RgbaImage};
use imageproc::drawing::{draw_text_mut, text_size};
use once_cell::sync::Lazy;
use rusttype::{Font, Scale};
use tokio::fs;

use crate::fast_model::export_model::export_obj::{
    PreparedObjExport, export_mesh_to_obj_with_unit_conversion, prepare_obj_export,
};
use crate::fast_model::model_exporter::CommonExportConfig;
use crate::fast_model::unit_converter::UnitConverter;
use crate::options::DbOptionExt;

#[derive(Debug, Clone)]
pub struct CaptureConfig {
    pub output_dir: PathBuf,
    pub width: u32,
    pub height: u32,
    pub include_descendants: bool,
    /// 额外视角数量（>=1）；1 表示仅生成默认视角 `{basename}.png`。
    /// 用于调试“看起来像缺失/断开但其实是视角遮挡”的情况。
    pub views: u8,
    /// 期望截图（baseline）目录；文件名需与生成的 basename 对齐（或使用 refno_格式回退）。
    pub baseline_dir: Option<PathBuf>,
    /// diff 输出目录；若为 None 且 baseline_dir 存在，则默认 output_dir/diff。
    pub diff_dir: Option<PathBuf>,
}

impl CaptureConfig {
    pub fn new(
        output_dir: PathBuf,
        width: u32,
        height: u32,
        include_descendants: bool,
        views: u8,
        baseline_dir: Option<PathBuf>,
        diff_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            output_dir,
            width,
            height,
            include_descendants,
            views: views.max(1),
            baseline_dir,
            diff_dir,
        }
    }
}

static CAPTURE_CONFIG: Lazy<Mutex<Option<CaptureConfig>>> = Lazy::new(|| Mutex::new(None));

pub fn set_capture_config(config: Option<CaptureConfig>) {
    let mut guard = CAPTURE_CONFIG.lock().unwrap();
    *guard = config;
}

pub fn capture_enabled() -> bool {
    CAPTURE_CONFIG.lock().unwrap().is_some()
}

fn current_config() -> Option<CaptureConfig> {
    CAPTURE_CONFIG.lock().unwrap().clone()
}

pub async fn capture_refnos_if_enabled(refnos: &[RefnoEnum], db_option: &DbOptionExt) -> Result<()> {
    if !capture_enabled() || refnos.is_empty() {
        return Ok(());
    }
    capture_refnos(refnos, db_option).await
}

pub async fn capture_refnos(refnos: &[RefnoEnum], db_option: &DbOptionExt) -> Result<()> {
    let config = match current_config() {
        Some(cfg) => cfg,
        None => return Ok(()),
    };

    // Windows 下 main 线程默认栈较小，prepare_obj_export/collect_export_data
    // 在某些情况下会触发 STATUS_STACK_OVERFLOW。
    // 这里将截图流程放到一个更大栈的专用线程中执行，保证截图/对比链路可用。
    let mesh_dir = db_option.get_meshes_path().to_path_buf();
    let refnos_vec: Vec<RefnoEnum> = refnos.to_vec();

    // 截图链路复用 OBJ 导出数据源策略：默认 cache-only；如需 SurrealDB，需显式开关。
    let allow_surrealdb = db_option.use_surrealdb;
    let cache_dir: Option<PathBuf> = if allow_surrealdb {
        None
    } else if db_option.use_cache {
        Some(db_option.get_foyer_cache_dir())
    } else {
        None
    };
    if !allow_surrealdb && cache_dir.is_none() {
        return Err(anyhow!("截图/OBJ 导出默认关闭 SurrealDB，但当前配置未启用 use_cache"));
    }

    let handle = tokio::runtime::Handle::current();

    tokio::task::spawn_blocking(move || -> Result<()> {
        std::thread::Builder::new()
            .name("aios-capture".to_string())
            // 64MB：足够覆盖导出/渲染阶段的较大栈帧
            .stack_size(64 * 1024 * 1024)
            .spawn(move || {
                handle.block_on(async move {
                    for refno in refnos_vec {
                        if let Err(err) =
                            capture_single(refno, &mesh_dir, &config, allow_surrealdb, cache_dir.clone()).await
                        {
                            eprintln!("[capture] 捕获参考号 {} 截图失败: {}", refno, err);
                        }
                    }
                    Ok::<(), anyhow::Error>(())
                })
            })
            .context("创建截图专用线程失败")?
            .join()
            .map_err(|_| anyhow!("截图专用线程 panic"))??;

        Ok(())
    })
    .await??;

    Ok(())
}

async fn capture_single(
    refno: RefnoEnum,
    mesh_dir: &Path,
    config: &CaptureConfig,
    allow_surrealdb: bool,
    cache_dir: Option<PathBuf>,
) -> Result<()> {
    let refno_str = refno.to_string().replace('/', "_");
    let basename = resolve_capture_basename(refno);

    fs::create_dir_all(&config.output_dir)
        .await
        .with_context(|| format!("创建截图目录失败: {}", config.output_dir.display()))?;
    let obj_cache_dir = config.output_dir.join("obj-cache");
    fs::create_dir_all(&obj_cache_dir)
        .await
        .with_context(|| format!("创建 OBJ 缓存目录失败: {}", obj_cache_dir.display()))?;

    let obj_path = obj_cache_dir.join(format!("{basename}.obj"));
    let png_path = config.output_dir.join(format!("{basename}.png"));

    let mut common_config = CommonExportConfig {
        include_descendants: config.include_descendants,
        filter_nouns: None,
        verbose: true, // 启用详细输出以便调试
        unit_converter: UnitConverter::default(),
        use_basic_materials: false,
        include_negative: false,
        allow_surrealdb,
        cache_dir,
    };

    let mut prepared = prepare_obj_export(&[refno], mesh_dir, &common_config).await?;
    let mut mesh = prepared.mesh;
    let mut stats = prepared.stats;

    println!(
        "[capture] 参考号 {} 初步查询结果: vertices={}, geometry_count={}, mesh_files_found={}, mesh_files_missing={}",
        refno,
        mesh.vertices.len(),
        stats.geometry_count,
        stats.mesh_files_found,
        stats.mesh_files_missing
    );

    if (mesh.vertices.is_empty() || stats.geometry_count == 0) && !common_config.include_descendants
    {
        let mut alt_config = common_config.clone();
        alt_config.include_descendants = true;
        prepared = prepare_obj_export(&[refno], mesh_dir, &alt_config).await?;
        if !prepared.mesh.vertices.is_empty() && prepared.stats.geometry_count > 0 {
            println!("[capture] 参考号 {} 自动启用子孙节点重新收集几何体", refno);
            mesh = prepared.mesh;
            stats = prepared.stats;
            common_config = alt_config;
        }
    }

    if mesh.vertices.is_empty() || stats.geometry_count == 0 {
        println!(
            "[capture] 参考号 {} 无可导出的几何体，跳过截图 (vertices={}, geometry_count={})",
            refno,
            mesh.vertices.len(),
            stats.geometry_count
        );
        return Ok(());
    }

    let obj_path_str = obj_path
        .to_str()
        .ok_or_else(|| anyhow!("OBJ 路径无法转换为字符串"))?;

    export_mesh_to_obj_with_unit_conversion(&mesh, obj_path_str, &common_config.unit_converter)?;

    let render_mesh = mesh_with_unit_conversion(&mesh, &common_config.unit_converter);
    let display_label = get_display_label(refno);
    render_mesh_to_png(
        &render_mesh,
        config.width,
        config.height,
        &png_path,
        &display_label,
    )?;

    println!("📸 已生成截图: {}", png_path.display());

    // 额外视角：用于人工确认“模型是否真缺失”。
    // 文件命名采用 `{basename}_viewXX.png`，不影响现有 `{basename}.png` 的对齐与 diff 流程。
    if config.views > 1 {
        let views = config.views as usize;
        for i in 1..views {
            let view_png = config
                .output_dir
                .join(format!("{basename}_view{:02}.png", i));
            render_mesh_to_png_with_camera(
                &render_mesh,
                config.width,
                config.height,
                &view_png,
                &display_label,
                45.0,
                45.0 + (360.0 / views as f32) * (i as f32),
            )?;
        }
    }

    // 额外生成一个“稳定文件名”（refno）版本，便于基准图/脚本对齐；不影响现有 basename 逻辑。
    if basename != refno_str {
        let stable_png = config.output_dir.join(format!("{refno_str}.png"));
        let _ = fs::copy(&png_path, &stable_png).await;
    }

    if let Some(baseline_dir) = &config.baseline_dir {
        if let Err(e) = compare_with_baseline(&basename, &refno_str, &png_path, baseline_dir, config).await {
            eprintln!(
                "[capture][diff] 参考号 {} 对比 baseline 失败: {}",
                refno, e
            );
        }
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct DiffStats {
    mae: f32,
    max_diff: u8,
    changed_pixels: u32,
    total_pixels: u32,
}

async fn compare_with_baseline(
    basename: &str,
    refno_str: &str,
    actual_png: &Path,
    baseline_dir: &Path,
    config: &CaptureConfig,
) -> Result<()> {
    let expected_by_basename = baseline_dir.join(format!("{basename}.png"));
    let expected_by_refno = baseline_dir.join(format!("{refno_str}.png"));
    let expected = if expected_by_basename.exists() {
        expected_by_basename
    } else if expected_by_refno.exists() {
        expected_by_refno
    } else {
        return Err(anyhow!(
            "未找到 baseline：{} 或 {}",
            expected_by_basename.display(),
            expected_by_refno.display()
        ));
    };

    let diff_dir = config
        .diff_dir
        .clone()
        .unwrap_or_else(|| config.output_dir.join("diff"));
    fs::create_dir_all(&diff_dir)
        .await
        .with_context(|| format!("创建 diff 目录失败: {}", diff_dir.display()))?;
    let diff_path = diff_dir.join(format!("{basename}.diff.png"));

    let stats = diff_png(&expected, actual_png, &diff_path)?;
    println!(
        "[capture][diff] {} mae={:.4} max={} changed={}/{} diff={}",
        basename,
        stats.mae,
        stats.max_diff,
        stats.changed_pixels,
        stats.total_pixels,
        diff_path.display()
    );
    Ok(())
}

fn diff_png(expected_path: &Path, actual_path: &Path, out_path: &Path) -> Result<DiffStats> {
    let expected = image::open(expected_path)
        .with_context(|| format!("读取 baseline 失败: {}", expected_path.display()))?
        .to_rgba8();
    let actual = image::open(actual_path)
        .with_context(|| format!("读取当前截图失败: {}", actual_path.display()))?
        .to_rgba8();

    if expected.dimensions() != actual.dimensions() {
        return Err(anyhow!(
            "图片尺寸不一致：baseline={} current={}",
            format!("{:?}", expected.dimensions()),
            format!("{:?}", actual.dimensions())
        ));
    }

    let (w, h) = expected.dimensions();
    let mut diff = RgbaImage::new(w, h);

    let mut sum_abs: u64 = 0;
    let mut max_diff: u8 = 0;
    let mut changed: u32 = 0;
    let total = w.saturating_mul(h);

    for y in 0..h {
        for x in 0..w {
            let e = expected.get_pixel(x, y).0;
            let a = actual.get_pixel(x, y).0;
            let dr = e[0].abs_diff(a[0]);
            let dg = e[1].abs_diff(a[1]);
            let db = e[2].abs_diff(a[2]);
            let m = dr.max(dg).max(db);
            if m > 0 {
                changed += 1;
            }
            max_diff = max_diff.max(m);
            sum_abs += dr as u64 + dg as u64 + db as u64;
            diff.put_pixel(x, y, Rgba([dr, dg, db, 255]));
        }
    }

    let denom = (total as f64).max(1.0) * 3.0;
    let mae = (sum_abs as f64 / denom) as f32;

    diff.save(out_path)
        .with_context(|| format!("保存 diff 失败: {}", out_path.display()))?;

    Ok(DiffStats {
        mae,
        max_diff,
        changed_pixels: changed,
        total_pixels: total,
    })
}

fn resolve_capture_basename(refno: RefnoEnum) -> String {
    // cache-only：导出/截图阶段不查库补齐 NAME/TYPE，避免引入 SurrealDB 依赖。
    let raw = refno.to_string().replace('/', "_");
    sanitize_label(&raw)
}

fn sanitize_label(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut sanitized = String::with_capacity(trimmed.len());
    for c in trimmed.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' | '.' | ' ' => sanitized.push(c),
            '\u{4e00}'..='\u{9fff}' => sanitized.push(c),
            _ => sanitized.push('_'),
        }
    }

    if sanitized.len() > 128 {
        sanitized.chars().take(128).collect()
    } else {
        sanitized
    }
}

fn mesh_with_unit_conversion(mesh: &PlantMesh, unit_converter: &UnitConverter) -> PlantMesh {
    if !unit_converter.needs_conversion() {
        return mesh.clone();
    }

    let mut converted = mesh.clone();
    for vertex in &mut converted.vertices {
        *vertex = unit_converter.convert_vec3(vertex);
    }
    converted
}

fn get_display_label(refno: RefnoEnum) -> String {
    let refno_str = refno.to_string().trim_start_matches('/').to_string();
    refno_str
}

fn render_mesh_to_png(
    mesh: &PlantMesh,
    width: u32,
    height: u32,
    output_path: &Path,
    label: &str,
) -> Result<()> {
    // 默认截图风格：正视 + 正交投影 + 灰底 + 边缘描线（贴近人工验收习惯）
    render_mesh_to_png_with_camera(mesh, width, height, output_path, label, 0.0, 0.0)
}

fn render_mesh_to_png_with_camera(
    mesh: &PlantMesh,
    width: u32,
    height: u32,
    output_path: &Path,
    label: &str,
    tilt_deg: f32,
    side_deg: f32,
) -> Result<()> {
    // 使用超采样抗锯齿：先渲染到高分辨率，然后下采样
    const SSAA_SCALE: u32 = 2; // 2x2 超采样
    let ssaa_width = width * SSAA_SCALE;
    let ssaa_height = height * SSAA_SCALE;
    let mut ssaa_image = RgbaImage::from_pixel(ssaa_width, ssaa_height, Rgba([0, 0, 0, 0]));

    if mesh.vertices.is_empty() || mesh.indices.is_empty() {
        // 与正常路径保持一致：灰底（而非透明），便于人工比对
        let mut image = RgbaImage::from_pixel(width, height, Rgba([235, 235, 235, 255]));
        image
            .save(output_path)
            .with_context(|| format!("保存截图失败: {}", output_path.display()))?;
        return Ok(());
    }

    let (bounds_min, bounds_max) =
        compute_bounds(&mesh.vertices).ok_or_else(|| anyhow!("mesh 缺失顶点数据，无法截图"))?;
    let center = (bounds_min + bounds_max) * 0.5;
    let extent = bounds_max - bounds_min;
    let max_extent = extent
        .x
        .abs()
        .max(extent.y.abs())
        .max(extent.z.abs())
        .max(1.0);
    let radius = extent.length().max(max_extent).max(1.0);
    // 增加相机距离系数，确保模型完整显示不被截断
    // 考虑到透视投影（FOV 45度）和45度视角，需要更大的距离系数
    // 使用 max_extent 而不是 radius，确保适应非立方体的边界框
    let camera_distance = max_extent * 1.8;
    // 前视图 + 俯视 45°：相机在前方（Y 轴负方向）并向上（Z 轴正方向）45 度俯视模型
    // 方向向量：(0, -cos(45°), sin(45°)) = (0, -1/√2, 1/√2)
    let cos45 = tilt_deg.to_radians().cos();
    let sin45 = tilt_deg.to_radians().sin();
    let mut camera_dir = Vec3::new(0.0, -cos45, sin45).normalize();
    if camera_dir.length_squared() < f32::EPSILON {
        camera_dir = Vec3::new(0.0, -1.0, 1.0).normalize();
    }
    // 添加 45 度侧向旋转：绕 Z 轴旋转相机方向
    let side_rotation_angle = side_deg.to_radians();
    let cos_side = side_rotation_angle.cos();
    let sin_side = side_rotation_angle.sin();
    // 绕 Z 轴旋转 camera_dir（保持 Z 分量不变，旋转 X-Y 平面分量）
    let camera_dir_xy = Vec3::new(camera_dir.x, camera_dir.y, 0.0);
    let camera_dir_z = camera_dir.z;
    let rotated_xy = Vec3::new(
        camera_dir_xy.x * cos_side - camera_dir_xy.y * sin_side,
        camera_dir_xy.x * sin_side + camera_dir_xy.y * cos_side,
        0.0,
    );
    camera_dir = Vec3::new(rotated_xy.x, rotated_xy.y, camera_dir_z).normalize();
    // up 向量指向 Z 轴正方向
    let up = Vec3::Z;
    let eye = center + camera_dir * camera_distance;

    let view = Mat4::look_at_rh(eye, center, up);
    let aspect = (ssaa_width as f32 / ssaa_height as f32).max(0.01);
    // 采用正交投影：用 view-space AABB 自适配裁剪面与可视范围（更像工程验收截图）
    let mut vmin = Vec3::splat(f32::INFINITY);
    let mut vmax = Vec3::splat(-f32::INFINITY);
    for v in &mesh.vertices {
        let p = (view * Vec4::new(v.x, v.y, v.z, 1.0)).truncate();
        vmin = vmin.min(p);
        vmax = vmax.max(p);
    }
    let mut view_w = (vmax.x - vmin.x).abs().max(1e-3);
    let mut view_h = (vmax.y - vmin.y).abs().max(1e-3);
    // 对齐目标画幅的宽高比，避免被拉伸或裁剪
    let view_aspect = view_w / view_h;
    if view_aspect > aspect {
        view_h = (view_w / aspect).max(1e-3);
    } else {
        view_w = (view_h * aspect).max(1e-3);
    }
    // 留白，避免紧贴边缘
    let pad = 1.08f32;
    view_w *= pad;
    view_h *= pad;
    let cx = (vmin.x + vmax.x) * 0.5;
    let cy = (vmin.y + vmax.y) * 0.5;
    let left = cx - view_w * 0.5;
    let right = cx + view_w * 0.5;
    let bottom = cy - view_h * 0.5;
    let top = cy + view_h * 0.5;
    // view-space 中：相机前方通常为负 Z（look_at_rh）。near/far 需要正数距离。
    let near = (-vmax.z).max((radius * 0.02).max(0.01));
    let far = (-vmin.z).max((radius * 12.0).max(near + 1.0));
    let rml = right - left;
    let tmb = top - bottom;
    let fmn = far - near;
    let proj = Mat4::from_cols(
        Vec4::new(2.0 / rml, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 2.0 / tmb, 0.0, 0.0),
        Vec4::new(0.0, 0.0, -2.0 / fmn, 0.0),
        Vec4::new(-(right + left) / rml, -(top + bottom) / tmb, -(far + near) / fmn, 1.0),
    );
    let view_proj = proj * view;

    let mut ndc_positions = vec![Vec3::ZERO; mesh.vertices.len()];
    let mut screen_positions = vec![Vec3::ZERO; mesh.vertices.len()];
    let mut valid = vec![false; mesh.vertices.len()];

    for (idx, vertex) in mesh.vertices.iter().enumerate() {
        let pos4 = view_proj * Vec4::new(vertex.x, vertex.y, vertex.z, 1.0);
        if pos4.w.abs() < 1e-6 {
            continue;
        }
        let ndc = pos4.truncate() / pos4.w;
        ndc_positions[idx] = ndc;
        let screen_x = (ndc.x * 0.5 + 0.5) * (ssaa_width as f32 - 1.0);
        let screen_y = (1.0 - (ndc.y * 0.5 + 0.5)) * (ssaa_height as f32 - 1.0);
        let depth = (ndc.z * 0.5) + 0.5;
        screen_positions[idx] = Vec3::new(screen_x, screen_y, depth);
        valid[idx] = true;
    }

    let mut depth_buffer = vec![f32::INFINITY; (ssaa_width as usize) * (ssaa_height as usize)];
    // 验收截图更偏“工程视图”：使用接近纯色的填充（避免因法线/光照导致颜色忽明忽暗）。
    let base_color = Vec3::new(0.18, 0.52, 0.86);
    let base_color_u8 = [
        (base_color.x.clamp(0.0, 1.0) * 255.0) as u8,
        (base_color.y.clamp(0.0, 1.0) * 255.0) as u8,
        (base_color.z.clamp(0.0, 1.0) * 255.0) as u8,
    ];

    for tri in mesh.indices.chunks(3) {
        if tri.len() < 3 {
            continue;
        }
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= screen_positions.len()
            || i1 >= screen_positions.len()
            || i2 >= screen_positions.len()
        {
            continue;
        }
        if !valid[i0] || !valid[i1] || !valid[i2] {
            continue;
        }

        let p0 = Vec2::new(screen_positions[i0].x, screen_positions[i0].y);
        let p1 = Vec2::new(screen_positions[i1].x, screen_positions[i1].y);
        let p2 = Vec2::new(screen_positions[i2].x, screen_positions[i2].y);

        let area = edge(p0, p1, p2);
        if area.abs() < 1e-5 {
            continue;
        }

        let inv_area = 1.0 / area;
        let x_min = p0.x.min(p1.x).min(p2.x).floor().max(0.0) as i32;
        let x_max = p0.x.max(p1.x).max(p2.x).ceil().min((ssaa_width - 1) as f32) as i32;
        let y_min = p0.y.min(p1.y).min(p2.y).floor().max(0.0) as i32;
        let y_max =
            p0.y.max(p1.y)
                .max(p2.y)
                .ceil()
                .min((ssaa_height - 1) as f32) as i32;

        for y in y_min..=y_max {
            for x in x_min..=x_max {
                // 使用像素中心采样
                let sample = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let mut w0 = edge(p1, p2, sample);
                let mut w1 = edge(p2, p0, sample);
                let mut w2 = edge(p0, p1, sample);

                if (area > 0.0 && (w0 < 0.0 || w1 < 0.0 || w2 < 0.0))
                    || (area < 0.0 && (w0 > 0.0 || w1 > 0.0 || w2 > 0.0))
                {
                    continue;
                }

                w0 *= inv_area;
                w1 *= inv_area;
                w2 *= inv_area;

                let depth = w0 * screen_positions[i0].z
                    + w1 * screen_positions[i1].z
                    + w2 * screen_positions[i2].z;
                let buffer_index = (y as usize) * (ssaa_width as usize) + (x as usize);
                // 使用更严格的深度比较，减少精度误差
                if depth >= depth_buffer[buffer_index] - 1e-6 {
                    continue;
                }
                depth_buffer[buffer_index] = depth;

                let color = Rgba([
                    base_color_u8[0],
                    base_color_u8[1],
                    base_color_u8[2],
                    255, // 完全不透明
                ]);
                ssaa_image.put_pixel(x as u32, y as u32, color);
            }
        }
    }

    // 将超采样图像下采样到目标分辨率，实现抗锯齿
    let mut image = RgbaImage::from_pixel(width, height, Rgba([0, 0, 0, 0]));
    for y in 0..height {
        for x in 0..width {
            let mut r_sum = 0u32;
            let mut g_sum = 0u32;
            let mut b_sum = 0u32;
            let mut a_sum = 0u32;
            let mut count = 0u32;

            // 对每个输出像素，采样 SSAA_SCALE x SSAA_SCALE 个超采样像素
            for sy in 0..SSAA_SCALE {
                for sx in 0..SSAA_SCALE {
                    let sx_coord = x * SSAA_SCALE + sx;
                    let sy_coord = y * SSAA_SCALE + sy;
                    if sx_coord < ssaa_width && sy_coord < ssaa_height {
                        let pixel = ssaa_image.get_pixel(sx_coord, sy_coord);
                        r_sum += pixel[0] as u32;
                        g_sum += pixel[1] as u32;
                        b_sum += pixel[2] as u32;
                        a_sum += pixel[3] as u32;
                        count += 1;
                    }
                }
            }

            if count > 0 {
                let scale = 1.0 / (count as f32);
                let color = Rgba([
                    ((r_sum as f32) * scale) as u8,
                    ((g_sum as f32) * scale) as u8,
                    ((b_sum as f32) * scale) as u8,
                    ((a_sum as f32) * scale) as u8,
                ]);
                image.put_pixel(x, y, color);
            }
        }
    }

    // 组合灰底，并按 alpha 做“边缘描线”（仅外轮廓），以贴近人工验收截图风格。
    let bg = [235u8, 235u8, 235u8];
    let edge = [60u8, 60u8, 60u8];
    let mut out = RgbaImage::from_pixel(width, height, Rgba([bg[0], bg[1], bg[2], 255]));

    // 先把模型按 alpha 贴到灰底上
    for y in 0..height {
        for x in 0..width {
            let p = *image.get_pixel(x, y);
            let a = (p[3] as f32) / 255.0;
            if a <= 0.0 {
                continue;
            }
            let inv = 1.0 - a;
            let r = (bg[0] as f32 * inv + p[0] as f32 * a).round().clamp(0.0, 255.0) as u8;
            let g = (bg[1] as f32 * inv + p[1] as f32 * a).round().clamp(0.0, 255.0) as u8;
            let b = (bg[2] as f32 * inv + p[2] as f32 * a).round().clamp(0.0, 255.0) as u8;
            out.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }

    // 再做 1px 外轮廓描边：alpha 边界 或 邻域从实到空的跳变
    for y in 1..height.saturating_sub(1) {
        for x in 1..width.saturating_sub(1) {
            let a = image.get_pixel(x, y)[3];
            if a == 0 {
                continue;
            }
            let is_soft_edge = a < 250;
            let neigh_empty = image.get_pixel(x - 1, y)[3] < 10
                || image.get_pixel(x + 1, y)[3] < 10
                || image.get_pixel(x, y - 1)[3] < 10
                || image.get_pixel(x, y + 1)[3] < 10;
            if !is_soft_edge && !neigh_empty {
                continue;
            }

            // 对软边缘像素按 coverage 做轻量混合，减少“黑边断裂”
            let t = (a as f32 / 255.0).clamp(0.0, 1.0);
            let blend = if is_soft_edge { 0.85 * t } else { 0.85 };
            let p = *out.get_pixel(x, y);
            let inv = 1.0 - blend;
            let r = (p[0] as f32 * inv + edge[0] as f32 * blend).round().clamp(0.0, 255.0) as u8;
            let g = (p[1] as f32 * inv + edge[1] as f32 * blend).round().clamp(0.0, 255.0) as u8;
            let b = (p[2] as f32 * inv + edge[2] as f32 * blend).round().clamp(0.0, 255.0) as u8;
            out.put_pixel(x, y, Rgba([r, g, b, 255]));
        }
    }

    // 保留 label 参数以兼容调用端，但默认不绘制（Windows 也无系统字体可依赖）。
    let _ = label;

    out
        .save(output_path)
        .with_context(|| format!("保存截图失败: {}", output_path.display()))?;
    Ok(())
}

fn draw_label_on_image(image: &mut RgbaImage, label: &str, width: u32, height: u32) -> Result<()> {
    if label.is_empty() {
        return Ok(());
    }

    // 尝试加载系统字体（优先使用常规字体，避免粗体）
    let font_paths = vec![
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf", // 常规字体
        "/System/Library/Fonts/PingFang.ttc",                   // 常规字体
        "/System/Library/Fonts/Helvetica.ttc",                  // 常规字体
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",      // 常规字体
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf", // 常规字体
        "/System/Library/Fonts/STHeiti Light.ttc",              // 细体字体
    ];

    let font_data = match font_paths.iter().find_map(|path| std::fs::read(path).ok()) {
        Some(data) => data,
        None => {
            // 如果无法加载字体，跳过文字绘制（不返回错误，避免影响截图功能）
            eprintln!("[capture] 警告: 无法加载系统字体，跳过文字标注");
            return Ok(());
        }
    };

    let font = match Font::try_from_bytes(&font_data) {
        Some(f) => f,
        None => {
            eprintln!("[capture] 警告: 无法解析字体数据，跳过文字标注");
            return Ok(());
        }
    };

    // 设置字体大小（根据图像高度自适应，进一步增大字体）
    let font_size = (height as f32 * 0.06).max(32.0).min(64.0);
    let scale = Scale::uniform(font_size);

    // 计算文字尺寸和位置（底部中间，稍微上移）
    let (text_width, text_height) = text_size(scale, &font, label);
    let x = (width.saturating_sub(text_width as u32)) / 2;
    let y = height.saturating_sub(text_height as u32 + 20); // 距离底部 20 像素（之前是 10，现在上移）

    // 绘制文字（红色，带增强的黑色描边以提高清晰度）
    let text_color = Rgba([255, 0, 0, 255]); // 红色
    let stroke_color = Rgba([0, 0, 0, 220]); // 半透明黑色描边，增强清晰度

    // 先绘制描边（增强描边效果：两层描边，外层更粗）
    // 外层描边（偏移2像素）
    for dy in -2i32..=2i32 {
        for dx in -2i32..=2i32 {
            if dx == 0 && dy == 0 {
                continue;
            }
            // 只绘制外围像素，不绘制内层
            if dx.abs() + dy.abs() == 2 {
                draw_text_mut(
                    image,
                    stroke_color,
                    (x as i32 + dx).max(0),
                    (y as i32 + dy).max(0),
                    scale,
                    &font,
                    label,
                );
            }
        }
    }
    // 内层描边（偏移1像素）
    for dy in -1i32..=1i32 {
        for dx in -1i32..=1i32 {
            if dx == 0 && dy == 0 {
                continue;
            }
            draw_text_mut(
                image,
                Rgba([0, 0, 0, 255]), // 完全不透明的黑色
                (x as i32 + dx).max(0),
                (y as i32 + dy).max(0),
                scale,
                &font,
                label,
            );
        }
    }

    // 绘制文字本体
    draw_text_mut(image, text_color, x as i32, y as i32, scale, &font, label);

    Ok(())
}

fn ensure_vertex_normals(mesh: &PlantMesh) -> Vec<Vec3> {
    if mesh.normals.len() == mesh.vertices.len() && !mesh.normals.is_empty() {
        return mesh.normals.clone();
    }

    let mut normals = vec![Vec3::ZERO; mesh.vertices.len()];
    for tri in mesh.indices.chunks(3) {
        if tri.len() < 3 {
            continue;
        }
        let i0 = tri[0] as usize;
        let i1 = tri[1] as usize;
        let i2 = tri[2] as usize;
        if i0 >= mesh.vertices.len() || i1 >= mesh.vertices.len() || i2 >= mesh.vertices.len() {
            continue;
        }

        let v0 = mesh.vertices[i0];
        let v1 = mesh.vertices[i1];
        let v2 = mesh.vertices[i2];
        let normal = (v1 - v0).cross(v2 - v0);
        if normal.length_squared() > f32::EPSILON {
            let n = normal.normalize();
            normals[i0] += n;
            normals[i1] += n;
            normals[i2] += n;
        }
    }

    for normal in &mut normals {
        if normal.length_squared() > f32::EPSILON {
            *normal = normal.normalize();
        } else {
            *normal = Vec3::Y;
        }
    }

    normals
}

fn compute_bounds(vertices: &[Vec3]) -> Option<(Vec3, Vec3)> {
    if vertices.is_empty() {
        return None;
    }

    let mut min = vertices[0];
    let mut max = vertices[0];
    for vertex in vertices.iter().skip(1) {
        min = min.min(*vertex);
        max = max.max(*vertex);
    }
    Some((min, max))
}

#[inline]
fn edge(a: Vec2, b: Vec2, c: Vec2) -> f32 {
    (c.x - a.x) * (b.y - a.y) - (c.y - a.y) * (b.x - a.x)
}
