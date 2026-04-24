import type { ManagedProjectSite } from '@/types/site'
import { resolveViewerBaseUrl } from './app-config'

/**
 * Viewer URL 生成。
 *
 * 来源优先级由 `resolveViewerBaseUrl()` 决定：
 *   runtime `/api/admin/app-config` (env `AIOS_VIEWER_BASE_URL`)
 *     → Vite build-time env `VITE_VIEWER_BASE`
 *     → null（隐藏 Viewer 按钮）
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
    'web_port' | 'project_name' | 'associated_project' | 'entry_url' | 'local_entry_url' | 'public_entry_url'
  >,
): string | null {
  if (!site.web_port) return null
  const base = resolveViewerBaseUrl()
  if (!base) return null

  const project = encodeURIComponent(site.associated_project || site.project_name)
  const backend =
    site.public_entry_url
    || site.local_entry_url
    || site.entry_url
    || `http://127.0.0.1:${site.web_port}`

  return `${base}/?backendPort=${site.web_port}&backend=${encodeURIComponent(backend)}&output_project=${project}`
}
