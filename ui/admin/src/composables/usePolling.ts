import { ref, onUnmounted, type Ref } from 'vue'

export function usePolling(fn: () => Promise<void>, intervalMs: number = 30000) {
  const active = ref(false)
  let timer: ReturnType<typeof setInterval> | null = null

  function start() {
    if (active.value) return
    active.value = true
    timer = setInterval(fn, intervalMs)
  }

  function stop() {
    active.value = false
    if (timer) {
      clearInterval(timer)
      timer = null
    }
  }

  function setInterval_(ms: number): Ref<boolean> {
    stop()
    intervalMs = ms
    start()
    return active
  }

  onUnmounted(stop)

  return { active, start, stop, setInterval: setInterval_ }
}
