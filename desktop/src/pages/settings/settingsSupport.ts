import {
  AiModelRouteScope
} from "../../features/settings/settingsModel";
import type {
  CliRuntimeEnvironmentRecord,
  CliRuntimeToolRecord
} from "../../types";
import { type Advisor } from "../Advisors";
import {
  type FileIndexDashboard
} from "./SettingsSections";
import {
  stringifyEnvRecord,
  type AiModelOption,
  type RuntimePerfBenchmarkMode,
  type RuntimePerfTimelineItem
} from "./shared";

export const MIN_CHAT_MAX_TOKENS = 1024;

export const DEFAULT_CHAT_MAX_TOKENS = 262144;

export const DEFAULT_CHAT_MAX_TOKENS_DEEPSEEK = 131072;

export const DEVELOPER_MODE_UNLOCK_TAP_COUNT = 7;

export const DEVELOPER_MODE_TTL_MS = 24 * 60 * 60 * 1000;

export const SETTINGS_ACTIVATION_DEBOUNCE_MS = 80;

export const SETTINGS_TAB_POLL_DELAY_MS = 300;

export const FILE_INDEX_DASHBOARD_CACHE_KEY = 'redbox:file-index-dashboard:v1';

export const CHAT_FAMILY_ROUTE_SCOPES: AiModelRouteScope[] = ['chat', 'wander', 'team', 'knowledge', 'redclaw'];

export const ELEVENLABS_TRANSCRIPTION_MODELS: AiModelOption[] = [
  { id: 'scribe_v2', label: 'scribe_v2（推荐，通用）' },
  { id: 'scribe_v2_realtime', label: 'scribe_v2_realtime（实时场景）' },
  { id: 'scribe_v2_medical', label: 'scribe_v2_medical（医疗音频）' },
  { id: 'scribe_v1', label: 'scribe_v1（已废弃）' },
];

export type RedclawProfileDraft = {
  user: string;
  creatorProfile: string;
};

export type WorkspaceSpace = {
  id: string;
  name: string;
  createdAt?: string;
  updatedAt?: string;
};

export const EMPTY_REDCLAW_PROFILE_DRAFT: RedclawProfileDraft = {
  user: '',
  creatorProfile: '',
};

export const DEFAULT_SPACE_ID = 'default';

export function teamAdvisorOrder(advisor: Advisor, index: number): number {
  return Number.isFinite(advisor.redclawOrder) ? Number(advisor.redclawOrder) : index;
}

export function sortTeamAdvisors(advisors: Advisor[]): Advisor[] {
  return advisors
    .map((advisor, index) => ({ advisor, index }))
    .sort((left, right) => {
      const orderDelta = teamAdvisorOrder(left.advisor, left.index) - teamAdvisorOrder(right.advisor, right.index);
      return orderDelta || left.index - right.index;
    })
    .map(({ advisor }) => advisor);
}

export function advisorAvatarLabel(advisor: Advisor): string {
  return String(advisor.avatar || advisor.name || '成').trim().slice(0, 2);
}

export type AssistantDaemonStatus = Awaited<ReturnType<typeof window.ipcRenderer.assistantDaemon.getStatus>>;

export type AcpGatewayStatus = NonNullable<AssistantDaemonStatus['acpGateway']>;

export type AcpClientStatus = AcpGatewayStatus['clients'][number];

export type RuntimeDiagnosticsSummary = Awaited<ReturnType<typeof window.ipcRenderer.debug.getRuntimeSummary>>;

export type CliRuntimeInstallMethodOption = 'npm' | 'pnpm' | 'python' | 'uv' | 'cargo' | 'go' | 'binary';

export type CliRuntimeExecutionMode = 'managed' | 'host_compatible' | 'unrestricted';

export const normalizeCliRuntimeExecutionMode = (value: unknown): CliRuntimeExecutionMode => {
  const normalized = String(value || '').trim();
  if (normalized === 'managed' || normalized === 'host_compatible' || normalized === 'unrestricted') {
    return normalized;
  }
  return 'host_compatible';
};

export type CliRuntimeInstallQueueItem = {
  installId: string;
  toolName: string;
  environmentId?: string;
  installMethod?: string;
  spec?: string;
  status: string;
  summary?: string;
  updatedAt: number;
};

