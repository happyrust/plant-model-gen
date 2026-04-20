<script setup lang="ts">
import { ref, computed } from 'vue'
import { useRouter } from 'vue-router'
import { extractErrorMessage } from '@/api/client'
import { useTasksStore } from '@/stores/tasks'
import { useSitesStore } from '@/stores/sites'
import type { TaskType, TaskPriority, DatabaseConfig } from '@/types/task'
import { PRIORITY_LABELS, getTaskTypeLabel } from '@/types/task'

const router = useRouter()
const tasksStore = useTasksStore()
const sitesStore = useSitesStore()

const step = ref(1)
const submitting = ref(false)
const submitError = ref('')

const taskName = ref('')
const taskType = ref<TaskType>('ParsePdmsData')
const taskPriority = ref<TaskPriority>('Normal')

const selectedSiteId = ref('')

const genModel = ref(true)
const genMesh = ref(false)
const genSpatialTree = ref(true)
const applyBooleanOp = ref(true)
const meshTolRatio = ref(3.0)
const manualDbNumsStr = ref('')
const manualRefnosStr = ref('')
const exportJson = ref(false)
const exportParquet = ref(true)

if (!sitesStore.sites.length) sitesStore.fetchSites()

const selectedSite = computed(() =>
  sitesStore.sites.find((s) => s.site_id === selectedSiteId.value)
)

const wizardTaskTypes: TaskType[] = [
  'ParsePdmsData',
  'DataGeneration',
  'FullGeneration',
]

const canProceedStepOne = computed(() =>
  !!taskName.value.trim() && !!selectedSiteId.value
)

function nextStep() {
  if (step.value < 3) step.value++
}
function prevStep() {
  if (step.value > 1) step.value--
}

function buildConfig(): Partial<DatabaseConfig> {
  const dbNums = manualDbNumsStr.value
    .split(/[,\s]+/)
    .map(Number)
    .filter((n) => !isNaN(n) && n > 0)

  const refnos = manualRefnosStr.value
    .split(/[,\s]+/)
    .filter(Boolean)

  return {
    manual_db_nums: dbNums,
    manual_refnos: refnos,
    gen_model: genModel.value,
    gen_mesh: genMesh.value,
    gen_spatial_tree: genSpatialTree.value,
    apply_boolean_operation: applyBooleanOp.value,
    mesh_tol_ratio: meshTolRatio.value,
    export_json: exportJson.value,
    export_parquet: exportParquet.value,
  }
}

async function handleSubmit() {
  submitting.value = true
  submitError.value = ''
  try {
    await tasksStore.createTask({
      task_name: taskName.value,
      task_type: taskType.value,
      priority: taskPriority.value,
      site_id: selectedSiteId.value || undefined,
      config_override: buildConfig(),
    })
    router.push({ name: 'tasks' })
  } catch (err: unknown) {
    submitError.value = extractErrorMessage(err)
  } finally {
    submitting.value = false
  }
}

const stepLabels = ['基础信息', '生成配置', '确认创建']

const inputClass = 'flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring'
</script>

