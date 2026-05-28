<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import {
  CheckCircle2,
  Circle,
  Loader2,
  PackageOpen,
  Rocket,
  Server,
  ShieldCheck,
  XCircle,
} from 'lucide-vue-next'
import { extractErrorMessage } from '@/api/client'
import { sitesApi } from '@/api/sites'
import { tasksApi } from '@/api/tasks'
import { usePolling } from '@/composables/usePolling'
import type {
  ManagedProjectSite,
  ManagedRemoteDeployStatus,
  ManagedRemoteTargetOs,
  ManagedRemoteTargetRequest,
  ManagedSitePreflightCheck,
} from '@/types/site'
import type { TaskInfo } from '@/types/task'

const sites = ref<ManagedProjectSite[]>([])
const selectedSiteId = ref('')
const remoteStatus = ref<ManagedRemoteDeployStatus | null>(null)
const remoteAgentStatus = ref<Record<string, unknown> | null>(null)
const deployTask = ref<TaskInfo | null>(null)
const deployTaskId = ref('')
const loading = ref(false)
const preflightLoading = ref(false)
const prepareLoading = ref(false)
const deployLoading = ref(false)
const agentStatusLoading = ref(false)
const error = ref('')

const remoteTargetForm = ref<ManagedRemoteTargetRequest>({
  id: 'offline-default',
  name: '离线部署 Ubuntu22 目标',
  target_os: 'ubuntu22',
  host: '123.57.182.243',
  ssh_port: 22,
  ssh_user: 'root',
  password_env: 'REMOTE_PASS',
  ssh_password: '',
  remote_root: '/opt/plant3d/sites',
  remote_db_path: '',
  remote_web_port: 3100,
  remote_db_port: 8020,
  public_base_url: '',
  surreal_bin: '/usr/local/bin/surreal',
  remote_web_bin: '/root/web_server',
  auto_prepare: true,
  upload_web_server: true,
  upload_surreal: true,
  upload_resource: true,
  upload_viewer: true,
  open_firewall: true,
  allowed_cidrs: ['0.0.0.0/0'],
  web_bind_host: '0.0.0.0',
  db_bind_host: '127.0.0.1',
  local_web_bin: '',
  local_surreal_bin: '',
  local_resource_dir: '',
  local_viewer_dir: '',
})

const selectedSite = computed(() => sites.value.find((site) => site.site_id === selectedSiteId.value) ?? null)
const busy = computed(() => loading.value || preflightLoading.value || prepareLoading.value || deployLoading.value)
const blockingCount = computed(() => remoteStatus.value?.checks.filter((check) => check.status === 'blocking').length ?? 0)
const warningCount = computed(() => remoteStatus.value?.checks.filter((check) => check.status === 'warning').length ?? 0)
const remoteAgentStatusText = computed(() => remoteAgentStatus.value ? JSON.stringify(remoteAgentStatus.value, null, 2) : '')
const remoteAllowedCidrsText = computed({
  get: () => remoteTargetForm.value.allowed_cidrs?.join(', ') || '0.0.0.0/0',
  set: (value: string) => {
    remoteTargetForm.value.allowed_cidrs = value
      .split(',')
      .map((item) => item.trim())
      .filter(Boolean)
  },
})

const osOptions: Array<{ value: ManagedRemoteTargetOs; label: string; note: string }> = [
  { value: 'ubuntu22', label: 'Ubuntu 22', note: 'systemd + ufw' },
  { value: 'centos79', label: 'CentOS 7.9', note: 'systemd + firewalld' },
  { value: 'windows', label: 'Windows', note: 'OpenSSH + PowerShell 适配中' },
]

