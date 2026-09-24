
import { http } from '../http';

export interface SkillEntry {
  name: string;
  description: string;
  category?: string;
  outline?: string[];
  /** Body lead paragraph (between the H1 and the first section), for the detail card. */
  summary?: string;
}

export const skillsApi = {
  list: () => http.get<{ skills: SkillEntry[] }>('/api/skills'),

  create: (name: string, description: string, prompt: string, category?: string) =>
    http.post<{ ok: boolean; name: string }>('/api/skills', {
      name,
      description,
      prompt,
      category: category?.trim() || undefined,
    }),

  remove: (name: string) =>
    http.delete<{ ok: boolean }>(`/api/skills/${encodeURIComponent(name)}`),

  rename: (name: string, newName: string) =>
    http.post<{ ok: boolean; name: string }>(
      `/api/skills/${encodeURIComponent(name)}/rename`,
      { new_name: newName },
    ),

  /** Raw SKILL.md text, for the standalone detail window. */
  raw: (name: string) =>
    http.get<{ name: string; text: string }>(`/api/skills/${encodeURIComponent(name)}/raw`),

  openDir: (name: string) =>
    http.post<{ ok: boolean; path: string }>(`/api/skills/${encodeURIComponent(name)}/open`, {}),

  absorb: (p: { name: string; summary: string; evidence: string; category?: string; merge_into?: string }) =>
    http.post<{ ok: boolean; merged_into?: string; created?: string }>('/api/skills/absorb', {
      name: p.name,
      description: p.summary,
      prompt: `${p.summary}\n\n> 证据：${p.evidence}`,
      category: p.category?.trim() || undefined,
      merge_into: p.merge_into?.trim() || undefined,
    }),
};
