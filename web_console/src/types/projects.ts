export interface ProjectItem {
  id: string;
  name: string;
  env?: string;
  owner?: string;
  notes?: string;
  updatedAt?: string;
  healthUrl?: string;
  url?: string;
  status?: string;
}

export interface ProjectsResponse {
  items: ProjectItem[];
  total: number;
  page: number;
  per_page: number;
}
