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
import { X } from 'lucide-vue-next'

const props = defineProps<{
  open: boolean
  siteId: string | null
}>()

const emit = defineEmits<{
  close: []
  saved: []
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

const form = ref<CreateManagedSiteRequest>({
  project_name: '',
  project_path: '',
  project_code: 0,
  manual_db_nums: [],
  parse_db_types: [...DEFAULT_PARSE_DB_TYPES],
  force_rebuild_system_db: false,
  db_port: 8020,
  web_port: 8080,
  bind_host: '127.0.0.1',
  public_base_url: '',
  associated_project: '',
  db_user: '',
  db_password: '',
})

const manualDbNumsStr = ref('')

const isEditing = computed(() => !!props.siteId)
const title = computed(() => isEditing.value ? '编辑站点' : '新建站点')

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
  if (siteId) {
    try {
      existingSite.value = await sitesApi.get(siteId)
      const s = existingSite.value
      form.value = {
        project_name: s.project_name,
        project_path: s.project_path,
        project_code: s.project_code,
        manual_db_nums: s.manual_db_nums,
        parse_db_types: s.parse_db_types?.length ? [...s.parse_db_types] : [...DEFAULT_PARSE_DB_TYPES],
        force_rebuild_system_db: s.force_rebuild_system_db ?? false,
        db_port: s.db_port,
        web_port: s.web_port,
        bind_host: s.bind_host || '127.0.0.1',
        public_base_url: s.public_base_url || '',
        associated_project: s.associated_project || '',
        db_user: '',
        db_password: '',
      }
      manualDbNumsStr.value = s.manual_db_nums.join(', ')
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
      db_port: 8020,
      web_port: 8080,
      bind_host: '127.0.0.1',
      public_base_url: '',
      associated_project: '',
      db_user: '',
      db_password: '',
    }
    manualDbNumsStr.value = ''
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
  if (!props.open || !projectName || !projectPath || !form.value.web_port) {
    return null
  }
  const parseDbTypes = normalizeParseDbTypes(form.value.parse_db_types ?? [])
  return {
    site_id: props.siteId ?? undefined,
    project_name: projectName,
    project_path: projectPath,
    manual_db_nums: parseManualDbNumsInput(manualDbNumsStr.value),
    parse_db_types: parseDbTypes,
    force_rebuild_system_db: parseDbTypes.includes('SYST') ? !!form.value.force_rebuild_system_db : false,
    web_port: form.value.web_port,
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

async function handleSubmit() {
  saving.value = true
  error.value = ''
  parseDbNums()
  form.value.parse_db_types = normalizeParseDbTypes(form.value.parse_db_types ?? [])
  if (!form.value.parse_db_types.includes('SYST')) {
    form.value.force_rebuild_system_db = false
  }
  try {
    if (isEditing.value && props.siteId) {
      const payload: UpdateManagedSiteRequest = {
        ...form.value,
        db_user: form.value.db_user?.trim() ? form.value.db_user.trim() : undefined,
        db_password: form.value.db_password?.trim() ? form.value.db_password.trim() : undefined,
      }
      await sitesStore.updateSite(props.siteId, payload)
    } else {
      await sitesStore.createSite({
        ...form.value,
        db_user: form.value.db_user?.trim() || '',
        db_password: form.value.db_password?.trim() || '',
      })
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
          <form class="flex-1 overflow-auto px-6 py-4 space-y-6" @submit.prevent="handleSubmit">
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
                <label class="text-sm font-medium">关联工程 <span class="text-muted-foreground">(可选)</span></label>
                <input v-model="form.associated_project" type="text" :placeholder="form.project_name || '默认使用项目名称'" :class="inputClass" />
                <p class="text-xs text-muted-foreground">打开 Viewer 时自动切换到的工程名</p>
              </div>
            </fieldset>

            <fieldset class="space-y-3">
              <legend class="text-xs font-semibold uppercase tracking-wider text-muted-foreground">运行配置</legend>
              <div class="grid grid-cols-2 gap-4">
                <div class="space-y-2">
                  <label class="text-sm font-medium">DB 端口 *</label>
                  <input v-model.number="form.db_port" type="number" required min="1" max="65535" :class="inputClass" />
                </div>
                <div class="space-y-2">
                  <label class="text-sm font-medium">Web 端口 *</label>
                  <input v-model.number="form.web_port" type="number" required min="1" max="65535" :class="inputClass" />
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
                  <p class="text-xs text-muted-foreground">填写后会优先按指定 dbnum 解析目标设计库。</p>
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
                  推荐快速部署：SYST + DESI。需要补属性定义时加 DICT；需要补元件规格时加 CATA。
                </p>
                <p class="text-xs text-muted-foreground">
                  如果清空所有勾选，且不填写手动 DB Nums，就会退回按项目配置全量解析。
                </p>
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
              @click="handleSubmit"
              :disabled="saving
                || !form.project_name
                || !form.project_path
                || (!isEditing && (!form.db_user?.trim() || !form.db_password?.trim()))"
              class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow hover:bg-primary/90 transition-colors disabled:pointer-events-none disabled:opacity-50">
              {{ saving ? '保存中...' : '保存' }}
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
