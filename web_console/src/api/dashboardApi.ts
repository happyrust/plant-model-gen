import { fetchJson } from '@/api/http';
import type { DashboardActivityItem, StatusMetrics } from '@/types/dashboard';

interface RawStatusResponse {
  cpu_usage: number;
  memory_usage: number;
  active_tasks: number;
  queued_task_count?: number;
  database_connected?: boolean;
  surrealdb_connected?: boolean;
  uptime?: number | { secs?: number };
}

interface ActivitiesResponse {
  success: boolean;
  data: DashboardActivityItem[];
}

export async function getStatusMetrics(): Promise<StatusMetrics> {
  const raw = await fetchJson<RawStatusResponse>('/api/status');
  return {
    cpuUsage: raw.cpu_usage,
    memoryUsage: raw.memory_usage,
    activeTaskCount: raw.active_tasks,
    queuedTaskCount: raw.queued_task_count ?? 0,
    databaseConnected: raw.database_connected,
    surrealdbConnected: raw.surrealdb_connected,
    uptimeSeconds: typeof raw.uptime === 'number' ? raw.uptime : raw.uptime?.secs,
  };
}

export async function getDashboardActivities(limit = 8): Promise<DashboardActivityItem[]> {
  const response = await fetchJson<ActivitiesResponse>(`/api/dashboard/activities?limit=${limit}`);
  return Array.isArray(response.data) ? response.data : [];
}
