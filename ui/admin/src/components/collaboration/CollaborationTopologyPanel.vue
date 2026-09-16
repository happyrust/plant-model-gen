<script setup lang="ts">
import { computed, ref } from 'vue'
import { Activity, RefreshCw } from 'lucide-vue-next'
import { formatDateTime } from '@/lib/collaboration'
import type {
  CollaborationEnv,
  CollaborationFlowStat,
  CollaborationSiteCard,
} from '@/types/collaboration'

const props = withDefaults(
  defineProps<{
    env: CollaborationEnv | null
    items: CollaborationSiteCard[]
    flows?: CollaborationFlowStat[]
    loading?: boolean
    actionDisabled?: boolean
  }>(),
  { flows: () => [], loading: false, actionDisabled: false },
)

const emit = defineEmits<{
  diagnose: [siteId: string]
  refresh: []
  select: [siteId: string]
}>()

type TimeFilter = '1h' | '24h' | '7d'
type DirFilter = 'all' | 'push' | 'pull'

const timeFilter = ref<TimeFilter>('24h')
const dirFilter = ref<DirFilter>('all')
const selectedId = ref<string | null>(null)

const selfSite = computed(
  () => props.items.find((s) => s.roleLabel === '本站' || s.roleLabel === 'Master') ?? props.items[0] ?? null,
)

const effectiveSelection = computed<CollaborationSiteCard | null>(() => {
  if (!props.items.length) return null
  if (selectedId.value) {
    const hit = props.items.find((s) => s.id === selectedId.value)
    if (hit) return hit
  }
  return selfSite.value
})

const CANVAS = { w: 900, h: 480, cx: 450, cy: 240 }

const layout = computed<Record<string, { x: number; y: number; w: number; h: number }>>(() => {
  const sites = props.items
  const result: Record<string, { x: number; y: number; w: number; h: number }> = {}
  if (!sites.length) return result

  const self = selfSite.value
  const peers = sites.filter((s) => s !== self)

  if (self) {
    result[self.id] = { x: CANVAS.cx - 90, y: CANVAS.cy - 45, w: 180, h: 90 }
  }

  const count = peers.length
  if (!count) return result

  const radius = Math.min(200, 120 + Math.max(0, count - 3) * 10)
  peers.forEach((peer, i) => {
    const angle = (2 * Math.PI * i) / count - Math.PI / 2
    result[peer.id] = {
      x: CANVAS.cx + radius * Math.cos(angle) - 85,
      y: CANVAS.cy + radius * Math.sin(angle) - 40,
      w: 170,
      h: 80,
    }
  })
  return result
})

const filteredFlows = computed(() => {
  if (dirFilter.value === 'all') return props.flows
  return props.flows.filter((f) => f.direction === dirFilter.value)
})

function nodeToneClass(site: CollaborationSiteCard) {
  if (site.availability === 'online') return ''
  if (site.availability === 'cached') return 'warn'
  return 'bad'
}

function toneDotColor(site: CollaborationSiteCard) {
  if (site.availability === 'online') return 'var(--collab-ok)'
  if (site.availability === 'cached') return 'var(--collab-warn)'
  return 'var(--collab-bad)'
}

function nameToSiteId(targetName: string | null | undefined): string | null {
  if (!targetName) return null
  const hit = props.items.find((s) => s.name === targetName || s.location === targetName)
  return hit?.id ?? null
}

function flowToneKey(flow: CollaborationFlowStat): 'ok' | 'warn' | 'bad' {
  if (flow.failed === 0) return 'ok'
  if (flow.failed >= flow.total) return 'bad'
  return 'warn'
}

function flowPath(fromId: string, toId: string) {
  const a = layout.value[fromId]
  const b = layout.value[toId]
  if (!a || !b) return ''
  const ax = a.x + a.w / 2
  const ay = a.y + a.h / 2
  const bx = b.x + b.w / 2
  const by = b.y + b.h / 2
  const mx = (ax + bx) / 2
  const my = (ay + by) / 2 - 14
  return `M ${ax} ${ay} Q ${mx} ${my} ${bx} ${by}`
}

function flowMidpoint(fromId: string, toId: string) {
  const a = layout.value[fromId]
  const b = layout.value[toId]
  if (!a || !b) return { x: 0, y: 0 }
  return {
    x: ((a.x + a.w / 2) + (b.x + b.w / 2)) / 2,
    y: ((a.y + a.h / 2) + (b.y + b.h / 2)) / 2 - 20,
  }
}

