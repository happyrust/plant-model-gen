import { fetchJson } from '@/api/http';

export type LegacyEnvelope<T> = {
  status: 'success' | 'error';
  message?: string;
  data?: T | null;
};

export type SpatialStatsResult = {
  success: boolean;
  total_elements: number;
  index_type: string;
  index_path: string;
  error?: string;
};

export type FittingRequest = {
  dbnum?: number;
  suppo_refno: string;
  tolerance?: number;
};

export type WallDistanceRequest = {
  dbnum?: number;
  suppo_refno: string;
  suppo_type?: string;
  search_radius?: number;
  target_nouns?: string[];
};

export type SteelRelativeRequest = {
  dbnum?: number;
  suppo_refno: string;
  suppo_type?: string;
  search_radius?: number;
};

export type TraySpanRequest = {
  dbnum?: number;
  suppo_refno: string;
  neighbor_window?: number;
};

export async function querySpatialStats(): Promise<SpatialStatsResult> {
  return fetchJson<SpatialStatsResult>('/api/sqlite-spatial/stats');
}

export async function postFitting(request: FittingRequest) {
  return fetchJson<LegacyEnvelope<unknown>>('/api/space/fitting', {
    method: 'POST',
    body: JSON.stringify(request),
  });
}

export async function postFittingOffset(request: FittingRequest) {
  return fetchJson<LegacyEnvelope<unknown>>('/api/space/fitting-offset', {
    method: 'POST',
    body: JSON.stringify(request),
  });
}

export async function postWallDistance(request: WallDistanceRequest) {
  return fetchJson<LegacyEnvelope<unknown>>('/api/space/wall-distance', {
    method: 'POST',
    body: JSON.stringify(request),
  });
}

export async function postSuppoTrays(request: FittingRequest) {
  return fetchJson<LegacyEnvelope<unknown>>('/api/space/suppo-trays', {
    method: 'POST',
    body: JSON.stringify(request),
  });
}

export async function postSteelRelative(request: SteelRelativeRequest) {
  return fetchJson<LegacyEnvelope<unknown>>('/api/space/steel-relative', {
    method: 'POST',
    body: JSON.stringify(request),
  });
}

export async function postTraySpan(request: TraySpanRequest) {
  return fetchJson<LegacyEnvelope<unknown>>('/api/space/tray-span', {
    method: 'POST',
    body: JSON.stringify(request),
  });
}