const deploySteps = computed(() => {
  const step = remoteStatus.value?.current_step ?? ''
  const status = remoteStatus.value?.status ?? 'idle'
  return [
    { key: 'preflight', label: '预检', done: ['ready', 'prepared', 'running', 'completed'].includes(status) || !!remoteStatus.value, active: step.includes('preflight') },
    { key: 'remote_prepare', label: '远端准备', done: ['prepared', 'running', 'completed'].includes(status), active: step.includes('prepare') },
    { key: 'upload', label: '上传数据/产物', done: ['remote_config', 'remote_start', 'validation', '远端部署完成'].includes(step) || status === 'completed', active: step.includes('upload') },
    { key: 'remote_config', label: '写入配置', done: ['remote_start', 'validation', '远端部署完成'].includes(step) || status === 'completed', active: step.includes('config') },
    { key: 'remote_start', label: '启动服务', done: ['validation', '远端部署完成'].includes(step) || status === 'completed', active: step.includes('start') },
    { key: 'validation', label: '验收', done: status === 'completed', active: step.includes('validation') },
  ]
})

function applyOsDefaults(os: ManagedRemoteTargetOs) {
  const siteId = selectedSiteId.value || 'site'
  if (os === 'windows') {
    remoteTargetForm.value.name = '离线部署 Windows 目标'
    remoteTargetForm.value.remote_root = 'C:/Plant3D/sites'
    remoteTargetForm.value.remote_db_path = `C:/Plant3D/runtime/surrealdb/${siteId}.db`
    remoteTargetForm.value.surreal_bin = 'C:/Plant3D/bin/surreal/surreal.exe'
    remoteTargetForm.value.remote_web_bin = 'C:/Plant3D/bin/web_server.exe'
    remoteTargetForm.value.db_bind_host = '127.0.0.1'
    remoteTargetForm.value.web_bind_host = '0.0.0.0'
    return
  }
  remoteTargetForm.value.name = os === 'centos79' ? '离线部署 CentOS 7.9 目标' : '离线部署 Ubuntu22 目标'
  remoteTargetForm.value.remote_root = '/opt/plant3d/sites'
  remoteTargetForm.value.remote_db_path = `/root/surreal_data/${siteId}.db`
  remoteTargetForm.value.surreal_bin = '/usr/local/bin/surreal'
  remoteTargetForm.value.remote_web_bin = '/root/web_server'
  remoteTargetForm.value.db_bind_host = '127.0.0.1'
  remoteTargetForm.value.web_bind_host = '0.0.0.0'
}

watch(() => remoteTargetForm.value.target_os, (next) => applyOsDefaults(next || 'ubuntu22'))
watch(selectedSiteId, () => {
  applyOsDefaults(remoteTargetForm.value.target_os || 'ubuntu22')
  remoteStatus.value = null
  remoteAgentStatus.value = null
  deployTask.value = null
  deployTaskId.value = ''
})

async function loadSites() {
  loading.value = true
  error.value = ''
  try {
    sites.value = await sitesApi.list()
    if (!selectedSiteId.value && sites.value.length > 0) {
      selectedSiteId.value = sites.value[0].site_id
    }
  } catch (err: unknown) {
    error.value = extractErrorMessage(err)
  } finally {
    loading.value = false
  }
}

async function refreshRemoteStatus() {
  if (!selectedSiteId.value) return
  try {
    remoteStatus.value = await sitesApi.remoteDeployStatus(selectedSiteId.value)
  } catch {
    // Keep the last visible deployment state; explicit button actions surface errors.
  }
}

async function refreshRemoteAgentStatus(surfaceError = true) {
  if (!selectedSiteId.value || !remoteStatus.value?.remote_entry_url) return
  agentStatusLoading.value = true
  try {
    remoteAgentStatus.value = await sitesApi.remoteAgentStatus(selectedSiteId.value)
    if (surfaceError) error.value = ''
  } catch (err: unknown) {
    if (surfaceError) error.value = extractErrorMessage(err)
  } finally {
    agentStatusLoading.value = false
  }
}

