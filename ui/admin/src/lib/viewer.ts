import type { ManagedProjectSite } from '@/types/site'

function resolveViewerBase(): string {
  const configured = import.meta.env.VITE_VIEWER_BASE as string | undefined
  if (configured && configured.trim()) {
    return configured.replace(/\/$/, '')
  }
  return ''
}

export function buildViewerUrl(
  site: Pick<
    ManagedProjectSite,
    'web_port' | 'project_name' | 'associated_project' | 'entry_url' | 'local_entry_url' | 'public_entry_url'
  >,
): string | null {
  if (!site.web_port) return null
  const base = resolveViewerBase()
  if (!base) return null

  const project = encodeURIComponent(site.associated_project || site.project_name)
  const backend =
    site.public_entry_url
    || site.local_entry_url
    || site.entry_url
    || `http://127.0.0.1:${site.web_port}`

  return `${base}/?backendPort=${site.web_port}&backend=${encodeURIComponent(backend)}&output_project=${project}`
}