export type AssistantDaemonDraft = {
  enabled: boolean;
  autoStart: boolean;
  keepAliveWhenNoWindow: boolean;
  host: string;
  port: string;
  feishu: {
    enabled: boolean;
    receiveMode: 'webhook' | 'websocket';
    endpointPath: string;
    verificationToken: string;
    encryptKey: string;
    appId: string;
    appSecret: string;
    replyUsingChatId: boolean;
  };
  relay: {
    enabled: boolean;
    endpointPath: string;
    authToken: string;
  };
  acpGateway: {
    enabled: boolean;
    requireToken: boolean;
    localOnly: boolean;
    endpointPath: string;
    manifestPath: string;
    guidePath: string;
    defaultRuntimeMode: string;
    defaultClientLabel: string;
  };
  weixin: {
    enabled: boolean;
    endpointPath: string;
    authToken: string;
    accountId: string;
    autoStartSidecar: boolean;
    cursorFile: string;
    sidecarCommand: string;
    sidecarArgs: string;
    sidecarCwd: string;
    sidecarEnvText: string;
  };
};

export type AssistantDaemonWeixinLoginState = {
  sessionKey?: string;
  qrcodeUrl?: string;
  qrcodeImageUrl?: string;
  message: string;
  accountId?: string;
  userId?: string;
  connected: boolean;
  stateDir?: string;
};

export const createDefaultAssistantDaemonDraft = (): AssistantDaemonDraft => ({
  enabled: true,
  autoStart: true,
  keepAliveWhenNoWindow: true,
  host: '127.0.0.1',
  port: '31937',
  feishu: {
    enabled: false,
    receiveMode: 'webhook',
    endpointPath: '/hooks/feishu/events',
    verificationToken: '',
    encryptKey: '',
    appId: '',
    appSecret: '',
    replyUsingChatId: true,
  },
  relay: {
    enabled: true,
    endpointPath: '/hooks/channel/relay',
    authToken: '',
  },
  acpGateway: {
    enabled: false,
    requireToken: false,
    localOnly: true,
    endpointPath: '/acp/v1',
    manifestPath: '/acp/v1/manifest',
    guidePath: '/acp/v1/guide',
    defaultRuntimeMode: 'default',
    defaultClientLabel: 'External Agent',
  },
  weixin: {
    enabled: false,
    endpointPath: '/hooks/weixin/relay',
    authToken: '',
    accountId: '',
    autoStartSidecar: false,
    cursorFile: '',
    sidecarCommand: '',
    sidecarArgs: '',
    sidecarCwd: '',
    sidecarEnvText: '',
  },
});

export const assistantDaemonStatusToDraft = (status?: AssistantDaemonStatus | null): AssistantDaemonDraft => {
  if (!status) return createDefaultAssistantDaemonDraft();
  return {
    enabled: Boolean(status.enabled),
    autoStart: Boolean(status.autoStart),
    keepAliveWhenNoWindow: Boolean(status.keepAliveWhenNoWindow),
    host: String(status.host || '127.0.0.1'),
    port: String(status.port || 31937),
    feishu: {
      enabled: Boolean(status.feishu?.enabled),
      receiveMode: status.feishu?.receiveMode === 'websocket' ? 'websocket' : 'webhook',
      endpointPath: String(status.feishu?.endpointPath || '/hooks/feishu/events'),
      verificationToken: String(status.feishu?.verificationToken || ''),
      encryptKey: String(status.feishu?.encryptKey || ''),
      appId: String(status.feishu?.appId || ''),
      appSecret: String(status.feishu?.appSecret || ''),
      replyUsingChatId: status.feishu?.replyUsingChatId !== false,
    },
    relay: {
      enabled: status.relay?.enabled !== false,
      endpointPath: String(status.relay?.endpointPath || '/hooks/channel/relay'),
      authToken: String(status.relay?.authToken || ''),
    },
    acpGateway: {
      enabled: Boolean(status.acpGateway?.enabled),
      requireToken: Boolean(status.acpGateway?.requireToken),
      localOnly: status.acpGateway?.localOnly !== false,
      endpointPath: String(status.acpGateway?.endpointPath || '/acp/v1'),
      manifestPath: String(status.acpGateway?.manifestPath || '/acp/v1/manifest'),
      guidePath: String(status.acpGateway?.guidePath || '/acp/v1/guide'),
      defaultRuntimeMode: String(status.acpGateway?.defaultRuntimeMode || 'default'),
      defaultClientLabel: String(status.acpGateway?.defaultClientLabel || 'External Agent'),
    },
    weixin: {
      enabled: Boolean(status.weixin?.enabled),
      endpointPath: String(status.weixin?.endpointPath || '/hooks/weixin/relay'),
      authToken: String(status.weixin?.authToken || ''),
      accountId: String(status.weixin?.accountId || ''),
      autoStartSidecar: Boolean(status.weixin?.autoStartSidecar),
      cursorFile: String(status.weixin?.cursorFile || ''),
      sidecarCommand: String(status.weixin?.sidecarCommand || ''),
      sidecarArgs: Array.isArray(status.weixin?.sidecarArgs) ? status.weixin.sidecarArgs.join(' ') : '',
      sidecarCwd: String(status.weixin?.sidecarCwd || ''),
      sidecarEnvText: stringifyEnvRecord(status.weixin?.sidecarEnv || {}),
    },
  };
};