async function refreshDeployTask() {
  if (!deployTaskId.value) return
  try {
    deployTask.value = await tasksApi.get(deployTaskId.value)
    await refreshRemoteStatus()
  } catch {
    // Polling is best-effort here; the main action still reports failures.
  }
}

async function runPreflight() {
  if (!selectedSiteId.value) return
  preflightLoading.value = true
  error.value = ''
  try {
    remoteStatus.value = await sitesApi.remotePreflight(selectedSiteId.value, { target: remoteTargetForm.value })
  } catch (err: unknown) {
    error.value = extractErrorMessage(err)
  } finally {
    preflightLoading.value = false
  }
}

async function prepareRemote() {
  if (!selectedSiteId.value) return
  prepareLoading.value = true
  error.value = ''
  try {
    remoteStatus.value = await sitesApi.remotePrepare(selectedSiteId.value, { target: remoteTargetForm.value })
  } catch (err: unknown) {
    error.value = extractErrorMessage(err)
  } finally {
    prepareLoading.value = false
  }
}

async function deployRemote() {
  if (!selectedSiteId.value) return
  deployLoading.value = true
  error.value = ''
  try {
    remoteStatus.value = await sitesApi.remotePreflight(selectedSiteId.value, { target: remoteTargetForm.value })
    if (remoteStatus.value.status === 'blocked') return
    const submitted = await sitesApi.remoteDeploy(selectedSiteId.value, { target: remoteTargetForm.value })
    const taskId = submitted.data?.task_id
    if (taskId) {
      deployTaskId.value = String(taskId)
      await refreshDeployTask()
    }
    await refreshRemoteStatus()
  } catch (err: unknown) {
    error.value = extractErrorMessage(err)
  } finally {
    deployLoading.value = false
  }
}

function checkClass(check: ManagedSitePreflightCheck) {
  if (check.status === 'blocking') return 'border-destructive/40 bg-destructive/5 text-destructive'
  if (check.status === 'warning') return 'border-amber-300 bg-amber-50 text-amber-800 dark:bg-amber-950 dark:text-amber-200'
  return 'border-emerald-300 bg-emerald-50 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-200'
}

const { start: startPolling } = usePolling(async () => {
  await refreshDeployTask()
  await refreshRemoteStatus()
  await refreshRemoteAgentStatus(false)
}, 2500)

onMounted(async () => {
  await loadSites()
  await refreshRemoteStatus()
  startPolling()
})
</script>

