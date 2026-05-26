import type { ManagedProjectSite } from '@/types/site'
import { resolveViewerBaseUrl } from './app-config'

/**
 * Viewer URL 生成。
 *
 * 来源优先级：
 *   1. 受管站点后端返回的 `site.viewer_url`（站点启动 plant3d-web 后写入）
 *   2. runtime `/api/admin/app-config` (env `AIOS_VIEWER_BASE_URL`)
 *   3. Vite build-time env `VITE_VIEWER_BASE`
 *   4. null（隐藏 Viewer 按钮）
 *
 * 调用方保持同步：前端在 `main.ts` 启动时 `await loadAppConfig()`，之后本函数
 * 可以安全地只读缓存；如果启动时拉取失败，会回退到 Vite env，不阻断 UI。
 *
 * 输出 query 协议保持不变（backendPort / backend / output_project），
 * 以保证现有 plant3d-web viewer 页面向后兼容。
 */
export function buildViewerUrl(
  site: Pick<
    ManagedProjectSite,
    | 'web_port'
    | 'project_name'
    | 'associated_project'
    | 'entry_url'
    | 'local_entry_url'
    | 'public_entry_url'
    | 'viewer_url'
    | 'manual_db_nums'
    | 'export_parquet'
  >,
): string | null {
  if (site.viewer_url) return site.viewer_url
  if (!site.web_port) return null
  const base = resolveViewerBaseUrl()
  if (!base) return null

  const backend =
    site.public_entry_url
    || site.local_entry_url
    || site.entry_url
    || `http://127.0.0.1:${site.web_port}`

  const params = new URLSearchParams({
    backendPort: String(site.web_port),
    backend,
    output_project: site.associated_project || site.project_name,
  })
  const dbnum = site.manual_db_nums?.length === 1 ? site.manual_db_nums[0] : null
  if (dbnum) params.set('show_dbnum', String(dbnum))
  if (site.export_parquet) params.set('data_source', 'parquet')

  return `${base}/?${params.toString()}`
}
