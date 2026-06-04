<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useSitesStore } from '@/stores/sites'
import { sitesApi } from '@/api/sites'
import type {
  CreateManagedSiteRequest,
  ManagedProjectSite,
  ManagedSiteParsePlan,
  PreviewManagedSiteParsePlanRequest,
  UpdateManagedSiteRequest,
  ProjectRole,
  ScannedDbnumConflict,
  SiteProject,
} from '@/types/site'
import {
  DEFAULT_PARSE_DB_TYPES,
  MODEL_PARSE_DB_TYPE_OPTIONS,
  PARSE_PRESET_OPTIONS,
  SYSTEM_PARSE_DB_TYPE_OPTIONS,
  matchParsePreset,
  normalizeParseDbTypes,
} from './parse-db-types'
import { parsePlanClass } from './site-status'
import { MANAGED_SITE_FORM_PRESETS, type ManagedSiteFormPreset } from './site-presets'
import { X } from 'lucide-vue-next'

const props = defineProps<{
  open: boolean
  siteId: string | null
  /**
   * D6 / Sprint D · 修 G14：克隆站点模式
   *
   * 开启后从 `siteId` 拉取既有站点配置，但**保持创建语义**：
   * - 标题为「克隆站点」
   * - project_name 自动加 ` (副本)` 后缀
   * - 默认交给后端自动分配 db_port / web_port
   * - 凭据强制清空，必须重填
   * - 提交走 createSite 而非 updateSite
   */
  clone?: boolean
}>()

const emit = defineEmits<{
  close: []
  saved: [payload?: { site: ManagedProjectSite; autoDeploy: boolean }]
}>()

const sitesStore = useSitesStore()
const saving = ref(false)
const error = ref('')
const existingSite = ref<ManagedProjectSite | null>(null)
const previewLoading = ref(false)
const previewError = ref('')
const previewPlan = ref<ManagedSiteParsePlan | null>(null)
let previewTimer: ReturnType<typeof setTimeout> | null = null
let previewRequestSeq = 0
const DEFAULT_DB_PORT = 8020
const DEFAULT_WEB_PORT = 8080

// D4 / Sprint D · 修 G12：端口冲突前端预检
//
// Drawer 提交前 onBlur 调 /api/admin/ports/check，给用户立即反馈端口是否
// 已被本机其他进程占用。提示是软警告（不阻断提交，因为：
//   1. 编辑既有站点时端口可能本来就归这个站点的子进程，自我冲突属正常
//   2. 后端创建/启动会再次校验，是真正的 source of truth）。
type PortFieldKey = 'db_port' | 'web_port'
type PortStatus =
  | { state: 'idle' }
  | { state: 'checking' }
  | { state: 'available' }
  | { state: 'in_use'; pids: number[] }
  | { state: 'error'; message: string }

const portStatuses = ref<Record<PortFieldKey, PortStatus>>({
  db_port: { state: 'idle' },
  web_port: { state: 'idle' },
})
const portCheckSeq: Record<PortFieldKey, number> = {
  db_port: 0,
  web_port: 0,
}

async function checkPortField(field: PortFieldKey) {
  if (autoAllocatePorts.value && !isEditing.value) {
    portStatuses.value[field] = { state: 'idle' }
    return
  }
  const port = field === 'db_port' ? form.value.db_port : form.value.web_port
  if (!port || port < 1 || port > 65535) {
    portStatuses.value[field] = { state: 'idle' }
    return
  }
  // 编辑模式下，如端口与既有 site 一致，跳过预检（自己占自己不算冲突）
  if (existingSite.value) {
    const stored = field === 'db_port' ? existingSite.value.db_port : existingSite.value.web_port
    if (stored === port) {
      portStatuses.value[field] = { state: 'idle' }
      return
    }
  }
  const seq = ++portCheckSeq[field]
  portStatuses.value[field] = { state: 'checking' }
  try {
    const result = await sitesApi.checkPort(port, form.value.bind_host?.trim() || undefined)
    if (seq !== portCheckSeq[field]) return
    if (result.in_use) {
      portStatuses.value[field] = { state: 'in_use', pids: result.pids }
    } else {
      portStatuses.value[field] = { state: 'available' }
    }
  } catch (e) {
    if (seq !== portCheckSeq[field]) return
    portStatuses.value[field] = {
      state: 'error',
      message: e instanceof Error ? e.message : '端口探测失败',
    }
  }
}

function portStatusLabel(status: PortStatus): string {
  switch (status.state) {
    case 'idle':
      return ''
    case 'checking':
      return '端口探测中...'
    case 'available':
      return '端口空闲，可用'
    case 'in_use':
      return `端口已被本机进程占用 (PIDs: ${status.pids.join(', ')})`
    case 'error':
      return `端口探测失败：${status.message}`
  }
}

function portStatusClass(status: PortStatus): string {
  switch (status.state) {
    case 'available':
      return 'text-emerald-600 dark:text-emerald-400'
    case 'in_use':
      return 'text-amber-600 dark:text-amber-400'
    case 'error':
      return 'text-destructive'
    default:
      return 'text-muted-foreground'
  }
}

const form = ref<CreateManagedSiteRequest>({
  project_name: '',
  project_path: '',
  project_code: 0,
  manual_db_nums: [],
  parse_db_types: [...DEFAULT_PARSE_DB_TYPES],
  force_rebuild_system_db: false,
  gen_model: true,
  gen_mesh: false,
  gen_spatial_tree: true,
  apply_boolean_operation: true,
  mesh_tol_ratio: 3.0,
  export_json: false,
  export_parquet: true,
  pipeline_db_mode: 'file',
  runtime_db_mode: 'ws',
  db_port: DEFAULT_DB_PORT,
  web_port: DEFAULT_WEB_PORT,
  bind_host: '127.0.0.1',
  public_base_url: '',
  associated_project: '',
  db_user: '',
  db_password: '',
})

const manualDbNumsStr = ref('')
const autoAllocatePorts = ref(true)

