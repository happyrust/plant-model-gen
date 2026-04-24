import { ref } from 'vue'
import { apiGet } from '@/api/client'

/**
 * Admin 前端在启动时从后端 `/api/admin/app-config` 拉一次，
 * 后续各模块（例如 `lib/viewer.ts`）同步读取这里的缓存值。
 *
 * 为什么要这一层：
 * - 历史上 `lib/viewer.ts` 只读 `import.meta.env.VITE_VIEWER_BASE`，
 *   build-time 写死，换部署环境要重出前端
 * - 现在改成 runtime：`AIOS_VIEWER_BASE_URL` 环境变量 → 后端 API →
 *   前端缓存 → `buildViewerUrl()` 同步消费
 * - 命中不到时，回退到 Vite env（兼容现有开发机），再回退到 null（不显示按钮）
 */

export interface AdminAppConfig {
  viewer_base_url?: string | null
}

const configRef = ref<AdminAppConfig>({})
let loadPromise: Promise<void> | null = null

function normalizeBase(value: unknown): string | null {
  if (typeof value !== 'string') return null
  const trimmed = value.trim().replace(/\/$/, '')
  return trimmed || null
}

/**
 * 幂等加载：首次成功后后续调用复用同一个 Promise；失败时清掉 Promise，
 * 让"登录完成再试一次"这种场景可以直接调用 `loadAppConfig()` 重试，
 * 不需要引入单独的 reload 入口。
 *
 * 失败不抛——让 UI 按"未配置"渲染，而不是卡死启动流程。
 */
export function loadAppConfig(): Promise<void> {
  if (loadPromise) return loadPromise
  loadPromise = (async () => {
    try {
      const resp = await apiGet<AdminAppConfig>('/api/admin/app-config')
      configRef.value = {
        viewer_base_url: normalizeBase(resp?.viewer_base_url),
      }
    } catch (err) {
      // Not fatal: fall back to Vite env / null in the resolver below.
      // Clear the cache so a later caller (e.g. after login) can retry.
      loadPromise = null
      // eslint-disable-next-line no-console
      console.warn('[admin] app-config load failed, falling back to build-time env:', err)
    }
  })()
  return loadPromise
}

/**
 * 同步解析 Viewer 基础 URL：
 *   runtime 后端配置 → Vite build-time env → null
 */
export function resolveViewerBaseUrl(): string | null {
  const runtime = normalizeBase(configRef.value.viewer_base_url)
  if (runtime) return runtime
  const viteEnv = import.meta.env.VITE_VIEWER_BASE as string | undefined
  return normalizeBase(viteEnv)
}

/** 测试 / 调试：直接读取当前缓存的配置对象。 */
export function getAppConfigSnapshot(): AdminAppConfig {
  return { ...configRef.value }
}
