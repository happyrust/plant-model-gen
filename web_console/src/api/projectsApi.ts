import { fetchJson } from '@/api/http';
import type { ProjectItem, ProjectsResponse } from '@/types/projects';

interface RawProjectItem {
  id?: string;
  name?: string;
  env?: string;
  owner?: string;
  notes?: string;
  updated_at?: string;
  health_url?: string;
  url?: string;
  status?: string;
}

function normalizeProject(item: RawProjectItem): ProjectItem {
  return {
    id: String(item.id || item.name || ''),
    name: String(item.name || ''),
    env: item.env,
    owner: item.owner,
    notes: item.notes,
    updatedAt: item.updated_at,
    healthUrl: item.health_url,
    url: item.url,
    status: item.status,
  };
}

export async function getProjects(): Promise<ProjectItem[]> {
  const raw = await fetchJson<ProjectsResponse | { items?: RawProjectItem[] }>('/api/projects');
  const items = Array.isArray((raw as ProjectsResponse).items) ? (raw as ProjectsResponse).items : [];
  return items.map((item) => normalizeProject(item as RawProjectItem));
}
