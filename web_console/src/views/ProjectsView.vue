<script setup lang="ts">
import { computed, onMounted, ref } from 'vue';
import { RefreshCwIcon, SearchIcon } from 'lucide-vue-next';

import ProjectCard from '@/components/ProjectCard.vue';
import { useProjects } from '@/composables/useProjects';

const { projects, isLoading, errorMessage, loadProjects } = useProjects();
const searchQuery = ref('');

const filteredProjects = computed(() => {
  const keyword = searchQuery.value.trim().toLowerCase();
  if (!keyword) return projects.value;
  return projects.value.filter((project) =>
    [project.name, project.owner, project.env, project.notes]
      .filter(Boolean)
      .some((value) => String(value).toLowerCase().includes(keyword)),
  );
});

onMounted(() => {
  loadProjects();
});
</script>

<template>
  <div class="page-stack">
    <section class="hero-panel slim">
      <div>
        <p class="section-tag">模型工程</p>
        <h2>项目列表先按 Plant3D Web 的卡片式入口重构</h2>
        <p class="hero-copy">
          数据直接来自 `/api/projects`，当前已接入统一任务中心导航，后续继续承接部署站点和 Viewer 工作台入口。
        </p>
      </div>
      <button type="button" class="primary-button" @click="loadProjects">
        <RefreshCwIcon class="small-icon" />
        <span>刷新项目</span>
      </button>
    </section>

    <section class="toolbar-panel">
      <label class="search-box">
        <SearchIcon class="small-icon" />
        <input v-model="searchQuery" type="text" placeholder="按项目名、环境、负责人搜索" />
      </label>
      <span class="muted-label">共 {{ filteredProjects.length }} 个项目</span>
    </section>

    <p v-if="isLoading" class="state-text">正在加载项目列表...</p>
    <p v-if="errorMessage" class="state-text error">{{ errorMessage }}</p>

    <section class="projects-grid">
      <ProjectCard v-for="project in filteredProjects" :key="project.id" :project="project" />
    </section>

    <div v-if="!isLoading && filteredProjects.length === 0" class="empty-card large">
      当前没有匹配的项目。
    </div>
  </div>
</template>