export type RuntimeSessionListItem = {
  id: string;
  runtimeMode?: string;
  contextBinding?: {
    contextType?: string;
    contextId?: string;
    isContextBound?: boolean;
  } | null;
  transcriptCount: number;
  checkpointCount: number;
  metadata?: Record<string, unknown> | null;
  chatSession?: {
    id: string;
    title?: string;
    updatedAt?: string;
  } | null;
};

export type RuntimeSessionTranscriptItem = {
  id: number;
  sessionId: string;
  recordType: string;
  role: string;
  content: string;
  payload?: unknown;
  createdAt: number;
};

export type RuntimeSessionCheckpointItem = {
  id: string;
  sessionId: string;
  checkpointType: string;
  summary: string;
  payload?: unknown;
  createdAt: number;
};

export type RuntimeSessionToolResultItem = {
  id: string;
  sessionId: string;
  callId: string;
  toolName: string;
  command?: string;
  success: boolean;
  resultText?: string;
  summaryText?: string;
  promptText?: string;
  originalChars?: number;
  promptChars?: number;
  truncated: boolean;
  payload?: unknown;
  createdAt: number;
  updatedAt: number;
};

export type RuntimeHookDefinition = {
  id: string;
  event: string;
  type: string;
  matcher?: string;
  enabled?: boolean;
  sourceScope?: string;
  pluginId?: string;
  pluginRoot?: string;
  pluginDataRoot?: string;
  sourcePath?: string;
  sourceRelativePath?: string;
  command?: string;
  commandWindows?: string;
  timeoutSec?: number;
  async?: boolean;
  statusMessage?: string;
  raw?: unknown;
};

export type RuntimePerfCollector = {
  runId: string;
  sessionId: string;
  startedAt: number;
  thinkingStartedMs?: number;
  thoughtFirstTokenMs?: number;
  firstResponseMs?: number;
  firstToolStartMs?: number;
  firstCheckpointMs?: number;
  toolCalls: number;
  toolSuccessCount: number;
  toolFailureCount: number;
  checkpointCount: number;
  checkpointTypes: string[];
  responseChars?: number;
  timeline: RuntimePerfTimelineItem[];
};

export function toRuntimePerfRecord(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object') return {};
  return value as Record<string, unknown>;
}

export function toRuntimePerfText(value: unknown): string {
  return String(value || '').trim();
}

export function toRuntimePerfNumber(value: unknown): number | undefined {
  if (typeof value === 'number' && Number.isFinite(value)) {
    return value;
  }
  if (typeof value === 'string' && value.trim()) {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) {
      return parsed;
    }
  }
  return undefined;
}

export function normalizeCliRuntimeToolRecord(value: unknown): CliRuntimeToolRecord | null {
  if (!value || typeof value !== 'object') return null;
  const record = value as Record<string, unknown>;
  const id = String(record.id || record.toolId || '').trim();
  const executable = String(record.executable || record.command || '').trim();
  const name = String(record.name || executable || id).trim();
  if (!id && !name && !executable) return null;
  return {
    id: id || name || executable,
    name: name || executable || id,
    executable: executable || name || id,
    resolvedPath: String(record.resolvedPath || record.resolved_path || '').trim() || null,
    resolvedFrom: String(record.resolvedFrom || record.resolved_from || '').trim().toLowerCase() as CliRuntimeToolRecord['resolvedFrom'],
    source: String(record.source || 'unknown').trim().toLowerCase() as CliRuntimeToolRecord['source'],
    installMethod: String(record.installMethod || record.install_method || '').trim() || null,
    installSpec: String(record.installSpec || record.install_spec || '').trim() || null,
    version: String(record.version || '').trim() || null,
    health: String(record.health || 'unknown').trim().toLowerCase() as CliRuntimeToolRecord['health'],
    manifestId: String(record.manifestId || record.manifest_id || '').trim() || null,
    environmentId: String(record.environmentId || record.environment_id || '').trim() || null,
    lastCheckedAt: toRuntimePerfNumber(record.lastCheckedAt) ?? null,
    effectivePathPreview: Array.isArray(record.effectivePathPreview)
      ? record.effectivePathPreview.map((item) => String(item || '').trim()).filter(Boolean)
      : Array.isArray(record.effective_path_preview)
        ? (record.effective_path_preview as unknown[]).map((item) => String(item || '').trim()).filter(Boolean)
        : [],
    searchedPathEntriesCount:
      toRuntimePerfNumber(record.searchedPathEntriesCount)
      ?? toRuntimePerfNumber(record.searched_path_entries_count)
      ?? null,
    isInDefaultDetectCatalog:
      record.isInDefaultDetectCatalog === true || record.is_in_default_detect_catalog === true,
    metadata: record.metadata && typeof record.metadata === 'object' ? record.metadata as Record<string, unknown> : null,
  };
}