// ─── Phase 4 · 多工程合并站点（可选） ────────────────────────────────────────
//
// 站点可包含多个工程条目（projects[]），恰好一个 primary，区分 design / library。
// 留空（projects 为空）时退回旧单工程语义，后端按 project_path 处理。
const siteName = ref('')
const projects = ref<SiteProject[]>([])
const scanRoot = ref('')
const scanLoading = ref(false)
const scanError = ref('')
const scanConflicts = ref<ScannedDbnumConflict[]>([])

function resetMultiProjectState(site: ManagedProjectSite | null, cloning: boolean) {
  siteName.value = site && !cloning ? (site.site_name ?? '') : ''
  projects.value = site?.projects?.length
    ? site.projects.map((p) => ({ ...p }))
    : []
  scanRoot.value = ''
  scanError.value = ''
  scanConflicts.value = []
  ensureSinglePrimary()
}

function ensureSinglePrimary() {
  if (!projects.value.length) return
  const primaries = projects.value.filter((p) => p.is_primary)
  if (primaries.length === 1) return
  projects.value.forEach((p) => (p.is_primary = false))
  const designIdx = projects.value.findIndex((p) => p.role === 'design')
  projects.value[designIdx >= 0 ? designIdx : 0].is_primary = true
}

function addProjectRow() {
  projects.value.push({
    path: '',
    name: '',
    role: 'design',
    is_primary: projects.value.length === 0,
    sort_order: projects.value.length,
  })
}

function removeProjectRow(idx: number) {
  const wasPrimary = projects.value[idx]?.is_primary
  projects.value.splice(idx, 1)
  if (wasPrimary) ensureSinglePrimary()
}

function setPrimary(idx: number) {
  projects.value.forEach((p, i) => (p.is_primary = i === idx))
}

function setProjectRole(idx: number, role: ProjectRole) {
  const target = projects.value[idx]
  if (target) target.role = role
}

async function runScan() {
  const root = scanRoot.value.trim()
  if (!root) {
    scanError.value = '请输入要扫描的根路径'
    return
  }
  scanLoading.value = true
  scanError.value = ''
  try {
    const result = await sitesApi.scanProjects(root)
    scanConflicts.value = result.conflicts ?? []
    const existingPaths = new Set(projects.value.map((p) => p.path))
    for (const candidate of result.projects) {
      if (existingPaths.has(candidate.path)) continue
      projects.value.push({
        path: candidate.path,
        name: candidate.name,
        role: candidate.role,
        is_primary: false,
        sort_order: projects.value.length,
      })
    }
    ensureSinglePrimary()
    if (!result.projects.length) {
      scanError.value = '该根路径下未发现包含 db 文件的候选工程'
    }
  } catch (e) {
    scanError.value = e instanceof Error ? e.message : '工程扫描失败'
  } finally {
    scanLoading.value = false
  }
}

const multiProjectError = computed<string | null>(() => {
  if (!projects.value.length) return null
  if (projects.value.some((p) => !p.path.trim())) return '存在未填写路径的工程条目'
  const designCount = projects.value.filter((p) => p.role === 'design').length
  if (designCount === 0) return '至少需要一个 design（设计）工程'
  const primaryCount = projects.value.filter((p) => p.is_primary).length
  if (primaryCount !== 1) return `必须恰好指定一个主工程，当前为 ${primaryCount} 个`
  const names = projects.value.map((p) => (p.name.trim() || p.path).toLowerCase())
  if (new Set(names).size !== names.length) return '工程名/路径重复，需唯一'
  return null
})

function buildProjectsPayload(): SiteProject[] | undefined {
  if (!projects.value.length) return undefined
  return projects.value.map((p, idx) => ({
    path: p.path.trim(),
    name: p.name.trim(),
    role: p.role,
    is_primary: p.is_primary,
    sort_order: idx,
  }))
}

const isEditing = computed(() => !!props.siteId && !props.clone)
const isCloning = computed(() => !!props.siteId && !!props.clone)
const title = computed(() => {
  if (isCloning.value) return '克隆站点'
  return isEditing.value ? '编辑站点' : '新建站点'
})

function applySitePreset(preset: ManagedSiteFormPreset) {
  if (isEditing.value || isCloning.value) return

  const presetForm = preset.form
  const manualDbNums = [...(presetForm.manual_db_nums ?? [])]
  form.value = {
    project_name: presetForm.project_name,
    project_path: presetForm.project_path,
    project_code: presetForm.project_code,
    manual_db_nums: manualDbNums,
    parse_db_types: normalizeParseDbTypes(presetForm.parse_db_types ?? []),
    force_rebuild_system_db: presetForm.force_rebuild_system_db ?? false,
    auto_parse_related_dbnums: presetForm.auto_parse_related_dbnums ?? false,
    gen_model: presetForm.gen_model ?? true,
    gen_mesh: presetForm.gen_mesh ?? false,
    gen_spatial_tree: presetForm.gen_spatial_tree ?? true,
    apply_boolean_operation: presetForm.apply_boolean_operation ?? true,
    mesh_tol_ratio: presetForm.mesh_tol_ratio ?? 3.0,
    export_json: presetForm.export_json ?? false,
    export_parquet: presetForm.export_parquet ?? true,
    pipeline_db_mode: presetForm.pipeline_db_mode ?? 'file',
    runtime_db_mode: presetForm.runtime_db_mode ?? 'ws',
    db_port: presetForm.db_port ?? DEFAULT_DB_PORT,
    web_port: presetForm.web_port ?? DEFAULT_WEB_PORT,
    bind_host: presetForm.bind_host ?? '127.0.0.1',
    public_base_url: presetForm.public_base_url ?? '',
    associated_project: presetForm.associated_project ?? '',
    db_user: presetForm.db_user ?? '',
    db_password: presetForm.db_password ?? '',
    auto_deploy: presetForm.auto_deploy,
  }
  manualDbNumsStr.value = manualDbNums.join(', ')
  siteName.value = presetForm.site_name ?? ''
  projects.value = (presetForm.projects ?? []).map((project) => ({ ...project }))
  scanRoot.value = ''
  scanError.value = ''
  scanConflicts.value = []
  autoAllocatePorts.value = presetForm.db_port === undefined && presetForm.web_port === undefined
  portStatuses.value = {
    db_port: { state: 'idle' },
    web_port: { state: 'idle' },
  }
  ensureSinglePrimary()
  schedulePreview()
}

