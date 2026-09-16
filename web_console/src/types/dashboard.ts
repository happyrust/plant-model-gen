export interface StatusMetrics {
  cpuUsage: number;
  memoryUsage: number;
  activeTaskCount: number;
  queuedTaskCount: number;
  databaseConnected?: boolean;
  surrealdbConnected?: boolean;
  uptimeSeconds?: number;
}

export interface MetricCard {
  id: string;
  label: string;
  value: string;
  hint: string;
  tone?: 'primary' | 'success' | 'warning' | 'danger';
  trend?: string;
}

export interface DashboardActivityItem {
  id: string;
  source: 'review' | 'task' | string;
  userId: string;
  userName: string;
  userType: 'human' | 'system_bot' | string;
  actionTitle: string;
  targetName: string;
  actionDesc: string;
  createdAt: string;
}

export interface DashboardTaskOverview {
  activeCount: number;
  queuedCount: number;
  healthLabel: string;
  healthDetail: string;
}
