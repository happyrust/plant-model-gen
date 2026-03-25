<script setup lang="ts">
import { computed } from 'vue';
import { useRouter } from 'vue-router';
import { ArrowRightIcon, BadgeCheckIcon, FolderRootIcon } from 'lucide-vue-next';

import type { ProjectItem } from '@/types/projects';

const props = defineProps<{ project: ProjectItem; compact?: boolean }>();
const router = useRouter();

const statusLabel = computed(() => props.project.status || 'Unknown');

function openProject() {
  router.push(`/projects/${encodeURIComponent(props.project.id)}`);
}
</script>

<template>
  <article class="project-card" :class="{ compact: compact }">
    <div class="project-card-top">
      <div class="project-icon-wrap">
        <FolderRootIcon class="project-icon" />
      </div>
      <div class="project-status">
        <BadgeCheckIcon class="status-icon" />
        <span>{{ statusLabel }}</span>
      </div>
    </div>

    <div class="project-card-body">
      <h3>{{ project.name }}</h3>
      <p>{{ project.notes || '暂无项目描述，后续可补充 Viewer / 工作台入口信息。' }}</p>
    </div>

    <dl class="project-meta">
      <div>
        <dt>环境</dt>
        <dd>{{ project.env || 'default' }}</dd>
      </div>
      <div>
        <dt>负责人</dt>
        <dd>{{ project.owner || '未填写' }}</dd>
      </div>
      <div>
        <dt>更新时间</dt>
        <dd>{{ project.updatedAt || '-' }}</dd>
      </div>
    </dl>

    <div class="project-card-footer">
      <span class="viewer-note">Viewer 入口预留中</span>
      <button type="button" class="ghost-button" @click="openProject">
        <span>进入工程</span>
        <ArrowRightIcon class="small-icon" />
      </button>
    </div>
  </article>
</template>
