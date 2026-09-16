function envFlagEnabled(value: unknown): boolean {
  if (typeof value !== 'string') return false
  return ['1', 'true', 'yes', 'on'].includes(value.trim().toLowerCase())
}

/**
 * Admin feature gates.
 *
 * Offline deploy is hidden by default because it is still an operationally
 * sensitive flow. Enable it explicitly with VITE_ENABLE_OFFLINE_DEPLOY=true.
 */
export const OFFLINE_DEPLOY_ENABLED = envFlagEnabled(import.meta.env.VITE_ENABLE_OFFLINE_DEPLOY)
