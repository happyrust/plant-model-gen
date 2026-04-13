import type { ManagedProjectSite } from '@/types/site'
import type { DatabaseConfig } from '@/types/task'
import type {
  RegistrySite,
  RegistrySiteForm,
  RegistrySiteMutationPayload,
} from '@/types/registry'

function resolveBackendOrigin(): string {
  if (typeof window === 'undefined' || !window.location.origin) {
    return 'http://127.0.0.1:3100'
  }
  return window.location.origin
}

function resolveFrontendOrigin(backendUrl: string): string {
  try {
    const parsed = new URL(backendUrl)
    parsed.port = '5173'
    return parsed.toString().replace(/\/$/, '')
  } catch {
    return 'http://127.0.0.1:5173'
  }
}

export function createDefaultRegistryConfig(
  partial?: Partial<DatabaseConfig>,
): DatabaseConfig {
  const base: DatabaseConfig = {
    name: '默认配置',
    manual_db_nums: [],
    manual_refnos: [],
    enabled_nouns: null,
    excluded_nouns: null,
    debug_limit_per_noun_type: null,
    project_name: '',
    project_path: '',
    project_code: 1516,
    mdb_name: 'ALL',
    module: 'DESI',
    db_type: 'surrealdb',
    surreal_ns: 1516,
    db_ip: 'localhost',
    db_port: '8020',
    db_user: 'root',
    db_password: 'root',
    gen_model: true,
    gen_mesh: false,
    gen_spatial_tree: true,
    apply_boolean_operation: true,
    mesh_tol_ratio: 3.0,
    room_keyword: '-RM',
    target_sesno: null,
    meshes_path: null,
    export_json: false,
    export_parquet: true,
  }

  return {
    ...base,
    ...partial,
    manual_db_nums: partial?.manual_db_nums ?? base.manual_db_nums,
    manual_refnos: partial?.manual_refnos ?? base.manual_refnos,
    enabled_nouns: partial?.enabled_nouns ?? base.enabled_nouns,
    excluded_nouns: partial?.excluded_nouns ?? base.excluded_nouns,
    debug_limit_per_noun_type:
      partial?.debug_limit_per_noun_type ?? base.debug_limit_per_noun_type,
  }
}

export function createEmptyRegistryForm(): RegistrySiteForm {
  const backendUrl = resolveBackendOrigin()
  const config = createDefaultRegistryConfig()
  const bindPort =
    typeof window !== 'undefined' && window.location.port
      ? Number(window.location.port)
      : 3100
  return {
    site_id: '',
    name: '',
    description: '',
    region: '',
    env: '',
    project_name: '',
    project_path: '',
    project_code: 1516,
    frontend_url: resolveFrontendOrigin(backendUrl),
    backend_url: backendUrl,
    bind_host: '0.0.0.0',
    bind_port: bindPort,
    owner: '',
    health_url: '',
    notes: '',
    config_json: JSON.stringify(config, null, 2),
  }
}

export function createRegistryFormFromSite(site: RegistrySite): RegistrySiteForm {
  const base = createEmptyRegistryForm()
  const config = createDefaultRegistryConfig(site.config ?? {})
  const projectName = site.project_name || config.project_name || base.project_name
  const projectPath = site.project_path || config.project_path || base.project_path
  const projectCode =
    Number(site.project_code ?? config.project_code ?? base.project_code) ||
    base.project_code

  config.project_name = projectName
  config.project_path = projectPath
  config.project_code = projectCode
  config.surreal_ns = Number(config.surreal_ns || projectCode)
  config.name = config.name || site.name || '默认配置'

  return {
    site_id: site.site_id || site.id || '',
    name: site.name || '',
    description: site.description || '',
    region: site.region || '',
    env: site.env || '',
    project_name: projectName,
    project_path: projectPath,
    project_code: projectCode,
    frontend_url: site.frontend_url || '',
    backend_url: site.backend_url || site.url || '',
    bind_host: site.bind_host || '0.0.0.0',
    bind_port: Number(site.bind_port || base.bind_port) || base.bind_port,
    owner: site.owner || '',
    health_url: site.health_url || '',
    notes: site.notes || '',
    config_json: JSON.stringify(config, null, 2),
  }
}