const WEAK_CREDENTIAL_SET = new Set([
  'root/root',
  'admin/admin',
  'admin/123456',
  'root/123456',
  'test/test',
])

const weakCredentialsWarning = computed<string | null>(() => {
  const user = (form.value.db_user || '').trim().toLowerCase()
  const password = (form.value.db_password || '').trim().toLowerCase()
  if (!user || !password) return null
  if (WEAK_CREDENTIAL_SET.has(`${user}/${password}`)) {
    return '检测到常见弱凭据（root/root、admin/admin 等）。后端会拒绝此组合；本地开发可设置 AIOS_ALLOW_WEAK_DB_CREDS=1 临时放行。'
  }
  return null
})

function parseManualDbNumsInput(value: string) {
  return value
    .split(/[,\s]+/)
    .map(Number)
    .filter((n) => !isNaN(n) && n > 0)
}

watch([() => props.open, () => props.siteId], async ([open, siteId]) => {
  if (!open) return
  error.value = ''
  previewError.value = ''
  portStatuses.value = {
    db_port: { state: 'idle' },
    web_port: { state: 'idle' },
  }
  if (siteId) {
    try {
      existingSite.value = await sitesApi.get(siteId)
      const s = existingSite.value
      const cloning = props.clone === true
      form.value = {
        project_name: cloning ? `${s.project_name} (副本)` : s.project_name,
        project_path: s.project_path,
        project_code: s.project_code,
        manual_db_nums: s.manual_db_nums,
        parse_db_types: s.parse_db_types?.length ? [...s.parse_db_types] : [...DEFAULT_PARSE_DB_TYPES],
        force_rebuild_system_db: s.force_rebuild_system_db ?? false,
        gen_model: s.gen_model ?? true,
        gen_mesh: s.gen_mesh ?? false,
        gen_spatial_tree: s.gen_spatial_tree ?? true,
        apply_boolean_operation: s.apply_boolean_operation ?? true,
        mesh_tol_ratio: s.mesh_tol_ratio ?? 3.0,
        export_json: s.export_json ?? false,
        export_parquet: s.export_parquet ?? true,
        pipeline_db_mode: s.pipeline_db_mode ?? 'file',
        runtime_db_mode: s.runtime_db_mode ?? 'ws',
        db_port: s.db_port,
        web_port: s.web_port,
        bind_host: s.bind_host || '127.0.0.1',
        public_base_url: s.public_base_url || '',
        associated_project: s.associated_project || '',
        db_user: '',
        db_password: '',
      }
      manualDbNumsStr.value = s.manual_db_nums.join(', ')
      resetMultiProjectState(s, cloning)
      // 克隆模式下不保留 existingSite，避免抽屉展示「正在编辑某 site」徽标
      if (cloning) {
        existingSite.value = null
        autoAllocatePorts.value = true
      } else {
        autoAllocatePorts.value = false
      }
    } catch (e) {
      error.value = e instanceof Error ? e.message : 'Failed to load site'
    }
  } else {
    existingSite.value = null
    form.value = {
      project_name: '',
      project_path: '',
      project_code: 0,
      manual_db_nums: [],
      parse_db_types: [...DEFAULT_PARSE_DB_TYPES],
      force_rebuild_system_db: false,
      gen_model: true,
      gen_mesh: false,
      gen_spatial_tree: true,
      apply_boolean_operation: true,
      mesh_tol_ratio: 3.0,
      export_json: false,
      export_parquet: true,
      pipeline_db_mode: 'file',
      runtime_db_mode: 'ws',
      db_port: DEFAULT_DB_PORT,
      web_port: DEFAULT_WEB_PORT,
      bind_host: '127.0.0.1',
      public_base_url: '',
      associated_project: '',
      db_user: '',
      db_password: '',
    }
    manualDbNumsStr.value = ''
    autoAllocatePorts.value = true
    resetMultiProjectState(null, false)
  }
  schedulePreview()
})

function parseDbNums() {
  form.value.manual_db_nums = parseManualDbNumsInput(manualDbNumsStr.value)
}

function toggleParseDbType(type: string) {
  const current = new Set(normalizeParseDbTypes(form.value.parse_db_types ?? []))
  if (current.has(type)) {
    current.delete(type)
  } else {
    current.add(type)
  }
  form.value.parse_db_types = [...current].sort()
  if (!current.has('SYST')) {
    form.value.force_rebuild_system_db = false
  }
}

function hasParseDbType(type: string) {
  return normalizeParseDbTypes(form.value.parse_db_types ?? []).includes(type)
}

const canForceRebuildSystemDb = computed(() => hasParseDbType('SYST'))
const activePresetKey = computed(() => matchParsePreset(
  form.value.parse_db_types ?? [],
  form.value.force_rebuild_system_db ?? false,
)?.key ?? '')

function applyParsePreset(presetKey: string) {
  const preset = PARSE_PRESET_OPTIONS.find((item) => item.key === presetKey)
  if (!preset) return
  form.value.parse_db_types = [...preset.parseDbTypes]
  form.value.force_rebuild_system_db = preset.forceRebuildSystemDb
}

