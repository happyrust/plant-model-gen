function joinBase(path: string): string {
  const base = (import.meta.env.BASE_URL || '/console/').replace(/\/$/, '');
  if (/^https?:\/\//.test(path)) return path;
  if (path.startsWith('/')) return path;
  return `${base}/${path}`;
}

export async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(joinBase(path), {
    ...init,
    headers: {
      'Content-Type': 'application/json',
      ...(init?.headers || {}),
    },
  });

  if (!response.ok) {
    const text = await response.text().catch(() => '');
    throw new Error(text || `HTTP ${response.status}`);
  }

  return (await response.json()) as T;
}
