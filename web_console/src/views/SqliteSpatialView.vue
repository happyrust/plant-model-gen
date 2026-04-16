<script setup lang="ts">
import { onMounted, reactive, ref } from 'vue';
import { DatabaseIcon, PlayIcon, RefreshCwIcon, RotateCcwIcon } from 'lucide-vue-next';

import {
  postFitting,
  postFittingOffset,
  postSteelRelative,
  postSuppoTrays,
  postTraySpan,
  postWallDistance,
  querySpatialStats,
  type SpatialStatsResult,
} from '@/api/space';

type ToolState = {
  suppoRefno: string;
  tolerance?: string;
  suppoType?: string;
  searchRadius?: string;
  targetNouns?: string;
  neighborWindow?: string;
  loading: boolean;
  error: string;
  response: string;
};

function createToolState(seed: Partial<ToolState>): ToolState {
  return {
    suppoRefno: seed.suppoRefno ?? '',
    tolerance: seed.tolerance ?? '',
    suppoType: seed.suppoType ?? '',
    searchRadius: seed.searchRadius ?? '',
    targetNouns: seed.targetNouns ?? '',
    neighborWindow: seed.neighborWindow ?? '',
    loading: false,
    error: '',
    response: '',
  };
}

const fitting = reactive(createToolState({ suppoRefno: '24383/89904' }));
const fittingOffset = reactive(createToolState({ suppoRefno: '24383/88342' }));
const wallDistance = reactive(createToolState({ suppoRefno: '24383/88342', suppoType: 'S2', searchRadius: '5000' }));
const suppoTrays = reactive(createToolState({ suppoRefno: '24383/89904' }));
const steelRelative = reactive(createToolState({ suppoRefno: '24383/89904', searchRadius: '8000' }));
const traySpan = reactive(createToolState({ suppoRefno: '24383/87412', neighborWindow: '5000' }));

const stats = ref<SpatialStatsResult | null>(null);
const statsLoading = ref(false);
const statsError = ref('');

function attachOptionalNumber<T extends Record<string, unknown>>(
  target: T,
  key: keyof T,
  raw: string | undefined,
) {
  if (!raw || !raw.trim()) return;
  const parsed = Number(raw);
  if (!Number.isFinite(parsed)) {
    throw new Error(`${String(key)} 必须是数字`);
  }
  target[key] = parsed as T[keyof T];
}

function formatJson(value: unknown) {
  return JSON.stringify(value, null, 2);
}

async function runAction(state: ToolState, action: () => Promise<unknown>) {
  state.loading = true;
  state.error = '';
  state.response = '';
  try {
    const response = await action();
    state.response = formatJson(response);
  } catch (error) {
    state.error = error instanceof Error ? error.message : String(error);
  } finally {
    state.loading = false;
  }
}

async function loadStats() {
  statsLoading.value = true;
  statsError.value = '';
  try {
    stats.value = await querySpatialStats();
  } catch (error) {
    statsError.value = error instanceof Error ? error.message : String(error);
  } finally {
    statsLoading.value = false;
  }
}

function resetState(state: ToolState, seed: Partial<ToolState>) {
  const fresh = createToolState(seed);
  Object.assign(state, fresh);
}

async function runFitting() {
  await runAction(fitting, () =>
    postFitting({
      suppo_refno: fitting.suppoRefno.trim(),
      ...(fitting.tolerance?.trim() ? { tolerance: Number(fitting.tolerance) } : {}),
    }),
  );
}

async function runFittingOffset() {
  await runAction(fittingOffset, () =>
    postFittingOffset({
      suppo_refno: fittingOffset.suppoRefno.trim(),
      ...(fittingOffset.tolerance?.trim() ? { tolerance: Number(fittingOffset.tolerance) } : {}),
    }),
  );
}

async function runWallDistance() {
  await runAction(wallDistance, () => {
    const payload: Record<string, unknown> = {
      suppo_refno: wallDistance.suppoRefno.trim(),
    };
    if (wallDistance.suppoType?.trim()) payload.suppo_type = wallDistance.suppoType.trim();
    attachOptionalNumber(payload, 'search_radius', wallDistance.searchRadius);
    const targetNouns = wallDistance.targetNouns
      ?.split(',')
      .map((item) => item.trim())
      .filter(Boolean);
    if (targetNouns?.length) payload.target_nouns = targetNouns;
    return postWallDistance(payload as Parameters<typeof postWallDistance>[0]);
  });
}