<template>
  <div class="mx-auto max-w-2xl space-y-6">
    <div>
      <h2 class="text-2xl font-semibold tracking-tight">创建任务</h2>
      <p class="text-sm text-muted-foreground">按步骤配置任务参数</p>
    </div>

    <div
      v-if="submitError"
      class="rounded-lg border border-destructive/50 bg-destructive/5 px-4 py-3 text-sm text-destructive"
    >
      {{ submitError }}
    </div>

    <!-- Stepper -->
    <div class="flex items-center gap-2">
      <template v-for="(label, idx) in stepLabels" :key="idx">
        <div class="flex items-center gap-2">
          <div class="flex h-8 w-8 items-center justify-center rounded-full text-sm font-medium"
            :class="step >= idx + 1 ? 'bg-primary text-primary-foreground' : 'bg-muted text-muted-foreground'">
            {{ idx + 1 }}
          </div>
          <span class="text-sm" :class="step >= idx + 1 ? 'text-foreground' : 'text-muted-foreground'">{{ label }}</span>
        </div>
        <div v-if="idx < stepLabels.length - 1" class="h-px flex-1 bg-border" />
      </template>
    </div>

    <!-- Step 1 -->
    <div v-if="step === 1" class="space-y-4 rounded-lg border border-border bg-card p-6">
      <div class="space-y-2">
        <label class="text-sm font-medium">任务名称 *</label>
        <input v-model="taskName" type="text" required placeholder="例：全量解析 Site-A" :class="inputClass" />
      </div>

      <div class="space-y-2">
        <label class="text-sm font-medium">关联站点 *</label>
        <select v-model="selectedSiteId" :class="inputClass">
          <option value="" disabled>请选择站点</option>
          <option v-for="s in sitesStore.sites" :key="s.site_id" :value="s.site_id">
            {{ s.project_name }} ({{ s.site_id }})
          </option>
        </select>
        <p class="text-xs text-muted-foreground">admin 任务必须绑定一个已创建站点。</p>
      </div>

      <div class="space-y-2">
        <label class="text-sm font-medium">任务类型</label>
        <div class="grid grid-cols-2 gap-2">
          <label v-for="t in wizardTaskTypes" :key="String(t)"
            class="flex cursor-pointer items-center gap-2 rounded-md border px-3 py-2 text-sm transition-colors"
            :class="taskType === t ? 'border-primary bg-primary/5' : 'border-border hover:bg-accent'">
            <input v-model="taskType" type="radio" :value="t" class="sr-only" />
            {{ getTaskTypeLabel(t) }}
          </label>
        </div>
      </div>

      <div class="space-y-2">
        <label class="text-sm font-medium">优先级</label>
        <div class="flex gap-2">
          <label v-for="(label, p) in PRIORITY_LABELS" :key="p"
            class="flex cursor-pointer items-center gap-2 rounded-md border px-4 py-2 text-sm transition-colors"
            :class="taskPriority === p ? 'border-primary bg-primary/5' : 'border-border hover:bg-accent'">
            <input v-model="taskPriority" type="radio" :value="p" class="sr-only" />
            {{ label }}
          </label>
        </div>
      </div>
    </div>

    <!-- Step 2 -->
    <div v-if="step === 2" class="space-y-4 rounded-lg border border-border bg-card p-6">
      <h3 class="text-lg font-medium">生成配置</h3>

      <div class="space-y-3">
        <label class="flex items-center gap-3 text-sm">
          <input v-model="genModel" type="checkbox" class="h-4 w-4 rounded border-input accent-primary" /> 生成模型
        </label>
        <label class="flex items-center gap-3 text-sm">
          <input v-model="genMesh" type="checkbox" class="h-4 w-4 rounded border-input accent-primary" /> 生成网格
        </label>
        <label class="flex items-center gap-3 text-sm">
          <input v-model="genSpatialTree" type="checkbox" class="h-4 w-4 rounded border-input accent-primary" /> 生成空间树
        </label>
        <label class="flex items-center gap-3 text-sm">
          <input v-model="applyBooleanOp" type="checkbox" class="h-4 w-4 rounded border-input accent-primary" /> 布尔运算
        </label>
      </div>

      <div v-if="genMesh" class="space-y-2">
        <label class="text-sm font-medium">网格公差比率</label>
        <input v-model.number="meshTolRatio" type="number" step="0.1" min="0.1" :class="inputClass" />
      </div>

      <div class="space-y-2">
        <label class="text-sm font-medium">手动 DB Nums <span class="text-muted-foreground">(可选)</span></label>
        <input v-model="manualDbNumsStr" type="text" placeholder="7997, 7998" :class="inputClass" />
      </div>

      <div class="space-y-2">
        <label class="text-sm font-medium">手动 Refnos <span class="text-muted-foreground">(可选)</span></label>
        <input v-model="manualRefnosStr" type="text" placeholder="24381_145018, 1/456" :class="inputClass" />
      </div>

      <div class="grid grid-cols-2 gap-4">
        <label class="flex items-center gap-3 text-sm">
          <input v-model="exportJson" type="checkbox" class="h-4 w-4 rounded border-input accent-primary" /> 导出 JSON
        </label>
        <label class="flex items-center gap-3 text-sm">
          <input v-model="exportParquet" type="checkbox" class="h-4 w-4 rounded border-input accent-primary" /> 导出 Parquet
        </label>
      </div>
    </div>

    <!-- Step 3 -->
    <div v-if="step === 3" class="space-y-4 rounded-lg border border-border bg-card p-6">
      <h3 class="text-lg font-medium">确认任务</h3>
      <div class="grid grid-cols-2 gap-y-3 text-sm">
        <div class="text-muted-foreground">名称</div><div>{{ taskName }}</div>
        <div class="text-muted-foreground">类型</div><div>{{ getTaskTypeLabel(taskType) }}</div>
        <div class="text-muted-foreground">优先级</div><div>{{ PRIORITY_LABELS[taskPriority] }}</div>
        <div class="text-muted-foreground">关联站点</div><div>{{ selectedSite?.project_name ?? '-' }}</div>
        <div class="text-muted-foreground">生成模型</div><div>{{ genModel ? '是' : '否' }}</div>
        <div class="text-muted-foreground">生成网格</div><div>{{ genMesh ? '是' : '否' }}</div>
        <div class="text-muted-foreground">空间树</div><div>{{ genSpatialTree ? '是' : '否' }}</div>
        <div class="text-muted-foreground">布尔运算</div><div>{{ applyBooleanOp ? '是' : '否' }}</div>
        <template v-if="manualDbNumsStr">
          <div class="text-muted-foreground">DB Nums</div><div>{{ manualDbNumsStr }}</div>
        </template>
        <template v-if="manualRefnosStr">
          <div class="text-muted-foreground">Refnos</div><div>{{ manualRefnosStr }}</div>
        </template>
      </div>
      <div class="border-t border-border pt-3 text-sm text-muted-foreground">
        确认后将立即提交站点动作，并在任务列表中按站点运行状态持续对账。
      </div>
    </div>

    <!-- Nav -->
    <div class="flex justify-between">
      <button v-if="step > 1" @click="prevStep"
        class="inline-flex h-9 items-center rounded-md border border-input bg-transparent px-4 text-sm font-medium shadow-sm hover:bg-accent hover:text-accent-foreground transition-colors">
        上一步
      </button>
      <div v-else />
      <button v-if="step < 3" @click="nextStep" :disabled="!canProceedStepOne"
        class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow hover:bg-primary/90 transition-colors disabled:pointer-events-none disabled:opacity-50">
        下一步
      </button>
      <button v-else @click="handleSubmit" :disabled="submitting || !canProceedStepOne"
        class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow hover:bg-primary/90 transition-colors disabled:pointer-events-none disabled:opacity-50">
        {{ submitting ? '创建中...' : '确认创建' }}
      </button>
    </div>
  </div>
</template>
