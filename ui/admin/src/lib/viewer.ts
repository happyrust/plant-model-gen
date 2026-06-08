import type { ManagedProjectSite } from '@/types/site'
import { resolveViewerBaseUrlInfo } from './app-config'

/**
 * Viewer URL 生成。
 *
 * 来源优先级：
 *   1. 受管站点后端返回的 `site.viewer_url`（独立 plant3d-web 根地址）
 *   2. Admin 运行期 `AIOS_VIEWER_BASE_URL` / Vite `VITE_VIEWER_BASE`
 *   3. 受管 Viewer 本机端口（本地调试兜底）
 *   4. null（隐藏 Viewer 按钮）
 *
 * 调用方保持同步：前端在 `main.ts` 启动时 `await loadAppConfig()`，之后本函数
 * 可以安全地只读缓存；如果启动时拉取失败，会回退到 Vite env，不阻断 UI。
 *
 * 输出 query 只保留 plant3d-web 的业务参数；后端 API / files 由站点
 * Viewer 端口上的 Nginx 配置代理。
 */
function normalizeViewerBaseUrl(value: string | null | undefined): string | null {
  if (typeof value !== 'string') return null
  const trimmed = value.trim()
  if (!trimmed) return null

  try {
    const url = new URL(trimmed, window.location.origin)
    url.search = ''
    url.hash = ''
    return url.toString().replace(/\/$/, '')
  } catch {
    return trimmed.replace(/[?#].*$/, '').replace(/\/$/, '') || null
  }
}

function buildStandaloneViewerUrl(
  base: string,
  site: Pick<
    ManagedProjectSite,
    | 'project_name'
    | 'associated_project'
    | 'manual_db_nums'
  >,
): string {
  const url = new URL(`${base.replace(/\/$/, '')}/`, window.location.origin)
  url.searchParams.set('output_project', site.associated_project || site.project_name)

  const dbnum = site.manual_db_nums?.length === 1 ? site.manual_db_nums[0] : null
  if (dbnum) url.searchParams.set('show_dbnum', String(dbnum))

  return url.toString()
}

function withViewerPort(base: string, port: number | null | undefined): string | null {
  if (!port) return null
  try {
    const url = new URL(base, window.location.origin)
    url.port = String(port)
    url.search = ''
    url.hash = ''
    return url.toString().replace(/\/$/, '')
  } catch {
    return `${base.replace(/\/$/, '')}:${port}`
  }
}

export function buildViewerUrl(
  site: Pick<
    ManagedProjectSite,
    | 'viewer_port'
    | 'project_name'
    | 'associated_project'
    | 'viewer_url'
    | 'manual_db_nums'
  >,
): string | null {
  const runtimeBase = resolveViewerBaseUrlInfo()
  const runtimeUrl =
    runtimeBase?.source === 'local_ip' && site.viewer_port
      ? withViewerPort(runtimeBase.url, site.viewer_port)
      : runtimeBase?.url
  const browserHost = typeof window !== 'undefined' ? window.location.hostname : ''
  const browserHostIsIp =
    browserHost
    && browserHost !== 'localhost'
    && browserHost !== '127.0.0.1'
    && browserHost !== '::1'

  const base =
    normalizeViewerBaseUrl(site.viewer_url)
    || normalizeViewerBaseUrl(runtimeUrl)
    || (site.viewer_port && browserHostIsIp
      ? normalizeViewerBaseUrl(`${window.location.protocol}//${window.location.hostname}:${site.viewer_port}`)
      : null)

  if (!base) return null
  return buildStandaloneViewerUrl(base, site)
}
