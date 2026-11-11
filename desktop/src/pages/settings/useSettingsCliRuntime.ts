import type { Dispatch } from 'react';
import { useCallback, useEffect, useState } from 'react';
import { subscribeRuntimeEventStream } from "../../runtime/runtimeEventStream";
import type {
  CliRuntimeEnvironmentRecord,
  CliRuntimeEnvironmentScope,
  CliRuntimeToolRecord
} from "../../types";
import {
  CliRuntimeExecutionMode,
  CliRuntimeInstallMethodOption,
  CliRuntimeInstallQueueItem,
  normalizeCliRuntimeEnvironmentRecord,
  normalizeCliRuntimeExecutionMode,
  normalizeCliRuntimeToolRecord,
} from './settingsSupport';

export function useSettingsCliRuntime({ setFormData, formData }: {
  setFormData: Dispatch<any>;
  formData: any;
}) {

  const [cliRuntimeTools, setCliRuntimeTools] = useState<CliRuntimeToolRecord[]>([]);
  const [cliRuntimeEnvironments, setCliRuntimeEnvironments] = useState<CliRuntimeEnvironmentRecord[]>([]);
  const [cliRuntimeInstallDraft, setCliRuntimeInstallDraft] = useState<{
    environmentId: string;
    installMethod: CliRuntimeInstallMethodOption;
    spec: string;
    toolName: string;
  }>({
    environmentId: '',
    installMethod: 'pnpm',
    spec: '',
    toolName: '',
  });

  const [cliRuntimeInstallQueue, setCliRuntimeInstallQueue] = useState<CliRuntimeInstallQueueItem[]>([]);
  const [cliRuntimeInstalling, setCliRuntimeInstalling] = useState(false);
  const [cliRuntimeStatusMessage, setCliRuntimeStatusMessage] = useState('');
  const [isCliRuntimeRefreshing, setIsCliRuntimeRefreshing] = useState(false);
  const [cliRuntimeInspectingToolId, setCliRuntimeInspectingToolId] = useState('');
  const [cliRuntimeDiagnosticCommand, setCliRuntimeDiagnosticCommand] = useState('');
  const [cliRuntimeExecutionMode, setCliRuntimeExecutionMode] = useState<CliRuntimeExecutionMode>('host_compatible');
  const [cliRuntimeDiscoverQuery, setCliRuntimeDiscoverQuery] = useState('');
  const [cliRuntimeDiscoverResults, setCliRuntimeDiscoverResults] = useState<CliRuntimeToolRecord[]>([]);
  const [cliRuntimeDiscovering, setCliRuntimeDiscovering] = useState(false);
  const [cliRuntimeCreatingEnvironment, setCliRuntimeCreatingEnvironment] = useState<CliRuntimeEnvironmentScope | ''>('');

  useEffect(() => {
    if (!cliRuntimeStatusMessage) return;
    const timer = window.setTimeout(() => setCliRuntimeStatusMessage(''), 3200);
    return () => window.clearTimeout(timer);
  }, [cliRuntimeStatusMessage]);

  const handleCliRuntimeExecutionModeChange = useCallback((mode: CliRuntimeExecutionMode) => {
    const nextMode = normalizeCliRuntimeExecutionMode(mode);
    setCliRuntimeExecutionMode(nextMode);
    setFormData((prev) => ({
      ...prev,
      cli_runtime_execution_mode: nextMode,
    }));
    void window.ipcRenderer.saveSettings({
      cli_runtime_execution_mode: nextMode,
    }).then(() => {
      setCliRuntimeStatusMessage(
        nextMode === 'unrestricted'
          ? 'CLI Runtime 已切换为完全访问；仅对你明确授权的命令使用。'
          : nextMode === 'managed'
            ? 'CLI Runtime 已切换为安全模式。'
            : 'CLI Runtime 已切换为兼容模式。',
      );
    }).catch((error) => {
      console.error('Failed to persist CLI runtime execution mode', error);
      setCliRuntimeStatusMessage(`保存 CLI Runtime 模式失败：${String(error)}`);
    });
  }, []);

  const upsertCliRuntimeInstallQueueItem = useCallback((item: CliRuntimeInstallQueueItem) => {
    setCliRuntimeInstallQueue((prev) => {
      const next = prev.filter((entry) => entry.installId !== item.installId);
      next.unshift(item);
      return next
        .sort((left, right) => right.updatedAt - left.updatedAt)
        .slice(0, 8);
    });
  }, []);

  const loadCliRuntimeDashboard = useCallback(async (options?: { silent?: boolean }) => {
    if (!options?.silent) {
      setIsCliRuntimeRefreshing(true);
    }
    try {
      const [detectResult, environmentsResult] = await Promise.all([
        window.ipcRenderer.cliRuntime.detect(),
        window.ipcRenderer.cliRuntime.listEnvironments(),
      ]);
      const detectedToolsRaw = Array.isArray(detectResult)
        ? detectResult
        : Array.isArray((detectResult as { tools?: unknown[] } | null)?.tools)
          ? (detectResult as { tools?: unknown[] }).tools || []
          : [];
      const nextTools = detectedToolsRaw
        .map(normalizeCliRuntimeToolRecord)
        .filter((item): item is CliRuntimeToolRecord => Boolean(item))
        .sort((left, right) => left.name.localeCompare(right.name));
      const nextEnvironments = (Array.isArray(environmentsResult) ? environmentsResult : [])
        .map(normalizeCliRuntimeEnvironmentRecord)
        .filter((item): item is CliRuntimeEnvironmentRecord => Boolean(item))
        .sort((left, right) => left.scope.localeCompare(right.scope) || left.id.localeCompare(right.id));
      setCliRuntimeTools(nextTools);
      setCliRuntimeEnvironments(nextEnvironments);
      if (!options?.silent) {
        setCliRuntimeStatusMessage(`已刷新 CLI runtime：${nextTools.length} 个工具，${nextEnvironments.length} 个环境`);
      }
      setCliRuntimeInstallDraft((current) => {
        if (current.environmentId && nextEnvironments.some((item) => item.id === current.environmentId)) {
          return current;
        }
        const fallbackEnvironment = nextEnvironments[0]?.id || '';
        if (!fallbackEnvironment || fallbackEnvironment === current.environmentId) {
          return current;
        }
        return {
          ...current,
          environmentId: fallbackEnvironment,
        };
      });
    } catch (error) {
      console.error('Failed to load CLI runtime dashboard', error);
      if (!options?.silent) {
        setCliRuntimeStatusMessage(`刷新 CLI runtime 失败：${String(error)}`);
      }
    } finally {
      if (!options?.silent) {
        setIsCliRuntimeRefreshing(false);
      }
    }
  }, []);

  const handleInspectCliRuntimeTool = useCallback(async (toolId: string) => {
    const normalizedToolId = String(toolId || '').trim();
    if (!normalizedToolId) return;
    setCliRuntimeInspectingToolId(normalizedToolId);
    try {
      const result = await window.ipcRenderer.cliRuntime.inspect({ toolId: normalizedToolId });
      const normalized = normalizeCliRuntimeToolRecord(result);
      if (normalized) {
        setCliRuntimeTools((prev) => {
          const next = prev.map((item) => (item.id === normalizedToolId ? { ...item, ...normalized } : item));
          if (!next.some((item) => item.id === normalizedToolId)) {
            next.unshift(normalized);
          }
          return next.sort((left, right) => left.name.localeCompare(right.name));
        });
        setCliRuntimeStatusMessage(`已检查 ${normalized.name || normalized.executable}`);
      } else {
        setCliRuntimeStatusMessage(`未返回 ${normalizedToolId} 的 inspect 数据`);
      }
    } catch (error) {
      console.error('Failed to inspect CLI runtime tool', error);
      setCliRuntimeStatusMessage(`Inspect 失败：${String(error)}`);
    } finally {
      setCliRuntimeInspectingToolId('');
    }
  }, []);

  const handleDiagnoseCliRuntimeCommand = useCallback(async () => {
    const command = String(cliRuntimeDiagnosticCommand || '').trim();
    if (!command) {
      setCliRuntimeStatusMessage('请先输入要诊断的 CLI 命令名，例如 lark-cli');
      return;
    }
    setCliRuntimeInspectingToolId(command);
    try {
      const result = await window.ipcRenderer.cliRuntime.diagnose({
        command,
        executionMode: cliRuntimeExecutionMode,
      });
      const normalized = normalizeCliRuntimeToolRecord((result as { tool?: unknown } | null)?.tool);
      if (normalized) {
        setCliRuntimeTools((prev) => {
          const next = prev.filter((item) => item.id !== normalized.id);
          next.unshift(normalized);
          return next.sort((left, right) => left.name.localeCompare(right.name));
        });
        const sandbox = (result as { sandbox?: { mode?: string; backend?: string; allowNetwork?: boolean } } | null)?.sandbox;
        const summary = String((result as { summary?: string } | null)?.summary || '').trim();
        setCliRuntimeStatusMessage(
          normalized.resolvedPath
            ? `${summary || `已解析 ${command}`} · ${sandbox?.backend || 'runtime'} · ${normalized.resolvedPath}`
            : summary || `未在当前 PATH 中解析到 ${command}`,
        );
      } else {
        setCliRuntimeStatusMessage(`未返回 ${command} 的诊断数据`);
      }
    } catch (error) {
      console.error('Failed to diagnose CLI runtime command', error);
      setCliRuntimeStatusMessage(`诊断失败：${String(error)}`);
    } finally {
      setCliRuntimeInspectingToolId('');
    }
  }, [cliRuntimeDiagnosticCommand, cliRuntimeExecutionMode]);

  const handleDiscoverCliRuntimeTools = useCallback(async () => {
    setCliRuntimeDiscovering(true);
    try {
      const result = await window.ipcRenderer.cliRuntime.discover({
        query: String(cliRuntimeDiscoverQuery || '').trim() || undefined,
        limit: 80,
      });
      const discoveredToolsRaw = Array.isArray((result as { tools?: unknown[] } | null)?.tools)
        ? (result as { tools?: unknown[] }).tools || []
        : Array.isArray(result)
          ? result
          : [];
      const normalizedTools = discoveredToolsRaw
        .map(normalizeCliRuntimeToolRecord)
        .filter((item): item is CliRuntimeToolRecord => Boolean(item))
        .sort((left, right) => left.name.localeCompare(right.name));
      setCliRuntimeDiscoverResults(normalizedTools);
      setCliRuntimeStatusMessage(
        normalizedTools.length > 0
          ? `已搜索 PATH，命中 ${normalizedTools.length} 个 CLI`
          : '当前 PATH 搜索没有命中结果',
      );
    } catch (error) {
      console.error('Failed to discover CLI runtime tools', error);
      setCliRuntimeStatusMessage(`PATH 搜索失败：${String(error)}`);
    } finally {
      setCliRuntimeDiscovering(false);
    }
  }, [cliRuntimeDiscoverQuery]);

  const handleCreateCliRuntimeEnvironment = useCallback(async (scope: CliRuntimeEnvironmentScope) => {
    setCliRuntimeCreatingEnvironment(scope);
    try {
      const workspaceRoot = scope === 'workspace-local'
        ? String(formData.workspace_dir || '').trim() || undefined
        : undefined;
      const result = await window.ipcRenderer.cliRuntime.createEnvironment({ scope, workspaceRoot });
      const normalized = normalizeCliRuntimeEnvironmentRecord(result);
      if (normalized) {
        setCliRuntimeEnvironments((prev) => {
          const next = prev.filter((item) => item.id !== normalized.id);
          next.unshift(normalized);
          return next.sort((left, right) => left.scope.localeCompare(right.scope) || left.id.localeCompare(right.id));
        });
        setCliRuntimeStatusMessage(`已创建环境 ${normalized.id}`);
      } else if ((result as { success?: boolean; error?: string } | null)?.success === false) {
        setCliRuntimeStatusMessage((result as { error?: string }).error || '创建 CLI environment 失败');
      } else {
        await loadCliRuntimeDashboard({ silent: true });
        setCliRuntimeStatusMessage(`已触发环境创建：${scope}`);
      }
    } catch (error) {
      console.error('Failed to create CLI runtime environment', error);
      setCliRuntimeStatusMessage(`创建环境失败：${String(error)}`);
    } finally {
      setCliRuntimeCreatingEnvironment('');
    }
  }, [formData.workspace_dir, loadCliRuntimeDashboard]);

  const handleInstallCliRuntimeTool = useCallback(async () => {
    const environmentId = String(cliRuntimeInstallDraft.environmentId || '').trim()
      || cliRuntimeEnvironments[0]?.id
      || '';
    const spec = String(cliRuntimeInstallDraft.spec || '').trim();
    const toolName = String(cliRuntimeInstallDraft.toolName || '').trim();
    if (!environmentId) {
      setCliRuntimeStatusMessage('请先选择一个 CLI environment');
      return;
    }
    if (!spec) {
      setCliRuntimeStatusMessage('请填写要安装的 spec，例如 ffmpeg-static 或 @scope/tool');
      return;
    }
    setCliRuntimeInstalling(true);
    try {
      const result = await window.ipcRenderer.cliRuntime.install({
        environmentId,
        installMethod: cliRuntimeInstallDraft.installMethod,
        spec,
        toolName: toolName || undefined,
        executionMode: cliRuntimeExecutionMode,
      });
      const installId = String((result as { installId?: string } | null)?.installId || '').trim();
      if (installId) {
        upsertCliRuntimeInstallQueueItem({
          installId,
          toolName: String((result as { toolName?: string } | null)?.toolName || toolName || spec),
          environmentId,
          installMethod: cliRuntimeInstallDraft.installMethod,
          spec,
          status: String((result as { status?: string } | null)?.status || 'queued'),
          summary: String((result as { summary?: string } | null)?.summary || ''),
          updatedAt: Date.now(),
        });
      }
      await loadCliRuntimeDashboard({ silent: true });
      setCliRuntimeStatusMessage(
        String((result as { summary?: string } | null)?.summary || `已触发安装：${toolName || spec}`),
      );
      setCliRuntimeInstallDraft((current) => ({
        ...current,
        toolName: '',
        spec: '',
        environmentId,
      }));
    } catch (error) {
      console.error('Failed to install CLI runtime tool', error);
      setCliRuntimeStatusMessage(`安装失败：${String(error)}`);
    } finally {
      setCliRuntimeInstalling(false);
    }
  }, [
    cliRuntimeEnvironments,
    cliRuntimeInstallDraft,
    loadCliRuntimeDashboard,
    upsertCliRuntimeInstallQueueItem,
  ]);

  const handleOpenCliRuntimeEnvironmentRoot = useCallback(async (rootPath: string) => {
    const normalizedPath = String(rootPath || '').trim();
    if (!normalizedPath) return;
    try {
      const result = await window.ipcRenderer.openPath(normalizedPath);
      if (!result?.success) {
        throw new Error(result?.error || '打开目录失败');
      }
    } catch (error) {
      console.error('Failed to open CLI runtime environment root', error);
      setCliRuntimeStatusMessage(`打开目录失败：${String(error)}`);
    }
  }, []);

  useEffect(() => subscribeRuntimeEventStream({
    eventTypes: [
      'runtime:cli-install-started',
      'runtime:cli-install-finished',
    ],
    onCliInstallStarted: ({
      installId,
      toolName,
      environmentId,
      installMethod,
      spec,
    }) => {
      const normalizedInstallId = String(installId || '').trim();
      if (!normalizedInstallId) return;
      upsertCliRuntimeInstallQueueItem({
        installId: normalizedInstallId,
        toolName,
        environmentId,
        installMethod,
        spec,
        status: 'running',
        summary: `正在安装 ${toolName}`,
        updatedAt: Date.now(),
      });
    },
    onCliInstallFinished: ({
      installId,
      toolName,
      environmentId,
      status,
      summary,
      raw,
    }) => {
      const normalizedInstallId = String(installId || '').trim();
      if (!normalizedInstallId) return;
      upsertCliRuntimeInstallQueueItem({
        installId: normalizedInstallId,
        toolName,
        environmentId,
        installMethod: typeof raw.installMethod === 'string' ? raw.installMethod : undefined,
        spec: typeof raw.spec === 'string' ? raw.spec : undefined,
        status,
        summary,
        updatedAt: Date.now(),
      });
      void loadCliRuntimeDashboard({ silent: true });
    },
  }), [loadCliRuntimeDashboard, upsertCliRuntimeInstallQueueItem]);
  return {
    setCliRuntimeExecutionMode,
    loadCliRuntimeDashboard,
    cliRuntimeTools,
    cliRuntimeEnvironments,
    cliRuntimeInstallDraft,
    setCliRuntimeInstallDraft,
    cliRuntimeInstallQueue,
    cliRuntimeStatusMessage,
    isCliRuntimeRefreshing,
    cliRuntimeInstalling,
    cliRuntimeInspectingToolId,
    cliRuntimeDiagnosticCommand,
    setCliRuntimeDiagnosticCommand,
    cliRuntimeExecutionMode,
    handleCliRuntimeExecutionModeChange,
    cliRuntimeDiscoverQuery,
    setCliRuntimeDiscoverQuery,
    cliRuntimeDiscoverResults,
    cliRuntimeDiscovering,
    cliRuntimeCreatingEnvironment,
    handleInspectCliRuntimeTool,
    handleDiagnoseCliRuntimeCommand,
    handleDiscoverCliRuntimeTools,
    handleCreateCliRuntimeEnvironment,
    handleInstallCliRuntimeTool,
    handleOpenCliRuntimeEnvironmentRoot,
  };
}
