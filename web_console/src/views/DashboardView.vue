<script setup lang="ts">
import { onMounted } from 'vue';
import { ArrowUpRightIcon, FolderOpenDotIcon, ActivityIcon, RefreshCwIcon } from 'lucide-vue-next';

import MetricCard from '@/components/MetricCard.vue';
import ProjectCard from '@/components/ProjectCard.vue';
import { useDashboard } from '@/composables/useDashboard';

const { metricCards, activities, recentProjects, loading, errorMessage, lastUpdatedLabel, refresh } = useDashboard();

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
      <article class="quick-card accent-blue">
        <FolderOpenDotIcon class="quick-icon" />
        <div>
          <h3>模型工程</h3>
          <p>优先重构项目入口，后续从这里衔接任务页和 Viewer 工作台。</p>
        </div>
      </article>
      <article class="quick-card accent-orange">
        <ActivityIcon class="quick-icon" />
        <div>
          <h3>运行概览</h3>
          <p>直接复用 `/api/status` 与 `/api/dashboard/activities`，不再把 fetch 散落在模板脚本里。</p>
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
          <span class="muted-label">{{ lastUpdatedLabel }}</span>
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
            <span>查看全部</span>
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
