<script setup lang="ts">
import { ref } from 'vue'
import { useRouter } from 'vue-router'
import { useTasksStore } from '@/stores/tasks'
import type { TaskType, TaskPriority, ParseConfig, GenModelConfig } from '@/types/task'

const router = useRouter()
const tasksStore = useTasksStore()

const step = ref(1)
const submitting = ref(false)

// Step 1
const taskName = ref('')
const taskType = ref<TaskType>('parse')
const taskPriority = ref<TaskPriority>('medium')
const taskDescription = ref('')

// Step 2 - Parse
const parseMode = ref<'full' | 'by_dbnum' | 'by_ref'>('full')
const parseDbNums = ref('')
const parseRefStart = ref('')
const parseRefEnd = ref('')
const parseConcurrency = ref(4)

// Step 2 - GenModel
const genModel = ref(true)
const genMesh = ref(true)
const genSpatialTree = ref(false)
const genBooleanOps = ref(false)
const meshTolerance = ref(0.001)
const genConcurrency = ref(4)
const exportWebPackage = ref(false)

function nextStep() {
  if (step.value < 3) step.value++
}

function prevStep() {
  if (step.value > 1) step.value--
}

function buildConfig() {
  if (taskType.value === 'parse') {
    const cfg: ParseConfig = {
      mode: parseMode.value,
      max_concurrency: parseConcurrency.value,
    }
    if (parseMode.value === 'by_dbnum') {
      cfg.db_nums = parseDbNums.value.split(',').map(Number).filter(Boolean)
    }
    if (parseMode.value === 'by_ref') {
      cfg.ref_range = { start: parseRefStart.value, end: parseRefEnd.value }
    }
    return cfg
  }
  const cfg: GenModelConfig = {
    generate_model: genModel.value,
    generate_mesh: genMesh.value,
    generate_spatial_tree: genSpatialTree.value,
    generate_boolean_ops: genBooleanOps.value,
    mesh_tolerance: meshTolerance.value,
    max_concurrency: genConcurrency.value,
    export_web_package: exportWebPackage.value,
  }
  return cfg
}

async function handleSubmit() {
  submitting.value = true
  try {
    await tasksStore.createTask({
      name: taskName.value,
      type: taskType.value,
      priority: taskPriority.value,
      description: taskDescription.value || undefined,
      config: buildConfig(),
    })
    router.push({ name: 'tasks' })
  } finally {
    submitting.value = false
  }
}

const stepLabels = ['基础信息', '配置参数', '确认创建']
const typeLabels: Record<TaskType, string> = {
  parse: '数据解析',
  gen_model: '模型生成',
  export: '模型导出',
}
const priorityLabels: Record<TaskPriority, string> = { low: '低', medium: '中', high: '高' }
</script>

