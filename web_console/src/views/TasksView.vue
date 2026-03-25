<script setup lang="ts">
import { onMounted } from 'vue';
import { Layers3Icon, ListTodoIcon, RefreshCwIcon } from 'lucide-vue-next';

import { useDashboard } from '@/composables/useDashboard';

const { metricCards, loading, errorMessage, refresh } = useDashboard();

onMounted(() => {
  refresh();
});
</script>

<template>
  <div class="page-stack">
    <section class="hero-panel slim">
      <div>
        <p class="section-tag">任务中心</p>
        <h2>先把任务页迁入 Vue 壳层，下一步再细化交互与操作流</h2>
        <p class="hero-copy">
          当前阶段先承接任务入口和系统概览，后续继续把现有 `/tasks`、`/batch-tasks` 的操作表单逐步搬迁过来。
        </p>
      </div>
      <button type="button" class="primary-button" @click="refresh">
        <RefreshCwIcon class="small-icon" />
        <span>刷新任务概览</span>
      </button>
    </section>

    <section class="quick-grid">
      <article class="quick-card accent-blue">
        <ListTodoIcon class="quick-icon" />
        <div>
          <h3>任务管理</h3>
          <p>现有后端能力仍由 `/api/tasks` 驱动，下一步迁移列表、详情和操作按钮。</p>
        </div>
      </article>
      <article class="quick-card accent-orange">
        <Layers3Icon class="quick-icon" />
        <div>
          <h3>批量任务</h3>
          <p>保留统一入口，后续将把批量任务配置页并入同一个 SPA 工作台。</p>
        </div>
      </article>
    </section>

    <section class="metric-grid">
      <article v-for="card in metricCards" :key="card.id" class="metric-card">
        <p class="metric-label">{{ card.label }}</p>
        <p class="metric-value">{{ card.value }}</p>
        <p class="metric-hint">{{ card.hint }}</p>
      </article>
    </section>

    <p v-if="loading" class="state-text">正在同步任务概览...</p>
    <p v-if="errorMessage" class="state-text error">{{ errorMessage }}</p>

    <section class="content-grid single-column">
      <article class="panel-card">
        <div class="panel-head">
          <div>
            <p class="panel-eyebrow">迁移计划</p>
            <h3>下一阶段准备迁移的能力</h3>
          </div>
        </div>
        <div class="migration-list">
          <div class="migration-item">
            <strong>任务列表</strong>
            <p>将原 `/tasks` 的列表、状态筛选、日志入口迁入 Vue 页面。</p>
          </div>
          <div class="migration-item">
            <strong>批量任务入口</strong>
            <p>把 `/batch-tasks` 的入口合并到同一导航体系，统一视觉和数据层。</p>
          </div>
          <div class="migration-item">
            <strong>Viewer 预留位</strong>
            <p>继续保留项目上下文和 Viewer 按钮，但不在当前阶段接入 3D 工作台。</p>
          </div>
        </div>
      </article>
    </section>
  </div>
</template>
