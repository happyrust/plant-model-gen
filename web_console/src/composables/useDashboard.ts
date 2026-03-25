import { computed, ref } from 'vue';

import { getDashboardActivities, getStatusMetrics } from '@/api/dashboardApi';
import { getProjects } from '@/api/projectsApi';
import type { DashboardActivityItem, MetricCard } from '@/types/dashboard';
import type { ProjectItem } from '@/types/projects';

const metricCards = ref<MetricCard[]>([]);
const activities = ref<DashboardActivityItem[]>([]);
const recentProjects = ref<ProjectItem[]>([]);
const loading = ref(false);
const errorMessage = ref('');
const lastUpdatedAt = ref('');

function formatRelativeTimestamp(value?: string): string {
  if (!value) return '刚刚同步';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function buildMetricCards(status: Awaited<ReturnType<typeof getStatusMetrics>>): MetricCard[] {
  return [
    {
      id: 'active-tasks',
      label: '活跃任务',
      value: String(status.activeTaskCount),
      hint: `排队 ${status.queuedTaskCount} 个任务`,
    },
    {
      id: 'cpu',
      label: 'CPU 使用率',
      value: `${Math.round(status.cpuUsage)}%`,
      hint: '来自 /api/status 实时指标',
    },
    {
      id: 'memory',
      label: '内存使用率',
      value: `${Math.round(status.memoryUsage)}%`,
      hint: status.databaseConnected ? '数据库连接正常' : '数据库连接待确认',
    },
    {
      id: 'uptime',
      label: '服务运行时长',
      value: status.uptimeSeconds ? `${Math.floor(status.uptimeSeconds / 60)} 分钟` : '-',
      hint: status.surrealdbConnected ? 'SurrealDB 已连接' : 'SurrealDB 连接待确认',
    },
  ];
}

export function useDashboard() {
  async function refresh() {
    if (loading.value) return;
    loading.value = true;
    errorMessage.value = '';
    try {
      const [status, dashboardActivities, projects] = await Promise.all([
        getStatusMetrics(),
        getDashboardActivities(8),
        getProjects(),
      ]);

      metricCards.value = buildMetricCards(status);
      activities.value = dashboardActivities;
      recentProjects.value = projects.slice(0, 6);
      lastUpdatedAt.value = new Date().toLocaleString();
    } catch (error) {
      errorMessage.value = error instanceof Error ? error.message : String(error);
    } finally {
      loading.value = false;
    }
  }

  return {
    metricCards: computed(() => metricCards.value),
    activities: computed(() => activities.value),
    recentProjects: computed(() => recentProjects.value),
    loading: computed(() => loading.value),
    errorMessage: computed(() => errorMessage.value),
    lastUpdatedLabel: computed(() => formatRelativeTimestamp(lastUpdatedAt.value)),
    refresh,
  };
}