<template>
  <div class="mx-auto max-w-2xl space-y-6">
    <div>
      <h2 class="text-2xl font-semibold tracking-tight">创建任务</h2>
      <p class="text-sm text-muted-foreground">按步骤配置任务参数</p>
    </div>

    <!-- Stepper -->
    <div class="flex items-center gap-2">
      <template v-for="(label, idx) in stepLabels" :key="idx">
        <div class="flex items-center gap-2">
          <div
            class="flex h-8 w-8 items-center justify-center rounded-full text-sm font-medium"
            :class="step > idx + 1 ? 'bg-primary text-primary-foreground'
              : step === idx + 1 ? 'bg-primary text-primary-foreground'
              : 'bg-muted text-muted-foreground'"
          >{{ idx + 1 }}</div>
          <span class="text-sm" :class="step >= idx + 1 ? 'text-foreground' : 'text-muted-foreground'">{{ label }}</span>
        </div>
        <div v-if="idx < stepLabels.length - 1" class="h-px flex-1 bg-border" />
      </template>
    </div>

    <!-- Step 1: Basic Info -->
    <div v-if="step === 1" class="space-y-4 rounded-lg border border-border bg-card p-6">
      <div class="space-y-2">
        <label class="text-sm font-medium">任务名称</label>
        <input v-model="taskName" type="text" required placeholder="例：全量解析 Site-A"
          class="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring" />
      </div>
      <div class="space-y-2">
        <label class="text-sm font-medium">任务类型</label>
        <div class="flex gap-3">
          <label v-for="t in (['parse', 'gen_model', 'export'] as TaskType[])" :key="t"
            class="flex cursor-pointer items-center gap-2 rounded-md border px-4 py-2 text-sm transition-colors"
            :class="taskType === t ? 'border-primary bg-primary/5' : 'border-border hover:bg-accent'">
            <input v-model="taskType" type="radio" :value="t" class="sr-only" />
            {{ typeLabels[t] }}
          </label>
        </div>
      </div>
      <div class="space-y-2">
        <label class="text-sm font-medium">优先级</label>
        <select v-model="taskPriority"
          class="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring">
          <option v-for="p in (['low', 'medium', 'high'] as TaskPriority[])" :key="p" :value="p">{{ priorityLabels[p] }}</option>
        </select>
      </div>
      <div class="space-y-2">
        <label class="text-sm font-medium">描述 <span class="text-muted-foreground">(可选)</span></label>
        <textarea v-model="taskDescription" rows="3"
          class="flex w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring" />
      </div>
    </div>

    <!-- Step 2: Parse Config -->
    <div v-if="step === 2 && taskType === 'parse'" class="space-y-4 rounded-lg border border-border bg-card p-6">
      <h3 class="text-lg font-medium">解析配置</h3>
      <div class="space-y-2">
        <label class="text-sm font-medium">解析模式</label>
        <div class="flex gap-3">
          <label v-for="m in ['full', 'by_dbnum', 'by_ref']" :key="m"
            class="flex cursor-pointer items-center gap-2 rounded-md border px-4 py-2 text-sm transition-colors"
            :class="parseMode === m ? 'border-primary bg-primary/5' : 'border-border hover:bg-accent'">
            <input v-model="parseMode" type="radio" :value="m" class="sr-only" />
            {{ m === 'full' ? '全量' : m === 'by_dbnum' ? '按 DB Num' : '按引用' }}
          </label>
        </div>
      </div>
      <div v-if="parseMode === 'by_dbnum'" class="space-y-2">
        <label class="text-sm font-medium">DB Num 列表</label>
        <input v-model="parseDbNums" type="text" placeholder="1,2,3"
          class="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring" />
      </div>
      <div v-if="parseMode === 'by_ref'" class="grid grid-cols-2 gap-4">
        <div class="space-y-2">
          <label class="text-sm font-medium">引用起始</label>
          <input v-model="parseRefStart" type="text"
            class="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring" />
        </div>
        <div class="space-y-2">
          <label class="text-sm font-medium">引用结束</label>
          <input v-model="parseRefEnd" type="text"
            class="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring" />
        </div>
      </div>
      <div class="space-y-2">
        <label class="text-sm font-medium">最大并发: {{ parseConcurrency }}</label>
        <input v-model.number="parseConcurrency" type="range" min="1" max="32"
          class="w-full accent-primary" />
      </div>
    </div>

    <!-- Step 2: GenModel Config -->
    <div v-if="step === 2 && taskType === 'gen_model'" class="space-y-4 rounded-lg border border-border bg-card p-6">
      <h3 class="text-lg font-medium">模型生成配置</h3>
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
          <input v-model="genBooleanOps" type="checkbox" class="h-4 w-4 rounded border-input accent-primary" /> 布尔运算
        </label>
      </div>
      <div v-if="genMesh" class="space-y-2">
        <label class="text-sm font-medium">网格公差</label>
        <input v-model.number="meshTolerance" type="number" step="0.0001"
          class="flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring" />
      </div>
      <div class="space-y-2">
        <label class="text-sm font-medium">最大并发: {{ genConcurrency }}</label>
        <input v-model.number="genConcurrency" type="range" min="1" max="32"
          class="w-full accent-primary" />
      </div>
      <label class="flex items-center gap-3 text-sm">
        <input v-model="exportWebPackage" type="checkbox" class="h-4 w-4 rounded border-input accent-primary" /> 导出 Web 数据包
      </label>
    </div>

    <!-- Step 2: Export placeholder -->
    <div v-if="step === 2 && taskType === 'export'" class="rounded-lg border border-border bg-card p-6 text-center text-muted-foreground">
      导出配置（待实现）
    </div>

    <!-- Step 3: Confirm -->
    <div v-if="step === 3" class="space-y-4 rounded-lg border border-border bg-card p-6">
      <h3 class="text-lg font-medium">确认任务</h3>
      <div class="grid grid-cols-2 gap-3 text-sm">
        <div class="text-muted-foreground">名称</div><div>{{ taskName }}</div>
        <div class="text-muted-foreground">类型</div><div>{{ typeLabels[taskType] }}</div>
        <div class="text-muted-foreground">优先级</div><div>{{ priorityLabels[taskPriority] }}</div>
        <div v-if="taskDescription" class="text-muted-foreground">描述</div>
        <div v-if="taskDescription">{{ taskDescription }}</div>
      </div>
      <div class="border-t border-border pt-3 text-sm text-muted-foreground">
        配置参数已就绪，点击确认开始创建任务。
      </div>
    </div>

    <!-- Navigation -->
    <div class="flex justify-between">
      <button v-if="step > 1" @click="prevStep"
        class="inline-flex h-9 items-center rounded-md border border-input bg-transparent px-4 text-sm font-medium shadow-sm hover:bg-accent hover:text-accent-foreground transition-colors">
        上一步
      </button>
      <div v-else />
      <button v-if="step < 3" @click="nextStep" :disabled="!taskName"
        class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow hover:bg-primary/90 transition-colors disabled:pointer-events-none disabled:opacity-50">
        下一步
      </button>
      <button v-else @click="handleSubmit" :disabled="submitting"
        class="inline-flex h-9 items-center rounded-md bg-primary px-4 text-sm font-medium text-primary-foreground shadow hover:bg-primary/90 transition-colors disabled:pointer-events-none disabled:opacity-50">
        {{ submitting ? '创建中...' : '确认创建' }}
      </button>
    </div>
  </div>
</template>