async function runSuppoTrays() {
  await runAction(suppoTrays, () =>
    postSuppoTrays({
      suppo_refno: suppoTrays.suppoRefno.trim(),
      ...(suppoTrays.tolerance?.trim() ? { tolerance: Number(suppoTrays.tolerance) } : {}),
    }),
  );
}

async function runSteelRelative() {
  await runAction(steelRelative, () => {
    const payload: Record<string, unknown> = {
      suppo_refno: steelRelative.suppoRefno.trim(),
    };
    if (steelRelative.suppoType?.trim()) payload.suppo_type = steelRelative.suppoType.trim();
    attachOptionalNumber(payload, 'search_radius', steelRelative.searchRadius);
    return postSteelRelative(payload as Parameters<typeof postSteelRelative>[0]);
  });
}

async function runTraySpan() {
  await runAction(traySpan, () => {
    const payload: Record<string, unknown> = {
      suppo_refno: traySpan.suppoRefno.trim(),
    };
    attachOptionalNumber(payload, 'neighbor_window', traySpan.neighborWindow);
    return postTraySpan(payload as Parameters<typeof postTraySpan>[0]);
  });
}

onMounted(() => {
  void loadStats();
});
</script>

<template>
  <div class="page-stack">
    <section class="hero-panel slim">
      <div>
        <p class="section-tag">DB</p>
        <h2>SQLite Spatial 现在直接承接支架空间工具</h2>
        <p class="hero-copy">
          六张卡片统一调用 `/api/space/*`，请求体和返回格式与后端当前实现保持一致。
        </p>
      </div>
      <button type="button" class="primary-button" @click="loadStats">
        <RefreshCwIcon class="small-icon" />
        <span>刷新索引状态</span>
      </button>
    </section>

    <section class="quick-grid spatial-quick-grid">
      <article class="quick-card accent-blue emphasis-card">
        <div class="quick-icon-shell">
          <DatabaseIcon class="quick-icon" />
        </div>
        <div>
          <div class="quick-card-head">
            <h3>索引状态</h3>
            <span class="quick-pill">{{ stats?.success ? 'ready' : 'check' }}</span>
          </div>
          <p v-if="stats">{{ stats.index_type }} · {{ stats.total_elements }} 条记录</p>
          <p v-else-if="statsLoading">正在读取 `/api/sqlite-spatial/stats` ...</p>
          <p v-else>还没有加载索引状态</p>
          <p v-if="statsError" class="state-text error compact">{{ statsError }}</p>
        </div>
      </article>
      <article class="quick-card accent-orange emphasis-card">
        <div>
          <div class="quick-card-head">
            <h3>当前样例</h3>
            <span class="quick-pill">6 cards</span>
          </div>
          <p>默认填了已验证过的 refno，打开页面后可以直接执行接口回归。</p>
        </div>
      </article>
    </section>

    <section class="content-grid spatial-grid">
      <article class="panel-card tool-card">
        <div class="panel-head">
          <div>
            <p class="panel-eyebrow">/api/space/fitting</p>
            <h3>支架对应预埋板</h3>
          </div>
          <button type="button" class="tool-button ghost" @click="resetState(fitting, { suppoRefno: '24383/89904' })">
            <RotateCcwIcon class="small-icon" />
            <span>重置样例</span>
          </button>
        </div>
        <div class="field-grid two-col">
          <label class="field-item full"><span>suppo_refno</span><input v-model="fitting.suppoRefno" type="text" placeholder="24383/89904 或 24383_89904" /></label>
          <label class="field-item"><span>tolerance</span><input v-model="fitting.tolerance" type="number" placeholder="可空" /></label>
        </div>
        <div class="tool-actions">
          <button type="button" class="tool-button" :disabled="fitting.loading" @click="runFitting">
            <PlayIcon class="small-icon" />
            <span>{{ fitting.loading ? '请求中...' : '执行' }}</span>
          </button>
        </div>
        <p v-if="fitting.error" class="state-text error compact">{{ fitting.error }}</p>
        <pre v-if="fitting.response" class="result-pre">{{ fitting.response }}</pre>
      </article>

      <article class="panel-card tool-card">
        <div class="panel-head">
          <div>
            <p class="panel-eyebrow">/api/space/fitting-offset</p>
            <h3>支架与预埋板相对定位</h3>
          </div>
          <button type="button" class="tool-button ghost" @click="resetState(fittingOffset, { suppoRefno: '24383/88342' })">
            <RotateCcwIcon class="small-icon" />
            <span>重置样例</span>
          </button>
        </div>
        <div class="field-grid two-col">
          <label class="field-item full"><span>suppo_refno</span><input v-model="fittingOffset.suppoRefno" type="text" placeholder="24383/88342 或 24383_88342" /></label>
          <label class="field-item"><span>tolerance</span><input v-model="fittingOffset.tolerance" type="number" placeholder="可空" /></label>
        </div>
        <div class="tool-actions">
          <button type="button" class="tool-button" :disabled="fittingOffset.loading" @click="runFittingOffset">
            <PlayIcon class="small-icon" />
            <span>{{ fittingOffset.loading ? '请求中...' : '执行' }}</span>
          </button>
        </div>
        <p v-if="fittingOffset.error" class="state-text error compact">{{ fittingOffset.error }}</p>
        <pre v-if="fittingOffset.response" class="result-pre">{{ fittingOffset.response }}</pre>
      </article>

      <article class="panel-card tool-card">
        <div class="panel-head">
          <div>
            <p class="panel-eyebrow">/api/space/wall-distance</p>
            <h3>支架距墙 / 定位块</h3>
          </div>
          <button
            type="button"
            class="tool-button ghost"
            @click="resetState(wallDistance, { suppoRefno: '24383/88342', suppoType: 'S2', searchRadius: '5000' })"
          >
            <RotateCcwIcon class="small-icon" />
            <span>重置样例</span>
          </button>
        </div>
        <div class="field-grid two-col">
          <label class="field-item full"><span>suppo_refno</span><input v-model="wallDistance.suppoRefno" type="text" placeholder="24383/88342 或 24383_88342" /></label>
          <label class="field-item"><span>suppo_type</span><input v-model="wallDistance.suppoType" type="text" placeholder="S1 / S2，可空" /></label>
          <label class="field-item"><span>search_radius</span><input v-model="wallDistance.searchRadius" type="number" placeholder="mm" /></label>
          <label class="field-item full"><span>target_nouns</span><input v-model="wallDistance.targetNouns" type="text" placeholder="逗号分隔，可空" /></label>
        </div>
        <div class="tool-actions">
          <button type="button" class="tool-button" :disabled="wallDistance.loading" @click="runWallDistance">
            <PlayIcon class="small-icon" />
            <span>{{ wallDistance.loading ? '请求中...' : '执行' }}</span>
          </button>
        </div>
        <p v-if="wallDistance.error" class="state-text error compact">{{ wallDistance.error }}</p>
        <pre v-if="wallDistance.response" class="result-pre">{{ wallDistance.response }}</pre>
      </article>

      <article class="panel-card tool-card">
        <div class="panel-head">
          <div>
            <p class="panel-eyebrow">/api/space/suppo-trays</p>
            <h3>支架对应桥架</h3>
          </div>
          <button type="button" class="tool-button ghost" @click="resetState(suppoTrays, { suppoRefno: '24383/89904' })">
            <RotateCcwIcon class="small-icon" />
            <span>重置样例</span>
          </button>
        </div>
        <div class="field-grid two-col">
          <label class="field-item full"><span>suppo_refno</span><input v-model="suppoTrays.suppoRefno" type="text" placeholder="24383/89904 或 24383_89904" /></label>
          <label class="field-item"><span>tolerance</span><input v-model="suppoTrays.tolerance" type="number" placeholder="可空" /></label>
        </div>
        <div class="tool-actions">
          <button type="button" class="tool-button" :disabled="suppoTrays.loading" @click="runSuppoTrays">
            <PlayIcon class="small-icon" />
            <span>{{ suppoTrays.loading ? '请求中...' : '执行' }}</span>
          </button>
        </div>
        <p v-if="suppoTrays.error" class="state-text error compact">{{ suppoTrays.error }}</p>
        <pre v-if="suppoTrays.response" class="result-pre">{{ suppoTrays.response }}</pre>
      </article>

      <article class="panel-card tool-card">
        <div class="panel-head">
          <div>
            <p class="panel-eyebrow">/api/space/steel-relative</p>
            <h3>支架与钢结构相对定位</h3>
          </div>
          <button type="button" class="tool-button ghost" @click="resetState(steelRelative, { suppoRefno: '24383/89904', searchRadius: '8000' })">
            <RotateCcwIcon class="small-icon" />
            <span>重置样例</span>
          </button>
        </div>
        <div class="field-grid two-col">
          <label class="field-item full"><span>suppo_refno</span><input v-model="steelRelative.suppoRefno" type="text" placeholder="24383/89904 或 24383_89904" /></label>
          <label class="field-item"><span>suppo_type</span><input v-model="steelRelative.suppoType" type="text" placeholder="可空" /></label>
          <label class="field-item"><span>search_radius</span><input v-model="steelRelative.searchRadius" type="number" placeholder="mm" /></label>
        </div>
        <div class="tool-actions">
          <button type="button" class="tool-button" :disabled="steelRelative.loading" @click="runSteelRelative">
            <PlayIcon class="small-icon" />
            <span>{{ steelRelative.loading ? '请求中...' : '执行' }}</span>
          </button>
        </div>
        <p v-if="steelRelative.error" class="state-text error compact">{{ steelRelative.error }}</p>
        <pre v-if="steelRelative.response" class="result-pre">{{ steelRelative.response }}</pre>
      </article>

      <article class="panel-card tool-card">
        <div class="panel-head">
          <div>
            <p class="panel-eyebrow">/api/space/tray-span</p>
            <h3>支架跨度</h3>
          </div>
          <button type="button" class="tool-button ghost" @click="resetState(traySpan, { suppoRefno: '24383/87412', neighborWindow: '5000' })">
            <RotateCcwIcon class="small-icon" />
            <span>重置样例</span>
          </button>
        </div>
        <div class="field-grid two-col">
          <label class="field-item full"><span>suppo_refno</span><input v-model="traySpan.suppoRefno" type="text" placeholder="24383/87412 或 24383_87412" /></label>
          <label class="field-item"><span>neighbor_window</span><input v-model="traySpan.neighborWindow" type="number" placeholder="mm" /></label>
        </div>
        <div class="tool-actions">
          <button type="button" class="tool-button" :disabled="traySpan.loading" @click="runTraySpan">
            <PlayIcon class="small-icon" />
            <span>{{ traySpan.loading ? '请求中...' : '执行' }}</span>
          </button>
        </div>
        <p v-if="traySpan.error" class="state-text error compact">{{ traySpan.error }}</p>
        <pre v-if="traySpan.response" class="result-pre">{{ traySpan.response }}</pre>
      </article>
    </section>
  </div>
