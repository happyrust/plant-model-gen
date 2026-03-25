import { computed, ref } from 'vue';

import { getProjects } from '@/api/projectsApi';
import type { ProjectItem } from '@/types/projects';

const projects = ref<ProjectItem[]>([]);
const isLoading = ref(false);
const errorMessage = ref('');
const currentProjectId = ref('');

export function useProjects() {
  async function loadProjects() {
    if (isLoading.value) return;
    isLoading.value = true;
    errorMessage.value = '';
    try {
      const items = await getProjects();
      projects.value = items;
      if (!currentProjectId.value && items.length > 0) {
        currentProjectId.value = items[0].id;
      }
    } catch (error) {
      errorMessage.value = error instanceof Error ? error.message : String(error);
    } finally {
      isLoading.value = false;
    }
  }

  function selectProject(projectId: string) {
    currentProjectId.value = projectId;
  }

  const currentProject = computed(() =>
    projects.value.find((item) => item.id === currentProjectId.value) || null,
  );

  return {
    projects: computed(() => projects.value),
    isLoading: computed(() => isLoading.value),
    errorMessage: computed(() => errorMessage.value),
    currentProject,
    loadProjects,
    selectProject,
  };
}
