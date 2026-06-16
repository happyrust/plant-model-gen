<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { Check, ChevronLeft, ChevronRight, X } from 'lucide-vue-next'

interface SiteDeploymentGuideStep {
  id: string
  targetSelector: string
  title: string
  description: string
  actionHint?: string
}

const props = defineProps<{
  open: boolean
  steps: SiteDeploymentGuideStep[]
}>()

const emit = defineEmits<{
  'update:open': [value: boolean]
}>()

const currentIndex = ref(0)
const targetRect = ref<DOMRect | null>(null)
const targetMissing = ref(false)

const currentStep = computed(() => props.steps[currentIndex.value] ?? null)
const isFirstStep = computed(() => currentIndex.value === 0)
const isLastStep = computed(() => currentIndex.value >= props.steps.length - 1)
const progressLabel = computed(() => `${Math.min(currentIndex.value + 1, props.steps.length)} / ${props.steps.length}`)

const highlightStyle = computed(() => {
  const rect = targetRect.value
  if (!rect) return {}
  return {
    left: `${rect.left - 6}px`,
    top: `${rect.top - 6}px`,
    width: `${rect.width + 12}px`,
    height: `${rect.height + 12}px`,
  }
})

const panelStyle = computed(() => {
  const rect = targetRect.value
  if (!rect || targetMissing.value) {
    return {
      left: '50%',
      top: '50%',
      transform: 'translate(-50%, -50%)',
    }
  }
  const panelWidth = 360
  const margin = 16
  const left = Math.min(
    Math.max(margin, rect.left),
    Math.max(margin, window.innerWidth - panelWidth - margin),
  )
  const below = rect.bottom + margin
  const top = below + 220 < window.innerHeight
    ? below
    : Math.max(margin, rect.top - 236)
  return {
    left: `${left}px`,
    top: `${top}px`,
  }
})

async function syncTarget() {
  if (!props.open || !currentStep.value) return
  await nextTick()
  const element = document.querySelector(currentStep.value.targetSelector) as HTMLElement | null
  if (!element) {
    targetRect.value = null
    targetMissing.value = true
    return
  }
  element.scrollIntoView({ block: 'center', inline: 'nearest', behavior: 'smooth' })
  window.setTimeout(() => {
    targetRect.value = element.getBoundingClientRect()
    targetMissing.value = false
  }, 220)
}

function closeGuide() {
  emit('update:open', false)
}

async function nextStep() {
  if (isLastStep.value) {
    closeGuide()
    return
  }
  currentIndex.value += 1
  await syncTarget()
}

async function prevStep() {
  if (isFirstStep.value) return
  currentIndex.value -= 1
  await syncTarget()
}

function handleViewportChange() {
  if (!props.open || !currentStep.value) return
  const element = document.querySelector(currentStep.value.targetSelector) as HTMLElement | null
  targetRect.value = element?.getBoundingClientRect() ?? null
  targetMissing.value = !element
}

watch(() => props.open, async (open) => {
  if (!open) return
  currentIndex.value = 0
  await syncTarget()
})

watch(currentStep, () => {
  void syncTarget()
})

window.addEventListener('resize', handleViewportChange)
window.addEventListener('scroll', handleViewportChange, true)

onBeforeUnmount(() => {
  window.removeEventListener('resize', handleViewportChange)
  window.removeEventListener('scroll', handleViewportChange, true)
})
</script>

<template>
  <Teleport to="body">
    <Transition name="site-guide">
      <div v-if="open && currentStep" class="fixed inset-0 z-[70]">
        <div class="absolute inset-0 bg-black/35" @click="closeGuide" />
        <div
          v-if="targetRect && !targetMissing"
          class="pointer-events-none fixed rounded-lg border-2 border-primary bg-primary/10 shadow-[0_0_0_9999px_rgba(0,0,0,0.25)]"
          :style="highlightStyle"
        />
        <section
          class="fixed w-[360px] max-w-[calc(100vw-32px)] rounded-lg border border-border bg-popover p-4 text-popover-foreground shadow-xl"
          :style="panelStyle"
        >
          <div class="flex items-start justify-between gap-3">
            <div>
              <div class="text-xs font-medium text-muted-foreground">站点部署向导 {{ progressLabel }}</div>
              <h3 class="mt-1 text-base font-semibold">{{ currentStep.title }}</h3>
            </div>
            <button
              type="button"
              class="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md hover:bg-accent"
              @click="closeGuide"
            >
              <X class="h-4 w-4" />
            </button>
          </div>
          <p class="mt-2 text-sm leading-6 text-muted-foreground">{{ currentStep.description }}</p>
          <p v-if="targetMissing" class="mt-3 rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-xs text-amber-800">
            {{ currentStep.actionHint || '当前步骤的目标控件暂未显示，请按页面提示展开对应区域后继续。' }}
          </p>
          <div class="mt-4 h-1.5 overflow-hidden rounded-full bg-muted">
            <div
              class="h-full rounded-full bg-primary transition-all"
              :style="{ width: `${((currentIndex + 1) / steps.length) * 100}%` }"
            />
          </div>
          <div class="mt-4 flex items-center justify-between gap-2">
            <button
              type="button"
              class="inline-flex h-9 items-center gap-1.5 rounded-md border border-input px-3 text-sm font-medium hover:bg-accent disabled:pointer-events-none disabled:opacity-50"
              :disabled="isFirstStep"
              @click="prevStep"
            >
              <ChevronLeft class="h-4 w-4" />
              上一步
            </button>
            <button
              type="button"
              class="inline-flex h-9 items-center gap-1.5 rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground hover:bg-primary/90"
              @click="nextStep"
            >
              <Check v-if="isLastStep" class="h-4 w-4" />
              <ChevronRight v-else class="h-4 w-4" />
              {{ isLastStep ? '完成' : '下一步' }}
            </button>
          </div>
        </section>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.site-guide-enter-active,
.site-guide-leave-active {
  transition: opacity 0.15s ease;
}

.site-guide-enter-from,
.site-guide-leave-to {
  opacity: 0;
}
</style>