</template>

<style scoped>
.spatial-quick-grid {
  grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
}

.spatial-grid {
  grid-template-columns: repeat(auto-fit, minmax(360px, 1fr));
}

.tool-card {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.field-grid {
  display: grid;
  gap: 12px;
}

.field-grid.two-col {
  grid-template-columns: repeat(2, minmax(0, 1fr));
}

.field-item {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.field-item.full {
  grid-column: 1 / -1;
}

.field-item span {
  font-size: 12px;
  font-weight: 600;
  color: #4b5563;
}

.field-item input {
  width: 100%;
  border: 1px solid rgba(15, 44, 61, 0.12);
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.9);
  padding: 10px 12px;
  font-size: 14px;
  color: #102a43;
}

.field-item input:focus {
  outline: none;
  border-color: rgba(15, 108, 91, 0.45);
  box-shadow: 0 0 0 3px rgba(15, 108, 91, 0.12);
}

.tool-actions {
  display: flex;
  justify-content: flex-end;
}

.tool-button {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  border: none;
  border-radius: 999px;
  padding: 10px 16px;
  background: linear-gradient(135deg, #0f6c5b, #154c79);
  color: #fff;
  font-weight: 600;
  cursor: pointer;
}

.tool-button.ghost {
  background: rgba(15, 44, 61, 0.08);
  color: #11324d;
}

.tool-button:disabled {
  cursor: wait;
  opacity: 0.7;
}

.result-pre {
  margin: 0;
  max-height: 300px;
  overflow: auto;
  border-radius: 16px;
  background: #0f172a;
  color: #e2e8f0;
  padding: 14px;
  font-size: 12px;
  line-height: 1.6;
}

.compact {
  margin: 0;
}

@media (max-width: 860px) {
  .field-grid.two-col,
  .spatial-grid {
    grid-template-columns: 1fr;
  }
}
</style>