const previewPayload = computed<PreviewManagedSiteParsePlanRequest | null>(() => {
  const projectName = form.value.project_name.trim()
  const projectPath = form.value.project_path.trim()
  if (!props.open || !projectName || !projectPath) {
    return null
  }
  const previewWebPort = form.value.web_port || DEFAULT_WEB_PORT
  const parseDbTypes = normalizeParseDbTypes(form.value.parse_db_types ?? [])
  return {
    site_id: props.siteId ?? undefined,
    project_name: projectName,
    project_path: projectPath,
    manual_db_nums: parseManualDbNumsInput(manualDbNumsStr.value),
    parse_db_types: parseDbTypes,
    force_rebuild_system_db: parseDbTypes.includes('SYST') ? !!form.value.force_rebuild_system_db : false,
    web_port: previewWebPort,
    bind_host: form.value.bind_host?.trim() || undefined,
    public_base_url: form.value.public_base_url?.trim() || undefined,
    associated_project: form.value.associated_project?.trim() || undefined,
  }
})

function resetPreviewState() {
  previewLoading.value = false
  previewError.value = ''
  previewPlan.value = null
}

async function refreshPreview() {
  const payload = previewPayload.value
  if (!payload) {
    resetPreviewState()
    return
  }
  const requestSeq = ++previewRequestSeq
  previewLoading.value = true
  previewError.value = ''
  try {
    const plan = await sitesApi.previewParsePlan(payload)
    if (requestSeq !== previewRequestSeq) return
    previewPlan.value = plan
  } catch (e) {
    if (requestSeq !== previewRequestSeq) return
    previewPlan.value = null
    previewError.value = e instanceof Error ? e.message : '解析预览加载失败'
  } finally {
    if (requestSeq === previewRequestSeq) {
      previewLoading.value = false
    }
  }
}

function schedulePreview() {
  if (previewTimer) {
    clearTimeout(previewTimer)
  }
  previewTimer = setTimeout(() => {
    void refreshPreview()
  }, 250)
}

watch(previewPayload, () => {
  if (!props.open) return
  schedulePreview()
}, { deep: true })

watch(() => props.open, (open) => {
  if (open) {
    schedulePreview()
    return
  }
  if (previewTimer) {
    clearTimeout(previewTimer)
    previewTimer = null
  }
  previewRequestSeq += 1
  resetPreviewState()
})

onBeforeUnmount(() => {
  if (previewTimer) {
    clearTimeout(previewTimer)
    previewTimer = null
  }
})

const canSubmit = computed(() => {
  if (!form.value.project_name || !form.value.project_path) return false
  if (multiProjectError.value) return false
  if (!isEditing.value && (!form.value.db_user?.trim() || !form.value.db_password?.trim())) return false
  if (!isEditing.value && autoAllocatePorts.value) return true
  return !!form.value.db_port && !!form.value.web_port
})

async function handleSubmit(autoDeploy = false) {
  saving.value = true
  error.value = ''
  parseDbNums()
  form.value.parse_db_types = normalizeParseDbTypes(form.value.parse_db_types ?? [])
  if (!form.value.parse_db_types.includes('SYST')) {
    form.value.force_rebuild_system_db = false
  }
  if (!Number.isFinite(form.value.mesh_tol_ratio ?? NaN) || (form.value.mesh_tol_ratio ?? 0) <= 0) {
    form.value.mesh_tol_ratio = 3.0
  }
  try {
    // 克隆模式走 create 路径（不是 update），保持新建语义
    const siteNameTrimmed = siteName.value.trim()
    const projectsPayload = buildProjectsPayload()
    if (isEditing.value && props.siteId) {
      const payload: UpdateManagedSiteRequest = {
        ...form.value,
        site_name: siteNameTrimmed || undefined,
        projects: projectsPayload,
        db_user: form.value.db_user?.trim() ? form.value.db_user.trim() : undefined,
        db_password: form.value.db_password?.trim() ? form.value.db_password.trim() : undefined,
      }
      await sitesStore.updateSite(props.siteId, payload)
    } else {
      const payload: CreateManagedSiteRequest = {
        ...form.value,
        site_name: siteNameTrimmed || undefined,
        projects: projectsPayload,
        auto_deploy: autoDeploy,
        db_user: form.value.db_user?.trim() || '',
        db_password: form.value.db_password?.trim() || '',
      }
      if (autoAllocatePorts.value) {
        delete payload.db_port
        delete payload.web_port
      }
      const site = await sitesStore.createSite(payload)
      emit('saved', { site, autoDeploy })
      return
    }
    emit('saved')
  } catch (e) {
    error.value = e instanceof Error ? e.message : 'Save failed'
  } finally {
    saving.value = false
  }
}

const inputClass = 'flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring'
</script>