<template>
  <div class="space-y-6">
    <section class="overflow-hidden rounded-2xl border border-border bg-card">
      <div class="relative p-6 md:p-8">
        <div class="absolute inset-y-0 right-0 hidden w-1/3 bg-gradient-to-l from-primary/10 to-transparent md:block" />
        <div class="relative max-w-3xl">
          <div class="mb-4 inline-flex items-center gap-2 rounded-full border border-primary/20 bg-primary/10 px-3 py-1 text-xs font-medium text-primary">
            <PackageOpen class="h-3.5 w-3.5" />
            离线安装部署向导
          </div>
          <h1 class="text-2xl font-semibold tracking-tight">一键推送数据库、SurrealDB、web_server 和 plant3d-web</h1>
          <p class="mt-3 text-sm leading-6 text-muted-foreground">
            这里就是独立入口：选择本机已解析/生成的站点，填写目标服务器 SSH 账号密码和操作系统，直接完成预检、上传、远端配置、启动和验收，不需要再进入站点详情页操作。
          </p>
        </div>
      </div>
    </section>

    <section class="grid gap-4 md:grid-cols-3">
      <div class="rounded-xl border border-border bg-card p-5">
        <Server class="mb-3 h-5 w-5 text-primary" />
        <div class="text-sm font-medium">站点数据随本机状态走</div>
        <div class="mt-1 text-xs text-muted-foreground">复制当前站点 RocksDB/输出结果，避免远端重新解析。</div>
      </div>
      <div class="rounded-xl border border-border bg-card p-5">
        <ShieldCheck class="mb-3 h-5 w-5 text-emerald-600" />
        <div class="text-sm font-medium">SSH 密码可保存到本机 SQLite</div>
        <div class="mt-1 text-xs text-muted-foreground">测试阶段允许保存密码，便于异步任务和重复部署复用。</div>
      </div>
      <div class="rounded-xl border border-border bg-card p-5">
        <Rocket class="mb-3 h-5 w-5 text-amber-600" />
        <div class="text-sm font-medium">带进度的远端执行</div>
        <div class="mt-1 text-xs text-muted-foreground">后端持续记录当前步骤，页面自动轮询展示部署进度。</div>
      </div>
    </section>

    <section class="rounded-xl border border-border bg-card p-5">
      <div class="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 class="text-base font-medium">1. 选择本机站点</h2>
          <p class="mt-1 text-sm text-muted-foreground">请先在站点管理中完成本机解析/生成；运行中的站点预检会阻断复制。</p>
        </div>
        <button
          class="inline-flex h-9 items-center gap-2 rounded-md border border-input bg-transparent px-3 text-sm font-medium hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
          :disabled="loading"
          @click="loadSites"
        >
          <Loader2 v-if="loading" class="h-4 w-4 animate-spin" />
          刷新站点
        </button>
      </div>

      <div v-if="error" class="mb-4 rounded-lg border border-destructive/50 bg-destructive/5 px-4 py-3 text-sm text-destructive">
        {{ error }}
      </div>

      <label class="block space-y-1 text-xs text-muted-foreground">
        <span>本机站点</span>
        <select v-model="selectedSiteId" class="h-10 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground">
          <option disabled value="">请选择站点</option>
          <option v-for="item in sites" :key="item.site_id" :value="item.site_id">
            {{ item.project_name }} · {{ item.site_id }} · {{ item.status }}/{{ item.parse_status }}
          </option>
        </select>
      </label>

      <div v-if="selectedSite" class="mt-4 rounded-lg border border-border bg-background/60 p-4 text-xs text-muted-foreground">
        <div class="grid gap-2 md:grid-cols-2">
          <div>数据库目录：<span class="break-all text-foreground">{{ selectedSite.db_data_path }}</span></div>
          <div>配置文件：<span class="break-all text-foreground">{{ selectedSite.config_path }}</span></div>
          <div>本机 Web 端口：<span class="text-foreground">{{ selectedSite.web_port }}</span></div>
          <div>本机 DB 端口：<span class="text-foreground">{{ selectedSite.db_port }}</span></div>
        </div>
      </div>
    </section>

    <section class="rounded-xl border border-border bg-card p-5">
      <div class="mb-4">
        <h2 class="text-base font-medium">2. 目标服务器</h2>
        <p class="mt-1 text-sm text-muted-foreground">选择 Windows / Ubuntu 22 / CentOS 7.9，填写 SSH、远端目录、端口和要随包上传的本地产物。</p>
      </div>

      <div class="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
        <label class="space-y-1 text-xs text-muted-foreground">
          <span>操作系统</span>
          <select v-model="remoteTargetForm.target_os" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground">
            <option v-for="option in osOptions" :key="option.value" :value="option.value">
              {{ option.label }} · {{ option.note }}
            </option>
          </select>
        </label>
        <label class="space-y-1 text-xs text-muted-foreground">
          <span>主机</span>
          <input v-model="remoteTargetForm.host" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
        </label>
        <label class="space-y-1 text-xs text-muted-foreground">
          <span>SSH 用户</span>
          <input v-model="remoteTargetForm.ssh_user" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
        </label>
        <label class="space-y-1 text-xs text-muted-foreground">
          <span>SSH 端口</span>
          <input v-model.number="remoteTargetForm.ssh_port" type="number" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
        </label>
        <label class="space-y-1 text-xs text-muted-foreground">
          <span>SSH 密码（测试阶段可落库）</span>
          <input v-model="remoteTargetForm.ssh_password" type="password" autocomplete="new-password" placeholder="保存到本机部署目标配置" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
        </label>
        <label class="space-y-1 text-xs text-muted-foreground">
          <span>Web 入口 URL</span>
          <input v-model="remoteTargetForm.public_base_url" placeholder="留空则使用 http://主机:Web端口" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
        </label>
        <label class="space-y-1 text-xs text-muted-foreground">
          <span>DB 端口</span>
          <input v-model.number="remoteTargetForm.remote_db_port" type="number" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
        </label>
        <label class="space-y-1 text-xs text-muted-foreground">
          <span>Web 端口</span>
          <input v-model.number="remoteTargetForm.remote_web_port" type="number" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
        </label>
        <label class="space-y-1 text-xs text-muted-foreground">
          <span>远端根目录</span>
          <input v-model="remoteTargetForm.remote_root" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
        </label>
        <label class="space-y-1 text-xs text-muted-foreground">
          <span>远端 DB 路径</span>
          <input v-model="remoteTargetForm.remote_db_path" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
        </label>
        <label class="space-y-1 text-xs text-muted-foreground">
          <span>Surreal 路径</span>
          <input v-model="remoteTargetForm.surreal_bin" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
        </label>
        <label class="space-y-1 text-xs text-muted-foreground">
          <span>Web Server 路径</span>
          <input v-model="remoteTargetForm.remote_web_bin" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
        </label>
        <label class="space-y-1 text-xs text-muted-foreground">
          <span>Web 绑定地址</span>
          <input v-model="remoteTargetForm.web_bind_host" placeholder="0.0.0.0" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
        </label>
        <label class="space-y-1 text-xs text-muted-foreground">
          <span>DB 绑定地址</span>
          <input v-model="remoteTargetForm.db_bind_host" placeholder="127.0.0.1" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
        </label>
        <label class="space-y-1 text-xs text-muted-foreground md:col-span-2">
          <span>放行来源 CIDR</span>
          <input v-model="remoteAllowedCidrsText" placeholder="0.0.0.0/0, 你的办公网段/32" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
        </label>
      </div>

      <div class="mt-4 rounded-lg border border-border bg-background/60 p-4">
        <div class="mb-3 flex flex-wrap items-center gap-3 text-xs text-muted-foreground">
          <label class="inline-flex items-center gap-2">
            <input v-model="remoteTargetForm.auto_prepare" type="checkbox" class="h-4 w-4 rounded border-input" />
            <span>部署前自动准备远端</span>
          </label>
          <label class="inline-flex items-center gap-2">
            <input v-model="remoteTargetForm.open_firewall" type="checkbox" class="h-4 w-4 rounded border-input" />
            <span>自动配置防火墙</span>
          </label>
          <label class="inline-flex items-center gap-2">
            <input v-model="remoteTargetForm.upload_web_server" type="checkbox" class="h-4 w-4 rounded border-input" />
            <span>上传 web_server</span>
          </label>
          <label class="inline-flex items-center gap-2">
            <input v-model="remoteTargetForm.upload_surreal" type="checkbox" class="h-4 w-4 rounded border-input" />
            <span>上传 SurrealDB</span>
          </label>
          <label class="inline-flex items-center gap-2">
            <input v-model="remoteTargetForm.upload_resource" type="checkbox" class="h-4 w-4 rounded border-input" />
            <span>上传 resource/surreal</span>
          </label>
          <label class="inline-flex items-center gap-2">
            <input v-model="remoteTargetForm.upload_viewer" type="checkbox" class="h-4 w-4 rounded border-input" />
            <span>上传 plant3d-web viewer</span>
          </label>
        </div>

        <div class="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
          <label class="space-y-1 text-xs text-muted-foreground">
            <span>本地 web_server</span>
            <input v-model="remoteTargetForm.local_web_bin" placeholder="留空自动查找 release 产物" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
          </label>
          <label class="space-y-1 text-xs text-muted-foreground">
            <span>本地 SurrealDB</span>
            <input v-model="remoteTargetForm.local_surreal_bin" placeholder="留空自动查找 tools/surrealdb" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
          </label>
          <label class="space-y-1 text-xs text-muted-foreground">
            <span>本地 resource/surreal</span>
            <input v-model="remoteTargetForm.local_resource_dir" placeholder="留空使用仓库 resource/surreal" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
          </label>
          <label class="space-y-1 text-xs text-muted-foreground">
            <span>本地 viewer</span>
            <input v-model="remoteTargetForm.local_viewer_dir" placeholder="留空使用仓库 viewer" class="h-9 w-full rounded-md border border-input bg-background px-3 text-sm text-foreground" />
          </label>
        </div>
      </div>
    </section>

    <section class="rounded-xl border border-border bg-card p-5">
      <div class="mb-4 flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 class="text-base font-medium">3. 执行安装部署</h2>
          <p class="mt-1 text-sm text-muted-foreground">推荐顺序：远端预检 → 远端准备 → 一键部署。也可以直接点一键部署，后端会先预检。</p>
        </div>
        <div class="flex flex-wrap gap-2">
          <button class="inline-flex h-9 items-center gap-2 rounded-md border border-input bg-transparent px-3 text-sm font-medium hover:bg-accent disabled:pointer-events-none disabled:opacity-50" :disabled="busy || !selectedSiteId" @click="runPreflight">
            <Loader2 v-if="preflightLoading" class="h-4 w-4 animate-spin" />
            远端预检
          </button>
          <button class="inline-flex h-9 items-center gap-2 rounded-md border border-input bg-transparent px-3 text-sm font-medium hover:bg-accent disabled:pointer-events-none disabled:opacity-50" :disabled="busy || !selectedSiteId" @click="prepareRemote">
            <Loader2 v-if="prepareLoading" class="h-4 w-4 animate-spin" />
            远端准备
          </button>
          <button class="inline-flex h-9 items-center gap-2 rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground hover:bg-primary/90 disabled:pointer-events-none disabled:opacity-50" :disabled="busy || !selectedSiteId" @click="deployRemote">
            <Loader2 v-if="deployLoading" class="h-4 w-4 animate-spin" />
            一键部署
          </button>
        </div>
      </div>

      <div class="grid gap-2 md:grid-cols-6">
        <div v-for="step in deploySteps" :key="step.key" class="rounded-lg border border-border bg-background/60 p-3 text-xs">
          <div class="mb-2 flex items-center gap-2">
            <CheckCircle2 v-if="step.done" class="h-4 w-4 text-emerald-600" />
            <Loader2 v-else-if="step.active" class="h-4 w-4 animate-spin text-primary" />
            <Circle v-else class="h-4 w-4 text-muted-foreground" />
            <span class="font-medium">{{ step.label }}</span>
          </div>
          <div class="text-muted-foreground">{{ step.done ? '已完成' : step.active ? '执行中' : '等待' }}</div>
        </div>
      </div>

      <div v-if="deployTask" class="mt-4 rounded-lg border border-border bg-background/60 p-4 text-sm">
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div>
            <div class="font-medium">任务：{{ deployTask.name }} · {{ deployTask.status }}</div>
            <div class="mt-1 text-xs text-muted-foreground">{{ deployTask.progress.current_step }}</div>
          </div>
          <div class="text-sm font-medium">{{ Math.round(deployTask.progress.percentage) }}%</div>
        </div>
        <div class="mt-3 h-2 overflow-hidden rounded-full bg-muted">
          <div class="h-full rounded-full bg-primary transition-all" :style="{ width: `${Math.round(deployTask.progress.percentage)}%` }" />
        </div>
        <div v-if="deployTask.error" class="mt-2 text-xs text-destructive">{{ deployTask.error }}</div>
      </div>

      <div v-if="remoteStatus" class="mt-4 rounded-lg border border-border bg-background/60 p-4 text-sm">
        <div class="flex flex-wrap items-center justify-between gap-3">
          <div>
            <div class="font-medium">状态：{{ remoteStatus.status }} / {{ remoteStatus.current_step }}</div>
            <div class="mt-1 text-xs text-muted-foreground">
              部署ID：{{ remoteStatus.deploy_id || '未生成' }} · 模式：{{ remoteStatus.deployment_mode || '未确定' }} · {{ remoteStatus.degraded ? '降级部署' : '完整部署' }}
            </div>
            <div v-if="remoteStatus.last_error" class="mt-1 text-xs text-destructive break-all">{{ remoteStatus.last_error }}</div>
            <a v-if="remoteStatus.remote_entry_url" :href="remoteStatus.remote_entry_url" target="_blank" class="mt-1 block text-xs text-primary hover:underline break-all">
              {{ remoteStatus.remote_entry_url }}
            </a>
          </div>
          <span
            class="inline-flex items-center rounded-full px-2 py-0.5 text-xs font-medium"
            :class="blockingCount > 0 ? 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200' : warningCount > 0 ? 'bg-amber-100 text-amber-800 dark:bg-amber-900 dark:text-amber-200' : 'bg-emerald-100 text-emerald-800 dark:bg-emerald-900 dark:text-emerald-200'"
          >
            {{ blockingCount }} 阻断 / {{ warningCount }} 警告
          </span>
        </div>

        <div v-if="remoteStatus.checks.length" class="mt-3 grid gap-2 md:grid-cols-2">
          <div v-for="check in remoteStatus.checks" :key="check.key" class="rounded-md border p-3 text-xs" :class="checkClass(check)">
            <div class="flex items-center gap-2 font-medium">
              <XCircle v-if="check.status === 'blocking'" class="h-4 w-4" />
              <CheckCircle2 v-else class="h-4 w-4" />
              {{ check.label }} · {{ check.status }}
            </div>
            <div class="mt-1">{{ check.message }}</div>
            <div v-if="check.detail" class="mt-1 break-all opacity-80">{{ check.detail }}</div>
            <div v-if="check.action_hint" class="mt-1 opacity-80">建议：{{ check.action_hint }}</div>
          </div>
        </div>

        <div class="mt-4 rounded-lg border border-border bg-muted/30 p-3 text-xs">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <div>
              <div class="font-medium text-foreground">远端 Agent 状态</div>
              <div class="mt-1 text-muted-foreground">从远端 `/api/site/agent-status` 拉取，用于部署后监控。</div>
            </div>
            <button
              type="button"
              :disabled="agentStatusLoading || !remoteStatus.remote_entry_url"
              @click="refreshRemoteAgentStatus()"
              class="inline-flex h-8 items-center gap-2 rounded-md border border-input bg-background px-3 font-medium hover:bg-accent transition-colors disabled:pointer-events-none disabled:opacity-50"
            >
              <Loader2 v-if="agentStatusLoading" class="h-3.5 w-3.5 animate-spin" />
              {{ agentStatusLoading ? '拉取中...' : '刷新 Agent' }}
            </button>
          </div>
          <pre v-if="remoteAgentStatus" class="mt-3 max-h-64 overflow-auto rounded-md bg-background p-3 text-[11px] leading-relaxed text-muted-foreground">{{ remoteAgentStatusText }}</pre>
          <div v-else class="mt-3 text-muted-foreground">
            {{ remoteStatus.remote_entry_url ? '尚未拉取远端 Agent 状态。' : '暂无远端访问地址，部署后可拉取 Agent 状态。' }}
          </div>
        </div>
      </div>
    </section>
  </div>
</template>