const flowEdges = computed(() => {
  const sourceId = selfSite.value?.id ?? null
  if (!sourceId) return []
  return filteredFlows.value
    .map((f) => {
      const peerId = nameToSiteId(f.target_site)
      if (!peerId) return null
      const fromId = f.direction === 'pull' ? peerId : sourceId
      const toId = f.direction === 'pull' ? sourceId : peerId
      const tone = flowToneKey(f)
      return {
        key: `${fromId}-${toId}-${f.direction}`,
        from: fromId,
        to: toId,
        tone,
        label: `${f.direction} · ${f.total}${f.failed ? ` (${f.failed} fail)` : ''}`,
        mid: flowMidpoint(fromId, toId),
        path: flowPath(fromId, toId),
      }
    })
    .filter((v): v is NonNullable<typeof v> => v != null)
})

function handleSelect(id: string) {
  selectedId.value = id
  emit('select', id)
}
</script>

<template>
  <section class="collab-v2 collab-panel">
    <header class="panel-head">
      <h2>拓扑 <em>· {{ items.length }} 节点</em></h2>
      <div class="ctrl">
        <span class="segctl">
          <button v-for="v in (['1h','24h','7d'] as TimeFilter[])" :key="v" :class="{ on: timeFilter === v }" @click="timeFilter = v">{{ v }}</button>
        </span>
        <span class="segctl">
          <button v-for="o in ([{v:'all',l:'全部'},{v:'push',l:'push'},{v:'pull',l:'pull'}] as Array<{v:DirFilter,l:string}>)" :key="o.v" :class="{ on: dirFilter === o.v }" @click="dirFilter = o.v">{{ o.l }}</button>
        </span>
        <button class="btn" :disabled="actionDisabled || loading" @click="emit('refresh')">
          <RefreshCw class="h-3.5 w-3.5" />
          刷新
        </button>
      </div>
    </header>

    <div v-if="loading && !items.length" class="empty">
      <div class="skel" />
    </div>

    <div v-else-if="!items.length" class="empty">
      当前协同组没有登记站点。
    </div>

    <div v-else class="layout">
      <div class="canvas">
        <svg viewBox="0 0 900 480" preserveAspectRatio="xMidYMid meet">
          <defs>
            <marker id="collab-arrow-ok" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
              <path d="M 0 0 L 10 5 L 0 10 z" fill="var(--collab-ok)" />
            </marker>
            <marker id="collab-arrow-warn" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
              <path d="M 0 0 L 10 5 L 0 10 z" fill="var(--collab-warn)" />
            </marker>
            <marker id="collab-arrow-bad" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
              <path d="M 0 0 L 10 5 L 0 10 z" fill="var(--collab-bad)" />
            </marker>
          </defs>

          <g v-for="edge in flowEdges" :key="edge.key">
            <path :class="['flow', edge.tone]" :d="edge.path" :marker-end="`url(#collab-arrow-${edge.tone})`" />
            <text class="flow-label" :x="edge.mid.x" :y="edge.mid.y" text-anchor="middle">{{ edge.label }}</text>
          </g>

          <g
            v-for="site in items"
            :key="site.id"
            :class="['node', nodeToneClass(site), { self: site === selfSite, selected: effectiveSelection?.id === site.id }]"
            :transform="`translate(${layout[site.id]?.x || 0} ${layout[site.id]?.y || 0})`"
            @click="handleSelect(site.id)"
          >
            <rect class="node-box" :width="layout[site.id]?.w || 0" :height="layout[site.id]?.h || 0" rx="12" />
            <circle :cx="14" :cy="16" :r="4" :fill="toneDotColor(site)" />
            <text class="node-label" :x="(layout[site.id]?.w || 0) / 2" y="32">{{ site.name }}</text>
            <text class="node-sub" :x="(layout[site.id]?.w || 0) / 2" y="52">{{ (site.httpHost || '').replace(/^https?:\/\//, '') || '—' }}</text>
            <foreignObject
              v-if="site.detectionStatus && site.detectionStatusLabel"
              :x="0"
              :y="(layout[site.id]?.h || 0) - 58"
              :width="layout[site.id]?.w || 0"
              height="22"
            >
              <div xmlns="http://www.w3.org/1999/xhtml" class="node-chip-wrap">
                <span :class="['node-chip', site.detectionStatus]">
                  <span class="d" />{{ site.detectionStatusLabel }}
                </span>
              </div>
            </foreignObject>
            <line :x1="20" :y1="(layout[site.id]?.h || 0) - 22" :x2="(layout[site.id]?.w || 0) - 20" :y2="(layout[site.id]?.h || 0) - 22" stroke="var(--collab-line)" stroke-width="1" />
            <text class="node-stamp" :x="(layout[site.id]?.w || 0) / 2" :y="(layout[site.id]?.h || 0) - 8">
              {{ site.location || site.roleLabel }}{{ site.siteCode ? ` · ${site.siteCode}` : '' }}
            </text>
          </g>
        </svg>
      </div>

      <aside v-if="effectiveSelection" class="detail">
        <div class="detail-head">
          <h3>{{ effectiveSelection.name }}</h3>
          <span v-if="effectiveSelection.siteCode" class="site-code">{{ effectiveSelection.siteCode }}</span>
        </div>
        <div class="meta">
          <span :class="['pill', effectiveSelection.availability === 'online' ? 'ok' : effectiveSelection.availability === 'cached' ? 'warn' : 'bad']">
            <span class="dot" />{{ effectiveSelection.availabilityLabel }}
          </span>
          <span class="role" :class="{ master: effectiveSelection === selfSite }">{{ effectiveSelection.roleLabel }}</span>
          <span v-if="effectiveSelection.detectionStatus && effectiveSelection.detectionStatusLabel"
                :class="['node-chip', effectiveSelection.detectionStatus]">
            <span class="d" />{{ effectiveSelection.detectionStatusLabel }}
          </span>
        </div>
        <div v-if="effectiveSelection.progress != null" class="progress">
          <div class="bar"><div class="f" :style="{ width: `${Math.max(2, effectiveSelection.progress)}%` }" /></div>
          <div class="pc">{{ Math.round(effectiveSelection.progress) }}% · 待同步 {{ effectiveSelection.pendingItems ?? 0 }}</div>
        </div>
        <dl class="kv">
          <dt>HTTP</dt><dd class="mono">{{ effectiveSelection.httpHost || '-' }}</dd>
          <dt>dbnums</dt><dd class="mono">{{ effectiveSelection.dbnums || '-' }}</dd>
          <dt>文件数</dt><dd class="mono">{{ effectiveSelection.fileCount.toLocaleString() }}</dd>
          <dt>记录数</dt><dd class="mono">{{ effectiveSelection.totalRecordCount.toLocaleString() }}</dd>
          <dt>元数据</dt><dd>{{ effectiveSelection.metadataSourceLabel }}</dd>
          <dt>备注</dt><dd>{{ effectiveSelection.notes || '—' }}</dd>
        </dl>
        <div class="diag">
          <div>最近诊断 · <strong :class="effectiveSelection.diagnosticTone">{{ effectiveSelection.diagnosticStatusLabel }}</strong> · {{ formatDateTime(effectiveSelection.diagnosticCheckedAt) }}{{ effectiveSelection.diagnosticLatencyMs != null ? ` · ${effectiveSelection.diagnosticLatencyMs}ms` : '' }}</div>
          <div class="mono-note">{{ effectiveSelection.diagnosticMessage || '尚未执行站点诊断' }}</div>
          <button
            class="btn"
            style="margin-top: 12px;"
            :disabled="actionDisabled || effectiveSelection.diagnosticPending"
            @click="emit('diagnose', effectiveSelection.id)"
          >
            <Activity class="h-3.5 w-3.5" />
            {{ effectiveSelection.diagnosticPending ? '诊断中' : '立即诊断' }}
          </button>
        </div>
      </aside>
    </div>

    <div class="legend">
      <span><i class="ok" />正常同步</span>
      <span><i class="warn" />部分失败</span>
      <span><i class="bad" />全部失败</span>
      <span class="src">数据源 <code>/api/remote-sync/stats/flows</code> + <code>/envs/:id/sites</code></span>
    </div>
  </section>
</template>

<style scoped>
.collab-v2 { font-family: var(--collab-font-body); color: var(--collab-ink-900); }
.collab-panel { border: 1px solid var(--collab-line); border-radius: 12px; background: #fff; padding: 22px 24px; }

.panel-head { display: flex; align-items: center; gap: 16px; margin-bottom: 14px; }
.panel-head h2 { font-family: var(--collab-font-body); font-weight: 500; font-size: 17px; margin: 0; letter-spacing: -.005em; color: var(--collab-ink-900); }
.panel-head h2 em { font-style: normal; color: var(--collab-ink-500); font-weight: 400; }
.panel-head .ctrl { margin-left: auto; display: flex; gap: 6px; align-items: center; }

.segctl { display: inline-flex; border: 1px solid var(--collab-line); border-radius: 8px; overflow: hidden; background: var(--collab-bg); }
.segctl button { padding: 6px 12px; border: 0; background: transparent; font-size: 12.5px; color: var(--collab-ink-500); cursor: pointer; font-family: inherit; }
.segctl button.on { background: #fff; color: var(--collab-ink-900); box-shadow: inset 0 0 0 1px var(--collab-brand); }

.btn { display: inline-flex; align-items: center; gap: 6px; height: 30px; padding: 0 12px; font-size: 12px; border: 1px solid var(--collab-line); background: #fff; color: var(--collab-ink-700); border-radius: 8px; cursor: pointer; font-family: inherit; }
.btn:hover { border-color: var(--collab-ink-400); }
.btn:disabled { opacity: .5; cursor: not-allowed; }

.empty { height: 240px; border: 1px dashed var(--collab-line); border-radius: 10px; background: var(--collab-bg); display: flex; align-items: center; justify-content: center; color: var(--collab-ink-500); font-size: 13px; }
.skel { width: 80%; height: 60%; border-radius: 8px; background: var(--collab-line-soft); animation: pulse 1.2s ease-in-out infinite; }
@keyframes pulse { 0%,100%{opacity:.5} 50%{opacity:1} }

.layout { display: grid; grid-template-columns: minmax(0, 1fr) 340px; gap: 20px; }

.canvas {
  position: relative; border: 1px solid var(--collab-line-soft); border-radius: 10px;
  background:
    linear-gradient(#FCFBF8, #FCFBF8),
    repeating-linear-gradient(0deg, transparent 0 47px, rgba(15,23,42,.035) 47px 48px),
    repeating-linear-gradient(90deg, transparent 0 47px, rgba(15,23,42,.035) 47px 48px);
  background-blend-mode: normal, multiply, multiply;
  height: 460px; overflow: hidden;
}
.canvas svg { position: absolute; inset: 0; width: 100%; height: 100%; }

.node { cursor: pointer; transition: filter .2s; }
.node:hover { filter: drop-shadow(0 4px 10px rgba(31,58,104,.22)); }
.node-box { fill: #fff; stroke: var(--collab-ink-300); stroke-width: 1; }
.node.self .node-box { stroke: var(--collab-brand); stroke-width: 1.5; fill: var(--collab-brand-soft); }
.node.selected .node-box { stroke: var(--collab-brand); stroke-width: 2; }
.node.warn .node-box { stroke: var(--collab-warn); fill: var(--collab-warn-bg); }
.node.bad .node-box { stroke: var(--collab-bad); fill: var(--collab-bad-bg); }
.node-label { font-family: var(--collab-font-body); font-size: 13px; font-weight: 500; fill: var(--collab-ink-900); text-anchor: middle; }
.node-sub { font-family: var(--collab-font-mono); font-size: 11px; fill: var(--collab-ink-700); text-anchor: middle; font-variant-numeric: tabular-nums; }
.node-stamp { font-family: var(--collab-font-mono); font-size: 10px; fill: var(--collab-ink-500); text-anchor: middle; letter-spacing: .15em; }
.node.self .node-label { fill: var(--collab-brand-strong); }
.node.self .node-sub { fill: var(--collab-brand); }

.flow { fill: none; stroke-linecap: round; }
.flow.ok { stroke: var(--collab-ok); stroke-width: 2.5; }
.flow.warn { stroke: var(--collab-warn); stroke-width: 2.5; }
.flow.bad { stroke: var(--collab-bad); stroke-width: 2.5; stroke-dasharray: 6 5; }
.flow-label { font-family: var(--collab-font-mono); font-size: 10px; fill: var(--collab-ink-700); font-variant-numeric: tabular-nums; }

.detail { border: 1px solid var(--collab-line-soft); border-radius: 10px; padding: 18px; background: var(--collab-bg); display: flex; flex-direction: column; }
.detail-head { display: flex; align-items: baseline; gap: 10px; margin-bottom: 10px; flex-wrap: wrap; }
.detail h3 { font-family: var(--collab-font-display); font-size: 20px; font-weight: 500; margin: 0; letter-spacing: -.01em; }
.site-code { font-family: var(--collab-font-mono); font-size: 10.5px; color: var(--collab-ink-400); letter-spacing: .12em; font-weight: 500; }
.progress { margin-bottom: 14px; }
.progress .bar { height: 3px; background: var(--collab-line-soft); border-radius: 2px; overflow: hidden; margin-bottom: 4px; }
.progress .bar .f { height: 100%; background: var(--collab-ok); border-radius: 2px; }
.progress .pc { font-family: var(--collab-font-mono); font-size: 11px; color: var(--collab-ink-500); font-variant-numeric: tabular-nums; }
.detail .meta { display: flex; align-items: center; gap: 8px; margin-bottom: 14px; flex-wrap: wrap; }
.node-chip { display: inline-flex; align-items: center; gap: 4px; padding: 2px 8px; border-radius: 4px; font-size: 10.5px; font-weight: 500; }
.node-chip .d { width: 5px; height: 5px; border-radius: 999px; background: currentColor; }
.node-chip.Idle, .node-chip.Completed { background: var(--collab-line-soft); color: var(--collab-ink-500); }
.node-chip.Scanning { background: var(--collab-brand-soft); color: var(--collab-brand); }
.node-chip.ChangesDetected { background: var(--collab-warn-bg); color: var(--collab-warn); }
.node-chip.Syncing { background: var(--collab-ok-bg); color: var(--collab-ok); }
.node-chip.Error { background: var(--collab-bad-bg); color: var(--collab-bad); }
.node-chip.Scanning .d, .node-chip.Syncing .d { animation: collab-topo-pulse 1.4s ease-in-out infinite; }
@keyframes collab-topo-pulse { 0%, 100% { opacity: 1 } 50% { opacity: .4 } }
:deep(.node-chip-wrap) { display: flex; justify-content: center; pointer-events: none; }
.pill { display: inline-flex; align-items: center; gap: 6px; padding: 3px 10px; border-radius: 999px; font-size: 11.5px; font-weight: 500; }
.pill .dot { width: 6px; height: 6px; border-radius: 999px; background: currentColor; }
.pill.ok { background: var(--collab-ok-bg); color: var(--collab-ok); }
.pill.warn { background: var(--collab-warn-bg); color: var(--collab-warn); }
.pill.bad { background: var(--collab-bad-bg); color: var(--collab-bad); }
.role { display: inline-flex; padding: 2px 8px; border-radius: 4px; font-size: 11px; background: var(--collab-line-soft); color: var(--collab-ink-700); font-weight: 500; }
.role.master { background: var(--collab-brand-soft); color: var(--collab-brand); }
.kv { display: grid; grid-template-columns: 76px 1fr; gap: 6px 12px; margin: 0; font-size: 12.5px; color: var(--collab-ink-700); }
.kv dt { color: var(--collab-ink-500); margin: 0; }
.kv dd { margin: 0; word-break: break-all; }
.kv dd.mono { font-family: var(--collab-font-mono); font-size: 11.5px; font-variant-numeric: tabular-nums; }
.diag { margin-top: auto; border-top: 1px dashed var(--collab-line); padding-top: 12px; font-size: 12px; color: var(--collab-ink-500); }
.diag strong { font-weight: 500; color: var(--collab-ink-700); }
.diag strong.success { color: var(--collab-ok); }
.diag strong.warning { color: var(--collab-warn); }
.diag strong.danger { color: var(--collab-bad); }
.mono-note { margin-top: 4px; font-family: var(--collab-font-mono); font-size: 11px; color: var(--collab-ink-500); font-variant-numeric: tabular-nums; }

.legend { display: flex; gap: 16px; margin-top: 12px; padding-left: 4px; font-size: 11.5px; color: var(--collab-ink-500); flex-wrap: wrap; }
.legend span { display: inline-flex; align-items: center; gap: 6px; }
.legend i { display: inline-block; width: 18px; height: 2px; border-radius: 2px; }
.legend i.ok { background: var(--collab-ok); }
.legend i.warn { background: var(--collab-warn); }
.legend i.bad { background: repeating-linear-gradient(90deg, var(--collab-bad) 0 4px, transparent 4px 8px); height: 2px; }
.legend .src { margin-left: auto; color: var(--collab-ink-400); }
.legend code { font-family: var(--collab-font-mono); font-size: 11px; padding: 0 4px; background: var(--collab-line-soft); border-radius: 3px; color: var(--collab-ink-700); }
</style>