export function buildRegistrySitePayload(
  form: RegistrySiteForm,
): RegistrySiteMutationPayload {
  const name = form.name.trim()
  if (!name) {
    throw new Error('站点名称不能为空')
  }

  const projectName = form.project_name.trim()
  if (!projectName) {
    throw new Error('项目名称不能为空')
  }

  const bindPort = Number(form.bind_port)
  if (!Number.isInteger(bindPort) || bindPort < 1 || bindPort > 65535) {
    throw new Error('监听端口必须为 1-65535 之间的整数')
  }

  const projectCode = Number(form.project_code)
  if (!Number.isInteger(projectCode) || projectCode <= 0) {
    throw new Error('项目代号必须大于 0')
  }

  validateHttpUrl(form.frontend_url, '前端地址')
  validateHttpUrl(form.backend_url, '后端地址')
  if (form.health_url.trim()) {
    validateHttpUrl(form.health_url, '健康检查地址')
  }

  let parsedConfig: unknown
  try {
    parsedConfig = JSON.parse(form.config_json || '{}')
  } catch {
    throw new Error('高级配置不是合法的 JSON')
  }

  if (parsedConfig == null || Array.isArray(parsedConfig) || typeof parsedConfig !== 'object') {
    throw new Error('高级配置必须是 JSON 对象')
  }

  const config = createDefaultRegistryConfig(parsedConfig as Partial<DatabaseConfig>)
  const projectPath = form.project_path.trim()

  config.project_name = projectName
  config.project_path = projectPath
  config.project_code = projectCode
  config.surreal_ns = Number(config.surreal_ns || projectCode)
  config.name = config.name || name || '默认配置'

  return {
    site_id: form.site_id.trim() || undefined,
    name,
    description: form.description.trim() || null,
    region: form.region.trim() || null,
    env: form.env.trim() || null,
    project_name: projectName,
    project_path: projectPath || null,
    project_code: projectCode,
    frontend_url: form.frontend_url.trim() || null,
    backend_url: form.backend_url.trim() || null,
    bind_host: form.bind_host.trim() || '0.0.0.0',
    bind_port: bindPort,
    owner: form.owner.trim() || null,
    health_url: form.health_url.trim() || null,
    notes: form.notes.trim() || null,
    selected_projects: projectPath ? [projectPath] : [],
    config,
  }
}

function validateHttpUrl(value: string, label: string) {
  try {
    const parsed = new URL(value)
    if (!parsed.protocol.startsWith('http')) {
      throw new Error('invalid protocol')
    }
  } catch {
    throw new Error(`${label}格式不正确`)
  }
}

function normalizeUrl(value?: string | null): string {
  return (value || '').trim().replace(/\/$/, '')
}

export function findLinkedLocalSite(
  registrySite: RegistrySite,
  localSites: ManagedProjectSite[],
): ManagedProjectSite | null {
  const registrySiteId = registrySite.site_id.trim()
  const registryProjectPath = (registrySite.project_path || '').trim()
  const registryBackendUrl = normalizeUrl(registrySite.backend_url || registrySite.url)

  return (
    localSites.find((localSite) => {
      const localEntryUrl = normalizeUrl(localSite.entry_url)
      return (
        (registrySiteId !== '' && localSite.site_id === registrySiteId) ||
        (registryProjectPath !== '' && localSite.project_path === registryProjectPath) ||
        (registryBackendUrl !== '' && localEntryUrl !== '' && registryBackendUrl === localEntryUrl)
      )
    }) || null
  )
}

export function getRegistryStatusLabel(status: string): string {
  const labels: Record<string, string> = {
    Configuring: '配置中',
    Deploying: '部署中',
    Running: '运行中',
    Failed: '失败',
    Stopped: '已停止',
    Offline: '离线',
  }

  return labels[status] ?? status
}
