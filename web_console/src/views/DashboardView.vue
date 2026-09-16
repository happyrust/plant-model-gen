<script setup lang="ts">
import { onMounted } from 'vue';
import { ArrowUpRightIcon, RefreshCwIcon, ListTodoIcon, FolderKanbanIcon } from 'lucide-vue-next';

import MetricCard from '@/components/MetricCard.vue';
import ProjectCard from '@/components/ProjectCard.vue';
import { useDashboard } from '@/composables/useDashboard';

const { metricCards, activities, recentProjects, taskOverview, loading, errorMessage, lastUpdatedLabel, refresh } =
  useDashboard();

onMounted(() => {
  refresh();
});
</script>

<template>
  <div class="page-stack">
    <section class="hero-panel">
      <div>
        <p class="section-tag">全局工作台</p>
        <h2>用 plant3d-web 的壳层方式，重做当前后端控制台首页</h2>
        <p class="hero-copy">
          第一阶段先承接首页和项目列表，保持 Rust API，不接入 3D viewer，只预留入口和交互位置。
        </p>
      </div>
      <button type="button" class="primary-button" @click="refresh">
        <RefreshCwIcon class="small-icon" />
        <span>刷新数据</span>
      </button>
    </section>

    <section class="quick-grid">
      <article class="quick-card accent-blue emphasis-card">
        <div class="quick-icon-shell">
          <ListTodoIcon class="quick-icon" />
        </div>
        <div>
          <div class="quick-card-head">
            <h3>任务概览</h3>
            <span class="quick-pill">{{ taskOverview.healthLabel }}</span>
          </div>
          <p>{{ taskOverview.healthDetail }}</p>
          <dl class="quick-stats">
            <div>
              <dt>运行中</dt>
              <dd>{{ taskOverview.activeCount }}</dd>
            </div>
            <div>
              <dt>排队中</dt>
              <dd>{{ taskOverview.queuedCount }}</dd>
            </div>
          </dl>
        </div>
      </article>
      <article class="quick-card accent-orange emphasis-card">
        <div class="quick-icon-shell">
          <FolderKanbanIcon class="quick-icon" />
        </div>
        <div>
          <div class="quick-card-head">
            <h3>项目概览</h3>
            <span class="quick-pill">{{ recentProjects.length }} / 6</span>
          </div>
          <p>首页仅展示最近项目，超过 6 个时保持摘要视图，并引导跳转到完整项目页。</p>
          <RouterLink to="/projects" class="inline-link quick-inline-link">
            <span>查看全部项目</span>
            <ArrowUpRightIcon class="small-icon" />
          </RouterLink>
        </div>
      </article>
    </section>

    <section class="metric-grid">
      <MetricCard v-for="card in metricCards" :key="card.id" :card="card" />
    </section>

    <p v-if="loading" class="state-text">正在同步首页数据...</p>
    <p v-if="errorMessage" class="state-text error">{{ errorMessage }}</p>

    <section class="content-grid">
      <article class="panel-card">
        <div class="panel-head">
          <div>
            <p class="panel-eyebrow">团队动态</p>
            <h3>最近活动</h3>
          </div>
          <div class="panel-head-meta">
            <span class="muted-label">最近更新时间</span>
            <strong>{{ lastUpdatedLabel }}</strong>
          </div>
        </div>
        <div class="activity-list">
          <div v-if="activities.length === 0" class="empty-card">暂无动态</div>
          <article v-for="activity in activities" :key="activity.id" class="activity-item">
            <div class="activity-avatar">{{ activity.userName.charAt(0).toUpperCase() }}</div>
            <div class="activity-body">
              <div class="activity-head">
                <strong>{{ activity.userName }}</strong>
                <span>{{ activity.createdAt }}</span>
              </div>
              <p>
                {{ activity.actionTitle }}
                <span class="activity-target">[{{ activity.targetName }}]</span>
                <template v-if="activity.actionDesc">，{{ activity.actionDesc }}</template>
              </p>
            </div>
          </article>
        </div>
      </article>

      <article class="panel-card">
        <div class="panel-head">
          <div>
            <p class="panel-eyebrow">项目入口</p>
            <h3>最近项目</h3>
          </div>
          <RouterLink to="/projects" class="inline-link">
            <span>查看全部项目</span>
            <ArrowUpRightIcon class="small-icon" />
          </RouterLink>
        </div>
        <div class="recent-projects">
          <div v-if="recentProjects.length === 0" class="empty-card">暂无项目</div>
          <ProjectCard
            v-for="project in recentProjects"
            :key="project.id"
            :project="project"
            compact
          />
        </div>
      </article>
    </section>
  </div>
</template>