<template>
  <Teleport to="body">
    <Transition name="drawer">
      <div v-if="open" class="fixed inset-0 z-50">
        <div class="absolute inset-0 bg-black/50" @click="emit('close')" />
        <div class="absolute right-0 top-0 h-full w-full max-w-[480px] bg-background border-l border-border shadow-xl flex flex-col">
          <!-- Header -->
          <div class="flex items-center justify-between border-b border-border px-6 py-4">
            <div>
              <h3 class="text-lg font-semibold">{{ title }}</h3>
              <div v-if="existingSite" class="mt-1">
                <span class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
                  :class="existingSite.status === 'Running' ? 'bg-green-100 text-green-800' :
                           existingSite.status === 'Failed' ? 'bg-red-100 text-red-800' :
                           'bg-muted text-muted-foreground'">
                  {{ existingSite.status }}
                </span>
              </div>
            </div>
            <button @click="emit('close')"
              class="inline-flex h-8 w-8 items-center justify-center rounded-md hover:bg-accent transition-colors">
              <X class="h-4 w-4" />
            </button>
          </div>

          <!-- Form -->
          <form class="flex-1 overflow-auto px-6 py-4 space-y-6" @submit.prevent="handleSubmit(false)">
            <section
              v-if="!isEditing && !isCloning"
              class="rounded-lg border border-primary/20 bg-primary/5 p-4 space-y-3"
            >
              <div class="space-y-1">
                <div class="text-sm font-medium">加载示例样例</div>
                <p class="text-xs text-muted-foreground">
                  选择一个模板先填入常用配置，之后仍可按本机路径、端口和凭据继续调整。
                </p>
              </div>
              <button
                v-for="preset in MANAGED_SITE_FORM_PRESETS"
                :key="preset.key"
                type="button"
                class="w-full rounded-lg border border-border/60 bg-background px-3 py-2 text-left transition-colors hover:border-primary/50 hover:bg-primary/10 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                @click="applySitePreset(preset)"
              >
                <div class="flex items-center justify-between gap-3">
                  <span class="text-sm font-medium">{{ preset.label }}</span>
                  <span class="shrink-0 rounded-full bg-primary/10 px-2 py-0.5 text-xs font-medium text-primary">
                    {{ preset.badge }}
                  </span>
                </div>
                <p class="mt-1 text-xs text-muted-foreground">{{ preset.detail }}</p>
              </button>
            </section>

            <fieldset class="space-y-3">
              <legend class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">项目信息</legend>
              <div class="space-y-2">
                <label class="text-sm font-medium">项目名称 *</label>
                <input v-model="form.project_name" type="text" required placeholder="例：AvevaMarineSample" :class="inputClass" />
              </div>
              <div class="space-y-2">
                <label class="text-sm font-medium">项目路径 *</label>
                <input v-model="form.project_path" type="text" required placeholder="/path/to/e3d_models" :class="inputClass" />
              </div>
              <div class="space-y-2">
                <label class="text-sm font-medium">项目代码 *</label>
                <input v-model.number="form.project_code" type="number" required min="1" :class="inputClass" />
              </div>
              <div class="space-y-2">
                <label class="text-sm font-medium">关联工程 <span class="text-muted-foreground">(可选，可逗号分隔)</span></label>
                <input v-model="form.associated_project" type="text" :placeholder="form.project_name || '默认使用项目名称'" :class="inputClass" />
                <p class="text-xs text-muted-foreground">
                  用于解析源目录和打开 Viewer 的真实 E3D 工程名。AvevaPlantSample 会自动带上同级 AvevaCatalogue 元件库。
                </p>
              </div>
            </fieldset>

            <fieldset class="space-y-3">
              <legend class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">工程组成（多工程，可选）</legend>
              <div class="rounded-lg border border-border/60 bg-background p-4 space-y-3">
                <div class="space-y-2">
                  <label class="text-sm font-medium">站点名称 <span class="text-muted-foreground">(可选)</span></label>
                  <input v-model="siteName" type="text" :placeholder="form.project_name || '多工程合并站点的显示名'" :class="inputClass" />
                </div>
                <div class="space-y-2">
                  <label class="text-sm font-medium">扫描根目录</label>
                  <p class="text-xs text-muted-foreground">填一个含多个工程子目录的根路径，自动发现候选工程并推断角色。工程列表留空则按上方单工程路径处理。</p>
                  <div class="flex gap-2">
                    <input
                      v-model="scanRoot"
                      type="text"
                      placeholder="/path/to/projects-root"
                      :class="inputClass"
                      @keydown.enter.prevent="runScan"
                    />
                    <button
                      type="button"
                      :disabled="scanLoading"
                      class="inline-flex h-9 shrink-0 items-center rounded-md border border-input bg-transparent px-3 text-sm font-medium hover:bg-accent transition-colors disabled:opacity-50"
                      @click="runScan"
                    >
                      {{ scanLoading ? '扫描中...' : '扫描' }}
                    </button>
                    <button
                      type="button"
                      class="inline-flex h-9 shrink-0 items-center rounded-md border border-input bg-transparent px-3 text-sm font-medium hover:bg-accent transition-colors"
                      @click="addProjectRow"
                    >
                      手动添加
                    </button>
                  </div>
                  <p v-if="scanError" class="text-xs text-destructive">{{ scanError }}</p>
                </div>

                <div
                  v-if="scanConflicts.length"
                  class="rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-xs text-amber-800 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-200"
                >
                  <div class="font-medium">检测到 dbnum 冲突（需消解后再保存）</div>
                  <ul class="mt-1 list-disc pl-4">
                    <li v-for="conflict in scanConflicts" :key="conflict.dbnum">
                      dbnum {{ conflict.dbnum }}：{{ conflict.projects.join(' / ') }}
                    </li>
                  </ul>
                </div>

                <div v-if="projects.length" class="space-y-2">
                  <div
                    v-for="(proj, idx) in projects"
                    :key="idx"
                    class="rounded-lg border border-border/60 bg-muted/20 p-3 space-y-2"
                  >
                    <div class="flex items-center gap-2">
                      <input
                        v-model="proj.name"
                        type="text"
                        placeholder="工程名"
                        class="h-8 w-28 shrink-0 rounded-md border border-input bg-transparent px-2 text-sm"
                      />
                      <input
                        v-model="proj.path"
                        type="text"
                        placeholder="工程绝对路径"
                        :class="inputClass"
                      />
                      <button
                        type="button"
                        class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-destructive hover:bg-destructive/10 transition-colors"
                        @click="removeProjectRow(idx)"
                      >
                        <X class="h-4 w-4" />
                      </button>
                    </div>
                    <div class="flex flex-wrap items-center gap-4 text-xs">
                      <div class="flex items-center gap-2">
                        <span class="text-muted-foreground">角色</span>
                        <select
                          :value="proj.role"
                          class="h-8 rounded-md border border-input bg-transparent px-2 text-sm"
                          @change="setProjectRole(idx, ($event.target as HTMLSelectElement).value as ProjectRole)"
                        >
                          <option value="design">design（设计）</option>
                          <option value="library">library（元件库）</option>
                        </select>
                      </div>
                      <label class="flex items-center gap-1.5 cursor-pointer">
                        <input type="radio" :checked="proj.is_primary" @change="setPrimary(idx)" />
                        <span>主工程</span>
                      </label>
                    </div>
                  </div>
                </div>
                <p v-else class="text-xs text-muted-foreground">
                  未配置多工程，将按上方「项目路径」作为单工程站点处理。
                </p>

                <div
                  v-if="multiProjectError"
                  class="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-xs text-destructive"
                >
                  {{ multiProjectError }}
                </div>
              </div>
            </fieldset>

            <fieldset class="space-y-3">
              <legend class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">运行配置</legend>
              <label v-if="!isEditing" class="flex items-start gap-2 rounded-lg border border-border/60 bg-muted/30 p-3 text-sm">
                <input v-model="autoAllocatePorts" type="checkbox" class="mt-0.5" />
                <span>
                  <span class="font-medium">自动分配端口</span>
                  <span class="mt-1 block text-xs text-muted-foreground">
                    保存时后端会从 DB {{ DEFAULT_DB_PORT }} / Web {{ DEFAULT_WEB_PORT }} 起自动选择空闲端口。
                  </span>
                </span>
              </label>
              <div
                v-if="!isEditing && autoAllocatePorts"
                class="rounded-md border border-emerald-200 bg-emerald-50 px-3 py-2 text-xs text-emerald-800 dark:border-emerald-800 dark:bg-emerald-950 dark:text-emerald-200"
              >
                无需手动填写端口；创建成功后会在站点详情中显示实际分配的 DB/Web 端口。
              </div>
              <div v-else class="grid grid-cols-2 gap-4">
                <div class="space-y-2">
                  <label class="text-sm font-medium">DB 端口 *</label>
                  <input
                    v-model.number="form.db_port"
                    type="number"
                    required
                    min="1"
                    max="65535"
                    :class="inputClass"
                    @blur="checkPortField('db_port')"
                  />
                  <p
                    v-if="portStatuses.db_port.state !== 'idle'"
                    class="text-xs"
                    :class="portStatusClass(portStatuses.db_port)"
                  >
                    {{ portStatusLabel(portStatuses.db_port) }}
                  </p>
                </div>
                <div class="space-y-2">
                  <label class="text-sm font-medium">Web 端口 *</label>
                  <input
                    v-model.number="form.web_port"
                    type="number"
                    required
                    min="1"
                    max="65535"
                    :class="inputClass"
                    @blur="checkPortField('web_port')"
                  />
                  <p
                    v-if="portStatuses.web_port.state !== 'idle'"
                    class="text-xs"
                    :class="portStatusClass(portStatuses.web_port)"
                  >
                    {{ portStatusLabel(portStatuses.web_port) }}
                  </p>
                </div>
              </div>
              <div class="grid grid-cols-2 gap-4">
                <div class="space-y-2">
                  <label class="text-sm font-medium">解析/生成 DB 模式</label>
                  <select v-model="form.pipeline_db_mode" :class="inputClass">
                    <option value="file">file（默认，离线文件）</option>
                    <option value="ws">ws（连接服务）</option>
                  </select>
                  <p class="text-xs text-muted-foreground">解析和模型生成默认使用 file，避免依赖已启动站点。</p>
                </div>
                <div class="space-y-2">
                  <label class="text-sm font-medium">站点运行 DB 模式</label>
                  <select v-model="form.runtime_db_mode" :class="inputClass">
                    <option value="ws">ws（默认，启动站点服务）</option>
                    <option value="file">file（单进程文件）</option>
                  </select>
                  <p class="text-xs text-muted-foreground">正式启动站点默认使用 ws，由站点进程连接 SurrealDB 服务。</p>
                </div>
              </div>
              <div class="space-y-2">
                <label class="text-sm font-medium">绑定地址</label>
                <input v-model="form.bind_host" type="text" placeholder="127.0.0.1" :class="inputClass" />
                <p class="text-xs text-muted-foreground">默认只监听本机，避免把管理数据库直接暴露到外网</p>
              </div>
              <div class="space-y-2">
                <label class="text-sm font-medium">对外访问地址 <span class="text-muted-foreground">(可选)</span></label>
                <input v-model="form.public_base_url" type="text" placeholder="http://example.com:3100" :class="inputClass" />
                <p class="text-xs text-muted-foreground">反代或外网访问地址，不填则使用本机地址</p>
              </div>
            </fieldset>

            <fieldset class="space-y-3">
              <legend class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">解析范围</legend>
              <div class="rounded-lg border border-border/60 bg-background p-4 space-y-3">
                <div>
                  <div class="text-sm font-medium">常用预设</div>
                  <p class="mt-1 text-xs text-muted-foreground">一键切换常见解析组合。预设只改解析类型和系统库策略，不改手动 DB Nums。</p>
                </div>
                <div class="grid gap-2">
                  <button
                    v-for="preset in PARSE_PRESET_OPTIONS"
                    :key="preset.key"
                    type="button"
                    class="rounded-lg border px-3 py-2 text-left transition-colors"
                    :class="activePresetKey === preset.key
                      ? 'border-primary bg-primary/5 text-primary'
                      : 'border-border/60 bg-background hover:border-border'"
                    @click="applyParsePreset(preset.key)"
                  >
                    <div class="text-sm font-medium">{{ preset.label }}</div>
                    <div class="mt-1 text-xs text-muted-foreground">{{ preset.detail }}</div>
                  </button>
                </div>
                <p class="text-xs text-muted-foreground">
                  当前{{ activePresetKey ? '已匹配预设，会跟随预设更新。' : '为自定义组合，可以继续手动微调。' }}
                </p>
              </div>
              <div class="rounded-lg border border-border/60 bg-background p-4 space-y-3">
                <div class="flex items-start justify-between gap-3">
                  <div>
                    <div class="text-sm font-medium">本次解析预览</div>
                    <p class="mt-1 text-xs text-muted-foreground">保存前直接查看这次预计会解析哪些 db 文件。</p>
                  </div>
                  <button
                    type="button"
                    class="inline-flex h-8 items-center rounded-md border border-input bg-transparent px-3 text-xs font-medium hover:bg-accent transition-colors"
                    @click="refreshPreview"
                  >
                    刷新
                  </button>
                </div>
                <p v-if="!previewPayload" class="text-xs text-muted-foreground">
                  填写项目名称、项目路径和 Web 端口后，自动显示预览结果。
                </p>
                <p v-else-if="previewLoading" class="text-xs text-muted-foreground">
                  正在计算解析文件…
                </p>
                <div v-else-if="previewError" class="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-xs text-destructive">
                  {{ previewError }}
                </div>
                <template v-else-if="previewPlan">
                  <div class="flex flex-wrap items-center gap-2">
                    <span
                      class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
                      :class="parsePlanClass(previewPlan)"
                    >
                      {{ previewPlan.label }}
                    </span>
                    <span class="text-xs text-muted-foreground">
                      {{ previewPlan.includes_system_db_files ? '包含系统数据' : '复用已解析系统库' }}
                    </span>
                  </div>
                  <p class="text-xs text-muted-foreground">{{ previewPlan.detail }}</p>
                  <div class="rounded-md border border-border/60 bg-muted/20 p-3">
                    <div class="text-xs text-muted-foreground">预计解析文件</div>
                    <div v-if="previewPlan.included_db_files.length" class="mt-2">
                      <div class="mb-2 text-xs text-muted-foreground">共 {{ previewPlan.included_db_files.length }} 个文件</div>
                      <div class="flex max-h-32 flex-wrap gap-2 overflow-auto">
                        <span
                          v-for="file in previewPlan.included_db_files"
                          :key="file"
                          class="inline-flex items-center rounded-full border border-border px-2 py-0.5 text-xs"
                        >
                          {{ file }}
                        </span>
                      </div>
                    </div>
                    <p v-else class="mt-2 text-xs text-muted-foreground">
                      当前没有限制具体文件，解析时会按项目配置做全量解析。
                    </p>
                  </div>
                </template>
              </div>
              <div class="rounded-lg border border-border/60 bg-background p-4 space-y-3">
                <div>
                  <div class="text-sm font-medium">模型数据</div>
                  <p class="mt-1 text-xs text-muted-foreground">用来控制本次要不要解析设计模型。默认保留 DESI。</p>
                </div>
                <div class="space-y-2">
                  <label class="text-sm font-medium">手动 DB Nums <span class="text-muted-foreground">(可选，逗号分隔)</span></label>
                  <input v-model="manualDbNumsStr" type="text" placeholder="7997, 7998, 7999" :class="inputClass" />
                  <p class="text-xs text-muted-foreground">留空表示解析并生成全部设计库；填写后才限制到指定 dbnum。</p>
                </div>
                <div class="grid gap-2">
                  <label
                    v-for="option in MODEL_PARSE_DB_TYPE_OPTIONS"
                    :key="option.value"
                    class="flex items-start gap-3 rounded-lg border border-border/60 bg-background px-3 py-2 cursor-pointer transition-colors hover:border-border"
                  >
                    <input
                      :checked="hasParseDbType(option.value)"
                      type="checkbox"
                      class="mt-0.5 h-4 w-4 rounded border-input"
                      @change="toggleParseDbType(option.value)"
                    />
                    <span class="min-w-0">
                      <span class="block text-sm font-medium">{{ option.label }}</span>
                      <span class="block text-xs text-muted-foreground">{{ option.detail }}</span>
                    </span>
                  </label>
                </div>
              </div>
              <div class="rounded-lg border border-border/60 bg-background p-4 space-y-3">
                <div>
                  <div class="text-sm font-medium">系统数据策略</div>
                  <p class="mt-1 text-xs text-muted-foreground">系统、字典、元件等基础库放在一起配置，避免和设计模型混在一处。</p>
                </div>
                <div class="grid gap-2">
                  <label
                    v-for="option in SYSTEM_PARSE_DB_TYPE_OPTIONS"
                    :key="option.value"
                    class="flex items-start gap-3 rounded-lg border border-border/60 bg-background px-3 py-2 cursor-pointer transition-colors hover:border-border"
                  >
                    <input
                      :checked="hasParseDbType(option.value)"
                      type="checkbox"
                      class="mt-0.5 h-4 w-4 rounded border-input"
                      @change="toggleParseDbType(option.value)"
                    />
                    <span class="min-w-0">
                      <span class="block text-sm font-medium">{{ option.label }}</span>
                      <span class="block text-xs text-muted-foreground">{{ option.detail }}</span>
                    </span>
                  </label>
                </div>
                <label
                  class="flex items-start gap-3 rounded-lg border border-border/60 bg-background px-3 py-2"
                  :class="canForceRebuildSystemDb ? 'cursor-pointer hover:border-border' : 'opacity-60'"
                >
                  <input
                    v-model="form.force_rebuild_system_db"
                    type="checkbox"
                    class="mt-0.5 h-4 w-4 rounded border-input"
                    :disabled="!canForceRebuildSystemDb"
                  />
                  <span class="min-w-0">
                    <span class="block text-sm font-medium">强制重建系统库</span>
                    <span class="block text-xs text-muted-foreground">
                      开启后，即使站点已经解析过，下一次解析也会重新读取 SYST。关闭时会优先复用已解析系统库。
                    </span>
                  </span>
                </label>
                <p class="text-xs text-muted-foreground">
                  默认完整部署：SYST + DESI + CATA + DICT + GLB + GLOB，用于同时补齐属性定义和元件库数据。
                </p>
                <p class="text-xs text-muted-foreground">
                  如果清空所有勾选，且不填写手动 DB Nums，就会退回按项目配置全量解析。
                </p>
              </div>
            </fieldset>

            <fieldset class="space-y-3">
              <legend class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">模型生成</legend>
              <div class="rounded-lg border border-border/60 bg-background p-4 space-y-3">
                <div>
                  <div class="text-sm font-medium">生成开关</div>
                  <p class="mt-1 text-xs text-muted-foreground">
                    保存后会写入该部署项目的 DbOption.toml，解析完成后启动站点/完整生成时使用这些配置。
                  </p>
                </div>
                <div class="grid gap-2">
                  <label class="flex items-start gap-3 rounded-lg border border-border/60 bg-background px-3 py-2 cursor-pointer transition-colors hover:border-border">
                    <input v-model="form.gen_model" type="checkbox" class="mt-0.5 h-4 w-4 rounded border-input" />
                    <span class="min-w-0">
                      <span class="block text-sm font-medium">生成模型数据</span>
                      <span class="block text-xs text-muted-foreground">控制 gen_model，关闭后只保留解析/数据准备流程。</span>
                    </span>
                  </label>
                  <label class="flex items-start gap-3 rounded-lg border border-border/60 bg-background px-3 py-2 cursor-pointer transition-colors hover:border-border">
                    <input v-model="form.gen_mesh" type="checkbox" class="mt-0.5 h-4 w-4 rounded border-input" />
                    <span class="min-w-0">
                      <span class="block text-sm font-medium">生成 Mesh</span>
                      <span class="block text-xs text-muted-foreground">控制 gen_mesh，开启后会生成网格/模型文件，耗时和磁盘占用更高。</span>
                    </span>
                  </label>
                  <label class="flex items-start gap-3 rounded-lg border border-border/60 bg-background px-3 py-2 cursor-pointer transition-colors hover:border-border">
                    <input v-model="form.gen_spatial_tree" type="checkbox" class="mt-0.5 h-4 w-4 rounded border-input" />
                    <span class="min-w-0">
                      <span class="block text-sm font-medium">生成空间树</span>
                      <span class="block text-xs text-muted-foreground">控制 gen_spatial_tree，用于空间查询、房间树和 Viewer 加载。</span>
                    </span>
                  </label>
                  <label class="flex items-start gap-3 rounded-lg border border-border/60 bg-background px-3 py-2 cursor-pointer transition-colors hover:border-border">
                    <input v-model="form.apply_boolean_operation" type="checkbox" class="mt-0.5 h-4 w-4 rounded border-input" />
                    <span class="min-w-0">
                      <span class="block text-sm font-medium">应用布尔运算</span>
                      <span class="block text-xs text-muted-foreground">控制 apply_boolean_operation，精度更高但生成耗时更长。</span>
                    </span>
                  </label>
                </div>
                <div class="grid grid-cols-2 gap-4">
                  <div class="space-y-2">
                    <label class="text-sm font-medium">Mesh 容差比</label>
                    <input
                      v-model.number="form.mesh_tol_ratio"
                      type="number"
                      min="0.1"
                      step="0.1"
                      :class="inputClass"
                    />
                  </div>
                  <div class="space-y-2">
                    <label class="text-sm font-medium">导出格式</label>
                    <div class="flex h-9 items-center gap-4 text-sm">
                      <label class="flex items-center gap-2">
                        <input v-model="form.export_json" type="checkbox" class="h-4 w-4 rounded border-input" />
                        JSON
                      </label>
                      <label class="flex items-center gap-2">
                        <input v-model="form.export_parquet" type="checkbox" class="h-4 w-4 rounded border-input" />
                        Parquet
                      </label>
                    </div>
                  </div>
                </div>
              </div>
            </fieldset>

            <fieldset class="space-y-3">
              <legend class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">数据库凭据</legend>
              <div class="grid grid-cols-2 gap-4">
                <div class="space-y-2">
                  <label class="text-sm font-medium">DB 用户名{{ isEditing ? '（可选）' : ' *' }}</label>
                  <input
                    v-model="form.db_user"
                    type="text"
                    :placeholder="isEditing ? '留空则保留当前用户名' : '请输入数据库用户名'"
                    :class="inputClass"
                  />
                </div>
                <div class="space-y-2">
                  <label class="text-sm font-medium">DB 密码{{ isEditing ? '（可选）' : ' *' }}</label>
                  <input
                    v-model="form.db_password"
                    type="password"
                    :placeholder="isEditing ? '留空则保留当前密码' : '请输入数据库密码'"
                    :class="inputClass"
                  />
                </div>
              </div>
              <p class="text-xs text-muted-foreground">
                {{ isEditing ? '编辑时留空表示沿用当前凭据。' : '不再自动写入默认 root/root，请显式填写。' }}
              </p>
              <div
                v-if="weakCredentialsWarning"
                class="rounded-md border border-amber-300 bg-amber-50 px-2 py-1.5 text-xs text-amber-800 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-200"
              >
                {{ weakCredentialsWarning }}
              </div>
            </fieldset>

            <div v-if="error" class="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive">
              {{ error }}
            </div>
          </form>

          <!-- Footer -->
          <div class="border-t border-border px-6 py-4 flex justify-end gap-3">
            <button @click="emit('close')"
              class="inline-flex h-9 items-center rounded-md border border-input bg-transparent px-4 text-sm font-medium shadow-sm hover:bg-accent transition-colors">
              取消
            </button>
            <button
              @click="handleSubmit(false)"
              :disabled="saving || !canSubmit"
              class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow hover:bg-primary/90 transition-colors disabled:pointer-events-none disabled:opacity-50">
              {{ saving ? '保存中...' : '保存' }}
            </button>
            <button
              v-if="!isEditing"
              @click="handleSubmit(true)"
              :disabled="saving || !canSubmit"
              class="inline-flex h-9 items-center rounded-md bg-emerald-600 px-4 text-sm font-medium text-white shadow hover:bg-emerald-700 transition-colors disabled:pointer-events-none disabled:opacity-50">
              {{ saving ? '提交中...' : '保存并一键部署' }}
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.drawer-enter-active,
.drawer-leave-active {
  transition: all 0.3s ease;
}
.drawer-enter-active > div:first-child,
.drawer-leave-active > div:first-child {
  transition: opacity 0.3s;
}
.drawer-enter-active > div:last-child,
.drawer-leave-active > div:last-child {
  transition: transform 0.3s ease;
}
.drawer-enter-from > div:first-child,
.drawer-leave-to > div:first-child {
  opacity: 0;
}
.drawer-enter-from > div:last-child,
.drawer-leave-to > div:last-child {
  transform: translateX(100%);
}
</style>
