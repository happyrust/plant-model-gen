import { computed, ref } from 'vue';

import { getDashboardActivities, getStatusMetrics } from '@/api/dashboardApi';
import { getProjects } from '@/api/projectsApi';
import type { DashboardActivityItem, DashboardTaskOverview, MetricCard } from '@/types/dashboard';
import type { ProjectItem } from '@/types/projects';

const metricCards = ref<MetricCard[]>([]);
const activities = ref<DashboardActivityItem[]>([]);
const recentProjects = ref<ProjectItem[]>([]);
const taskOverview = ref<DashboardTaskOverview>({
  activeCount: 0,
  queuedCount: 0,
  healthLabel: '等待同步',
  healthDetail: '点击刷新数据拉取 `/api/status`、`/api/dashboard/activities`、`/api/projects`。',
});
const loading = ref(false);
const errorMessage = ref('');
const lastUpdatedAt = ref('');

function formatRelativeTimestamp(value?: string): string {
  if (!value) return '尚未同步';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString();
}

function formatPercent(value?: number): string {
  if (!Number.isFinite(value)) return '-';
  return `${Math.round(value || 0)}%`;
}

function formatUptime(seconds?: number): string {
  if (!seconds || seconds <= 0) return '-';
  const totalMinutes = Math.floor(seconds / 60);
  const days = Math.floor(totalMinutes / (60 * 24));
  const hours = Math.floor((totalMinutes % (60 * 24)) / 60);
  const minutes = totalMinutes % 60;

  if (days > 0) return `${days} 天 ${hours} 小时`;
  if (hours > 0) return `${hours} 小时 ${minutes} 分钟`;
  return `${minutes} 分钟`;
}

function buildMetricCards(status: Awaited<ReturnType<typeof getStatusMetrics>>): MetricCard[] {
  const runtimeHealthy = status.databaseConnected && status.surrealdbConnected;

  return [
    {
      id: 'active-tasks',
      label: '活跃任务',
      value: String(status.activeTaskCount),
      hint: `排队 ${status.queuedTaskCount} 个任务`,
      tone: status.activeTaskCount > 0 ? 'primary' : 'success',
      trend: status.activeTaskCount > 0 ? '执行中' : '空闲中',
    },
    {
      id: 'cpu',
      label: 'CPU 使用率',
      value: formatPercent(status.cpuUsage),
      hint: '来自 /api/status 实时指标',
      tone: status.cpuUsage >= 80 ? 'danger' : status.cpuUsage >= 60 ? 'warning' : 'primary',
      trend: status.cpuUsage >= 60 ? '负载偏高' : '负载平稳',
    },
    {
      id: 'memory',
      label: '内存使用率',
      value: formatPercent(status.memoryUsage),
      hint: status.databaseConnected ? '数据库连接正常' : '数据库连接待确认',
      tone: status.memoryUsage >= 80 ? 'danger' : status.memoryUsage >= 60 ? 'warning' : 'success',
      trend: status.databaseConnected ? '数据库在线' : '数据库检查中',
    },
    {
      id: 'uptime',
      label: '服务运行时长',
      value: formatUptime(status.uptimeSeconds),
      hint: runtimeHealthy ? '数据库 / SurrealDB 都已连接' : '仍需确认运行环境连通性',
      tone: runtimeHealthy ? 'success' : 'warning',
      trend: runtimeHealthy ? '运行稳定' : '需要关注',
    },
  ];
}

function buildTaskOverview(status: Awaited<ReturnType<typeof getStatusMetrics>>): DashboardTaskOverview {
  const allConnected = status.databaseConnected && status.surrealdbConnected;

  return {
    activeCount: status.activeTaskCount,
    queuedCount: status.queuedTaskCount,
    healthLabel: allConnected ? '服务运行稳定' : '连接状态待确认',
    healthDetail: allConnected
      ? '任务中心和数据库都返回正常，可继续进入项目或任务页面。'
      : '当前至少有一项后端连接未确认，请优先查看系统状态卡或再次刷新。',
  };
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
      taskOverview.value = buildTaskOverview(status);
      activities.value = dashboardActivities;
      recentProjects.value = projects.slice(0, 6);
      lastUpdatedAt.value = new Date().toISOString();
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
    taskOverview: computed(() => taskOverview.value),
    loading: computed(() => loading.value),
    errorMessage: computed(() => errorMessage.value),
    lastUpdatedLabel: computed(() => formatRelativeTimestamp(lastUpdatedAt.value)),
    refresh,
  };
}
