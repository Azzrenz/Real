import { http } from '../http';
import { EP } from '../endpoints';
import type { McpApplyResult, McpListResult, McpServerConfig } from '../contracts';

export const mcpApi = {
  list: () => http.get<McpListResult>(EP.mcp.list()),
  save: (servers: McpServerConfig[]) =>
    http.put<{ ok: boolean; count?: number; error?: string }>(EP.mcp.save(), { servers }),
  apply: () => http.post<McpApplyResult>(EP.mcp.apply()),
};