export function normalizeCliRuntimeEnvironmentRecord(value: unknown): CliRuntimeEnvironmentRecord | null {
  if (!value || typeof value !== 'object') return null;
  const record = value as Record<string, unknown>;
  const id = String(record.id || '').trim();
  const rootPath = String(record.rootPath || record.root_path || '').trim();
  if (!id && !rootPath) return null;
  return {
    id: id || rootPath,
    scope: String(record.scope || 'workspace-local').trim().toLowerCase() as CliRuntimeEnvironmentRecord['scope'],
    rootPath,
    workspaceRoot: String(record.workspaceRoot || record.workspace_root || '').trim() || null,
    pathEntries: Array.isArray(record.pathEntries)
      ? record.pathEntries.map((item) => String(item || '').trim()).filter(Boolean)
      : [],
    installedToolIds: Array.isArray(record.installedToolIds)
      ? record.installedToolIds.map((item) => String(item || '').trim()).filter(Boolean)
      : [],
    runtimes: record.runtimes && typeof record.runtimes === 'object' ? record.runtimes as Record<string, unknown> : null,
    createdAt: toRuntimePerfNumber(record.createdAt) ?? null,
    updatedAt: toRuntimePerfNumber(record.updatedAt) ?? null,
    metadata: record.metadata && typeof record.metadata === 'object' ? record.metadata as Record<string, unknown> : null,
  };
}

export function runtimePerfContextTypeForMode(mode: RuntimePerfBenchmarkMode): string {
  if (mode === 'team') return 'team';
  if (mode === 'diagnostics') return 'diagnostics';
  return mode;
}

export function formatRuntimePerfRunIndex(index: number): string {
  return `Run ${String(index).padStart(2, '0')}`;
}

export const sanitizeChatMaxTokensInput = (value: string, fallback: number): string => {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed < MIN_CHAT_MAX_TOKENS) {
    return String(fallback);
  }
  return String(Math.floor(parsed));
};

export type FileIndexDashboardCacheRecord = {
  savedAt: number;
  dashboard: FileIndexDashboard;
};

export function readCachedFileIndexDashboard(): FileIndexDashboardCacheRecord | null {
  if (typeof window === 'undefined') return null;
  try {
    const raw = window.localStorage.getItem(FILE_INDEX_DASHBOARD_CACHE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<FileIndexDashboardCacheRecord>;
    if (!parsed || typeof parsed.savedAt !== 'number' || !parsed.dashboard) return null;
    return {
      savedAt: parsed.savedAt,
      dashboard: parsed.dashboard,
    };
  } catch (error) {
    console.warn('Failed to read cached file index dashboard:', error);
    return null;
  }
}

export function writeCachedFileIndexDashboard(dashboard: FileIndexDashboard): number {
  const savedAt = Date.now();
  if (typeof window === 'undefined') return savedAt;
  try {
    window.localStorage.setItem(
      FILE_INDEX_DASHBOARD_CACHE_KEY,
      JSON.stringify({ savedAt, dashboard }),
    );
  } catch (error) {
    console.warn('Failed to cache file index dashboard:', error);
  }
  return savedAt;
}

export function clearCachedFileIndexDashboard() {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.removeItem(FILE_INDEX_DASHBOARD_CACHE_KEY);
  } catch (error) {
    console.warn('Failed to clear cached file index dashboard:', error);
  }
}
