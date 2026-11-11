import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  RUNTIME_PERF_CHECKPOINT_WINDOW_MS,
  RUNTIME_PERF_HISTORY_LIMIT,
  RUNTIME_PERF_PRESETS,
  RUNTIME_PERF_TIMELINE_LIMIT,
  SettingsTab,
} from "../../features/settings/settingsModel";
import { appAlert } from "../../utils/appDialogs";
import {
  formatRuntimePerfRunIndex,
  RuntimeDiagnosticsSummary,
  RuntimeHookDefinition,
  RuntimePerfCollector,
  runtimePerfContextTypeForMode,
  RuntimeSessionCheckpointItem,
  RuntimeSessionListItem,
  RuntimeSessionToolResultItem,
  RuntimeSessionTranscriptItem,
  toRuntimePerfNumber,
  toRuntimePerfRecord,
  toRuntimePerfText,
} from './settingsSupport';
import {
  type AgentTaskSnapshot,
  type AgentTaskTrace,
  type BackgroundTaskItem,
  type BackgroundWorkerPoolState,
  type RoleSpec,
  type RuntimePerfBenchmarkMode,
  type RuntimePerfRunResult,
  type RuntimePerfTimelineItem,
  type ToolDiagnosticDescriptor,
  type ToolDiagnosticRunResult,
} from "./shared";

export function useSettingsRuntime({ isActive, activeTab, formData }: {
  isActive: boolean;
  activeTab: SettingsTab;
  formData: any;
}) {

  const [recentDebugLogs, setRecentDebugLogs] = useState<string[]>([]);
  const [isDebugLogsLoading, setIsDebugLogsLoading] = useState(false);
  const [toolDiagnostics, setToolDiagnostics] = useState<ToolDiagnosticDescriptor[]>([]);
  const [toolDiagnosticResults, setToolDiagnosticResults] = useState<Record<string, ToolDiagnosticRunResult | undefined>>({});
  const [toolDiagnosticRunning, setToolDiagnosticRunning] = useState<Record<string, 'direct' | 'ai' | undefined>>({});
  const [runtimeTasks, setRuntimeTasks] = useState<AgentTaskSnapshot[]>([]);
  const [runtimeRoles, setRuntimeRoles] = useState<RoleSpec[]>([]);
  const [runtimeDiagnosticsSummary, setRuntimeDiagnosticsSummary] = useState<RuntimeDiagnosticsSummary | null>(null);
  const [runtimeSessions, setRuntimeSessions] = useState<RuntimeSessionListItem[]>([]);
  const [selectedRuntimeTaskId, setSelectedRuntimeTaskId] = useState('');
  const [selectedRuntimeSessionId, setSelectedRuntimeSessionId] = useState('');
  const [runtimeTaskTraces, setRuntimeTaskTraces] = useState<AgentTaskTrace[]>([]);
  const [runtimeSessionTranscript, setRuntimeSessionTranscript] = useState<RuntimeSessionTranscriptItem[]>([]);
  const [runtimeSessionCheckpoints, setRuntimeSessionCheckpoints] = useState<RuntimeSessionCheckpointItem[]>([]);
  const [runtimeSessionToolResults, setRuntimeSessionToolResults] = useState<RuntimeSessionToolResultItem[]>([]);
  const [runtimeHooks, setRuntimeHooks] = useState<RuntimeHookDefinition[]>([]);
  const [backgroundTasks, setBackgroundTasks] = useState<BackgroundTaskItem[]>([]);
  const [backgroundWorkerPool, setBackgroundWorkerPool] = useState<BackgroundWorkerPoolState>({ json: [], runtime: [] });
  const [selectedBackgroundTaskId, setSelectedBackgroundTaskId] = useState('');
  const [selectedBackgroundTaskDetail, setSelectedBackgroundTaskDetail] = useState<BackgroundTaskItem | null>(null);
  const [runtimeDraftInput, setRuntimeDraftInput] = useState('');
  const [runtimeDraftMode, setRuntimeDraftMode] = useState<'redclaw' | 'knowledge' | 'team' | 'advisor-discussion' | 'background-maintenance' | 'diagnostics'>('redclaw');
  const [isRuntimeLoading, setIsRuntimeLoading] = useState(false);
  const [isRuntimeTraceLoading, setIsRuntimeTraceLoading] = useState(false);
  const [isRuntimeSessionLoading, setIsRuntimeSessionLoading] = useState(false);
  const [isBackgroundTasksLoading, setIsBackgroundTasksLoading] = useState(false);
  const [isRuntimeCreating, setIsRuntimeCreating] = useState(false);
  const [runtimeTaskActionRunning, setRuntimeTaskActionRunning] = useState<Record<string, 'resume' | 'cancel' | undefined>>({});
  const [backgroundTaskActionRunning, setBackgroundTaskActionRunning] = useState<Record<string, 'cancel' | undefined>>({});
  const [runtimePerfMode, setRuntimePerfMode] = useState<RuntimePerfBenchmarkMode>('diagnostics');
  const [runtimePerfPresetId, setRuntimePerfPresetId] = useState<string>(RUNTIME_PERF_PRESETS[0].id);
  const [runtimePerfMessage, setRuntimePerfMessage] = useState<string>(RUNTIME_PERF_PRESETS[0].message);
  const [runtimePerfIterations, setRuntimePerfIterations] = useState(1);
  const [isRuntimePerfRunning, setIsRuntimePerfRunning] = useState(false);
  const [runtimePerfStatusMessage, setRuntimePerfStatusMessage] = useState('');
  const [runtimePerfResults, setRuntimePerfResults] = useState<RuntimePerfRunResult[]>([]);
  const [activeRuntimePerfRunId, setActiveRuntimePerfRunId] = useState('');

  const hasSelectedRuntimeSession = useMemo(
    () => Boolean(selectedRuntimeSessionId && runtimeSessions.some((session) => session.id === selectedRuntimeSessionId)),
    [runtimeSessions, selectedRuntimeSessionId],
  );

  const updateRuntimePerfRun = useCallback((runId: string, updater: (run: RuntimePerfRunResult) => RuntimePerfRunResult) => {
    setRuntimePerfResults((prev) =>
      prev.map((run) => (run.id === runId ? updater(run) : run))
    );
  }, []);

  const snapshotRuntimePerfCollector = useCallback((collector: RuntimePerfCollector) => ({
    thinkingStartedMs: collector.thinkingStartedMs,
    thoughtFirstTokenMs: collector.thoughtFirstTokenMs,
    firstResponseMs: collector.firstResponseMs,
    firstToolStartMs: collector.firstToolStartMs,
    firstCheckpointMs: collector.firstCheckpointMs,
    responseChars: collector.responseChars,
    toolCalls: collector.toolCalls,
    toolSuccessCount: collector.toolSuccessCount,
    toolFailureCount: collector.toolFailureCount,
    checkpointCount: collector.checkpointCount,
    checkpointTypes: [...collector.checkpointTypes],
    timeline: [...collector.timeline],
  }), []);

  const appendRuntimePerfTimeline = useCallback((
    collector: RuntimePerfCollector,
    event: Omit<RuntimePerfTimelineItem, 'id' | 'offsetMs'> & { offsetMs?: number },
  ) => {
    const offsetMs = typeof event.offsetMs === 'number'
      ? event.offsetMs
      : Math.max(0, event.at - collector.startedAt);
    const item: RuntimePerfTimelineItem = {
      id: `${collector.runId}:${collector.timeline.length}:${event.eventType}:${event.at}`,
      at: event.at,
      offsetMs,
      eventType: event.eventType,
      label: event.label,
      detail: event.detail,
      tone: event.tone,
    };
    collector.timeline = [...collector.timeline, item].slice(-RUNTIME_PERF_TIMELINE_LIMIT);
  }, []);

  const debugLogsLoadRequestRef = useRef(0);

  const runtimeTasksLoadRequestRef = useRef(0);

  const runtimeSummaryLoadRequestRef = useRef(0);

  const runtimeSessionsLoadRequestRef = useRef(0);

  const runtimeTaskTracesLoadRequestRef = useRef(0);

  const runtimeSessionDetailsLoadRequestRef = useRef(0);

  const runtimeObservabilityRefreshTimerRef = useRef<number | null>(null);

  const runtimePerfCollectorRef = useRef<RuntimePerfCollector | null>(null);

  const runtimePerfRunCounterRef = useRef(0);

  const backgroundTasksLoadRequestRef = useRef(0);

  const backgroundWorkerPoolLoadRequestRef = useRef(0);

  useEffect(() => {
    if (!isActive) return;
    if (activeTab !== 'tools' || !formData.developer_mode_enabled) return;
    const onBackgroundTaskUpdated = (_event: unknown, task: BackgroundTaskItem) => {
      if (!task?.id) return;
      setBackgroundTasks((prev) => {
        const next = [...prev];
        const index = next.findIndex((item) => item.id === task.id);
        if (index >= 0) {
          next[index] = task;
        } else {
          next.unshift(task);
        }
        return next.sort((a, b) => new Date(b.updatedAt).getTime() - new Date(a.updatedAt).getTime()).slice(0, 200);
      });
      setSelectedBackgroundTaskId((prev) => prev || task.id);
      setSelectedBackgroundTaskDetail((prev) => (prev?.id === task.id ? { ...prev, ...task } : prev));
    };
    window.ipcRenderer.backgroundTasks.onUpdated(onBackgroundTaskUpdated);
    return () => {
      window.ipcRenderer.backgroundTasks.offUpdated(onBackgroundTaskUpdated);
    };
  }, [activeTab, formData.developer_mode_enabled, isActive]);

  useEffect(() => {
    if (activeTab !== 'tools' || !formData.developer_mode_enabled) return;
    if (!selectedRuntimeTaskId || !runtimeTasks.some((task) => task.id === selectedRuntimeTaskId)) return;
    void loadRuntimeTaskTraces(selectedRuntimeTaskId);
  }, [activeTab, formData.developer_mode_enabled, runtimeTasks, selectedRuntimeTaskId]);

  useEffect(() => {
    if (activeTab !== 'tools' || !formData.developer_mode_enabled) return;
    if (!selectedRuntimeSessionId || !hasSelectedRuntimeSession) return;
    void loadRuntimeSessionDetails(selectedRuntimeSessionId);
  }, [activeTab, formData.developer_mode_enabled, hasSelectedRuntimeSession, selectedRuntimeSessionId]);

  useEffect(() => {
    if (activeTab !== 'tools' || !formData.developer_mode_enabled) return;
    if (!selectedBackgroundTaskId) {
      setSelectedBackgroundTaskDetail(null);
      return;
    }
    let cancelled = false;
    void (async () => {
      const detail = await window.ipcRenderer.backgroundTasks.get(selectedBackgroundTaskId);
      if (cancelled) return;
      setSelectedBackgroundTaskDetail(detail && typeof detail === 'object' ? detail as BackgroundTaskItem : null);
    })();
    return () => {
      cancelled = true;
    };
  }, [activeTab, formData.developer_mode_enabled, selectedBackgroundTaskId]);

  const loadRecentDebugLogs = useCallback(async () => {
    const requestId = ++debugLogsLoadRequestRef.current;
    setIsDebugLogsLoading(true);
    try {
      const result = await window.ipcRenderer.logs.getRecent(120);
      if (requestId !== debugLogsLoadRequestRef.current) return;
      setRecentDebugLogs(Array.isArray(result?.lines) ? result.lines : []);
    } catch (e) {
      console.error('Failed to load debug logs', e);
    } finally {
      if (requestId === debugLogsLoadRequestRef.current) {
        setIsDebugLogsLoading(false);
      }
    }
  }, []);

  const openDebugLogDirectory = async () => {
    const result = await window.ipcRenderer.logs.openDir();
    if (!result?.success && result?.error) {
      void appAlert(`打开日志目录失败：${result.error}`);
    }
  };

  const loadToolDiagnostics = useCallback(async () => {
    try {
      const result = await window.ipcRenderer.toolDiagnostics.list();
      setToolDiagnostics(Array.isArray(result) ? result : []);
    } catch (e) {
      console.error('Failed to load tool diagnostics', e);
    }
  }, []);

  const loadRuntimeRoles = useCallback(async () => {
    try {
      const result = await window.ipcRenderer.aiRoles.list();
      setRuntimeRoles(Array.isArray(result) ? result : []);
    } catch (e) {
      console.error('Failed to load runtime roles', e);
    }
  }, []);

  const loadRuntimeSummary = useCallback(async () => {
    const requestId = ++runtimeSummaryLoadRequestRef.current;
    try {
      const result = await window.ipcRenderer.debug.getRuntimeSummary();
      if (requestId !== runtimeSummaryLoadRequestRef.current) return;
      setRuntimeDiagnosticsSummary(result || null);
    } catch (e) {
      console.error('Failed to load runtime diagnostics summary', e);
    }
  }, []);

  const loadRuntimeTasks = useCallback(async (preserveSelection = true) => {
    const requestId = ++runtimeTasksLoadRequestRef.current;
    setIsRuntimeLoading(true);
    try {
      const result = await window.ipcRenderer.tasks.list({ limit: 40 });
      if (requestId !== runtimeTasksLoadRequestRef.current) return;
      const taskList = Array.isArray(result) ? result : [];
      setRuntimeTasks(taskList);
      setSelectedRuntimeTaskId((prev) => {
        if (preserveSelection && prev && taskList.some((task) => task.id === prev)) {
          return prev;
        }
        return taskList[0]?.id || '';
      });
    } catch (e) {
      console.error('Failed to load runtime tasks', e);
      if (!preserveSelection) {
        setSelectedRuntimeTaskId('');
      }
    } finally {
      if (requestId === runtimeTasksLoadRequestRef.current) {
        setIsRuntimeLoading(false);
      }
    }
  }, []);

  const loadRuntimeSessions = useCallback(async (preserveSelection = true) => {
    const requestId = ++runtimeSessionsLoadRequestRef.current;
    try {
      const result = await window.ipcRenderer.sessions.list();
      if (requestId !== runtimeSessionsLoadRequestRef.current) return;
      const sessionList = Array.isArray(result) ? result as RuntimeSessionListItem[] : [];
      setRuntimeSessions(sessionList);
      setSelectedRuntimeSessionId((prev) => {
        if (preserveSelection && prev && sessionList.some((session) => session.id === prev)) {
          return prev;
        }
        return sessionList[0]?.id || '';
      });
    } catch (e) {
      console.error('Failed to load runtime sessions', e);
      if (!preserveSelection) {
        setSelectedRuntimeSessionId('');
      }
    }
  }, []);

  const loadRuntimeTaskTraces = useCallback(async (taskId: string) => {
    const normalizedTaskId = String(taskId || '').trim();
    if (!normalizedTaskId) {
      setRuntimeTaskTraces([]);
      return;
    }
    const requestId = ++runtimeTaskTracesLoadRequestRef.current;
    setIsRuntimeTraceLoading(true);
    try {
      const result = await window.ipcRenderer.tasks.trace({ taskId: normalizedTaskId, limit: 120 });
      if (requestId !== runtimeTaskTracesLoadRequestRef.current) return;
      setRuntimeTaskTraces(Array.isArray(result) ? result : []);
    } catch (e) {
      console.error('Failed to load runtime task traces', e);
    } finally {
      if (requestId === runtimeTaskTracesLoadRequestRef.current) {
        setIsRuntimeTraceLoading(false);
      }
    }
  }, []);

  const loadRuntimeSessionDetails = useCallback(async (sessionId: string, options?: { background?: boolean }) => {
    const normalizedSessionId = String(sessionId || '').trim();
    if (!normalizedSessionId) {
      setRuntimeSessionTranscript([]);
      setRuntimeSessionCheckpoints([]);
      setRuntimeSessionToolResults([]);
      return;
    }
    const requestId = ++runtimeSessionDetailsLoadRequestRef.current;
    if (!options?.background) {
      setIsRuntimeSessionLoading(true);
    }
    try {
      const [transcript, checkpoints, toolResults] = await Promise.all([
        window.ipcRenderer.sessions.getTranscript(normalizedSessionId, 120),
        window.ipcRenderer.runtime.getCheckpoints({ sessionId: normalizedSessionId, limit: 80 }),
        window.ipcRenderer.runtime.getToolResults({ sessionId: normalizedSessionId, limit: 120 }),
      ]);
      if (requestId !== runtimeSessionDetailsLoadRequestRef.current) return;
      setRuntimeSessionTranscript(Array.isArray(transcript) ? transcript as RuntimeSessionTranscriptItem[] : []);
      setRuntimeSessionCheckpoints(Array.isArray(checkpoints) ? checkpoints as RuntimeSessionCheckpointItem[] : []);
      setRuntimeSessionToolResults(Array.isArray(toolResults) ? toolResults as RuntimeSessionToolResultItem[] : []);
    } catch (e) {
      console.error('Failed to load runtime session details', e);
    } finally {
      if (!options?.background && requestId === runtimeSessionDetailsLoadRequestRef.current) {
        setIsRuntimeSessionLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    if (!isActive) return;
    if (activeTab !== 'tools' || !formData.developer_mode_enabled) return;
    const scheduleRefresh = () => {
      if (runtimeObservabilityRefreshTimerRef.current != null) {
        window.clearTimeout(runtimeObservabilityRefreshTimerRef.current);
      }
      runtimeObservabilityRefreshTimerRef.current = window.setTimeout(() => {
        runtimeObservabilityRefreshTimerRef.current = null;
        void loadRuntimeSessions();
        if (selectedRuntimeSessionId) {
          void loadRuntimeSessionDetails(selectedRuntimeSessionId, { background: true });
        }
      }, 450);
    };
    const onRuntimeEvent = () => scheduleRefresh();
    const onWanderProgress = () => scheduleRefresh();
    window.ipcRenderer.runtime.onEvent(onRuntimeEvent as (...args: unknown[]) => void);
    window.ipcRenderer.wander.onProgress(onWanderProgress as (...args: unknown[]) => void);
    return () => {
      window.ipcRenderer.runtime.offEvent(onRuntimeEvent as (...args: unknown[]) => void);
      window.ipcRenderer.wander.offProgress(onWanderProgress as (...args: unknown[]) => void);
      if (runtimeObservabilityRefreshTimerRef.current != null) {
        window.clearTimeout(runtimeObservabilityRefreshTimerRef.current);
        runtimeObservabilityRefreshTimerRef.current = null;
      }
    };
  }, [activeTab, formData.developer_mode_enabled, isActive, loadRuntimeSessionDetails, loadRuntimeSessions, selectedRuntimeSessionId]);

  useEffect(() => {
    if (!isActive) return;
    if (activeTab !== 'tools' || !formData.developer_mode_enabled) return;

    const onRuntimePerfEvent = (_event: unknown, envelope?: unknown) => {
      const collector = runtimePerfCollectorRef.current;
      if (!collector) return;

      const record = toRuntimePerfRecord(envelope);
      const eventType = toRuntimePerfText(record.eventType);
      const sessionId = toRuntimePerfText(record.sessionId);
      const timestamp = toRuntimePerfNumber(record.timestamp) || Date.now();
      if (!eventType || sessionId !== collector.sessionId) return;

      const payload = toRuntimePerfRecord(record.payload);
      let changed = false;

      if (eventType === 'runtime:stream-start') {
        const phase = toRuntimePerfText(payload.phase) || 'unknown';
        if (phase === 'thinking' && collector.thinkingStartedMs == null) {
          collector.thinkingStartedMs = Math.max(0, timestamp - collector.startedAt);
          changed = true;
        }
        appendRuntimePerfTimeline(collector, {
          at: timestamp,
          eventType,
          label: `phase · ${phase}`,
          detail: toRuntimePerfText(payload.runtimeMode) || undefined,
          tone: 'neutral',
        });
        changed = true;
      } else if (eventType === 'runtime:text-delta') {
        const stream = toRuntimePerfText(payload.stream);
        const content = toRuntimePerfText(payload.content);
        if (stream === 'thought' && collector.thoughtFirstTokenMs == null) {
          collector.thoughtFirstTokenMs = Math.max(0, timestamp - collector.startedAt);
          appendRuntimePerfTimeline(collector, {
            at: timestamp,
            eventType,
            label: 'thought first token',
            detail: `${content.length} chars`,
            tone: 'neutral',
          });
          changed = true;
        }
        if (stream === 'response') {
          if (collector.firstResponseMs == null) {
            collector.firstResponseMs = Math.max(0, timestamp - collector.startedAt);
            appendRuntimePerfTimeline(collector, {
              at: timestamp,
              eventType,
              label: 'response first token',
              detail: `${content.length} chars`,
              tone: 'success',
            });
            changed = true;
          }
          const nextChars = (collector.responseChars || 0) + content.length;
          if (nextChars !== collector.responseChars) {
            collector.responseChars = nextChars;
            changed = true;
          }
        }
      } else if (eventType === 'runtime:tool-start') {
        collector.toolCalls += 1;
        if (collector.firstToolStartMs == null) {
          collector.firstToolStartMs = Math.max(0, timestamp - collector.startedAt);
        }
        appendRuntimePerfTimeline(collector, {
          at: timestamp,
          eventType,
          label: `tool start · ${toRuntimePerfText(payload.name) || 'tool'}`,
          detail: toRuntimePerfText(payload.description) || undefined,
          tone: 'warning',
        });
        changed = true;
      } else if (eventType === 'runtime:tool-end') {
        const output = toRuntimePerfRecord(payload.output);
        const success = output.success !== false;
        if (success) {
          collector.toolSuccessCount += 1;
        } else {
          collector.toolFailureCount += 1;
        }
        appendRuntimePerfTimeline(collector, {
          at: timestamp,
          eventType,
          label: `tool ${success ? 'done' : 'failed'} · ${toRuntimePerfText(payload.name) || 'tool'}`,
          detail: toRuntimePerfText(output.content) || undefined,
          tone: success ? 'success' : 'error',
        });
        changed = true;
      } else if (eventType === 'runtime:checkpoint') {
        const checkpointType = toRuntimePerfText(payload.checkpointType) || 'checkpoint';
        collector.checkpointCount += 1;
        if (collector.firstCheckpointMs == null) {
          collector.firstCheckpointMs = Math.max(0, timestamp - collector.startedAt);
        }
        if (checkpointType && !collector.checkpointTypes.includes(checkpointType)) {
          collector.checkpointTypes = [...collector.checkpointTypes, checkpointType];
        }
        appendRuntimePerfTimeline(collector, {
          at: timestamp,
          eventType,
          label: `checkpoint · ${checkpointType}`,
          detail: toRuntimePerfText(payload.summary) || undefined,
          tone: checkpointType === 'chat.error' ? 'error' : 'neutral',
        });
        changed = true;
      } else if (eventType === 'runtime:done') {
        appendRuntimePerfTimeline(collector, {
          at: timestamp,
          eventType,
          label: `done · ${toRuntimePerfText(payload.status) || 'completed'}`,
          detail: toRuntimePerfText(payload.reason) || undefined,
          tone: toRuntimePerfText(payload.status) === 'error' ? 'error' : 'success',
        });
        const content = toRuntimePerfText(payload.content);
        if (content) {
          collector.responseChars = Math.max(collector.responseChars || 0, content.length);
        }
        changed = true;
      }

      if (!changed) return;

      updateRuntimePerfRun(collector.runId, (run) => ({
        ...run,
        ...snapshotRuntimePerfCollector(collector),
      }));
    };

    window.ipcRenderer.runtime.onEvent(onRuntimePerfEvent as (...args: unknown[]) => void);
    return () => {
      window.ipcRenderer.runtime.offEvent(onRuntimePerfEvent as (...args: unknown[]) => void);
    };
  }, [
    activeTab,
    appendRuntimePerfTimeline,
    formData.developer_mode_enabled,
    isActive,
    snapshotRuntimePerfCollector,
    updateRuntimePerfRun,
  ]);

  const loadRuntimeHooks = useCallback(async () => {
    try {
      const result = await window.ipcRenderer.toolHooks.list();
      setRuntimeHooks(Array.isArray(result) ? result as RuntimeHookDefinition[] : []);
    } catch (e) {
      console.error('Failed to load runtime hooks', e);
    }
  }, []);

  const loadBackgroundTasks = useCallback(async (preserveSelection = true) => {
    const requestId = ++backgroundTasksLoadRequestRef.current;
    setIsBackgroundTasksLoading(true);
    try {
      const result = await window.ipcRenderer.backgroundTasks.list();
      if (requestId !== backgroundTasksLoadRequestRef.current) return;
      const taskList = Array.isArray(result) ? result as BackgroundTaskItem[] : [];
      setBackgroundTasks(taskList);
      setSelectedBackgroundTaskId((prev) => {
        if (preserveSelection && prev && taskList.some((task) => task.id === prev)) {
          return prev;
        }
        return taskList[0]?.id || '';
      });
      setSelectedBackgroundTaskDetail((prev) => (
        prev && taskList.some((task) => task.id === prev.id) ? prev : null
      ));
    } catch (e) {
      console.error('Failed to load background tasks', e);
      if (!preserveSelection) {
        setSelectedBackgroundTaskId('');
      }
    } finally {
      if (requestId === backgroundTasksLoadRequestRef.current) {
        setIsBackgroundTasksLoading(false);
      }
    }
  }, []);

  const loadBackgroundWorkerPool = useCallback(async () => {
    const requestId = ++backgroundWorkerPoolLoadRequestRef.current;
    try {
      const result = await window.ipcRenderer.backgroundWorkers.getPoolState();
      if (requestId !== backgroundWorkerPoolLoadRequestRef.current) return;
      setBackgroundWorkerPool({
        json: Array.isArray(result?.json) ? result.json : [],
        runtime: Array.isArray(result?.runtime) ? result.runtime : [],
      });
    } catch (e) {
      console.error('Failed to load background worker pool', e);
    }
  }, []);

  const loadRuntimeDeveloperData = useCallback(async () => {
    await Promise.all([
      loadRuntimeRoles(),
      loadRuntimeSummary(),
      loadToolDiagnostics(),
    ]);
    await Promise.all([
      loadRuntimeTasks(),
      loadRuntimeSessions(),
      loadRuntimeHooks(),
    ]);
    await Promise.all([
      loadBackgroundTasks(),
      loadBackgroundWorkerPool(),
    ]);
  }, [
    loadBackgroundTasks,
    loadBackgroundWorkerPool,
    loadRuntimeHooks,
    loadRuntimeRoles,
    loadRuntimeSummary,
    loadRuntimeSessions,
    loadRuntimeTasks,
    loadToolDiagnostics,
  ]);

  const handleApplyRuntimePerfPreset = useCallback((presetId: string) => {
    const preset = RUNTIME_PERF_PRESETS.find((item) => item.id === presetId) || RUNTIME_PERF_PRESETS[0];
    setRuntimePerfPresetId(preset.id);
    setRuntimePerfMessage(preset.message);
  }, []);

  const ensureRuntimePerfSession = useCallback(async (
    mode: RuntimePerfBenchmarkMode,
    index: number,
  ): Promise<{ id: string }> => {
    const contextType = runtimePerfContextTypeForMode(mode);
    const timestamp = Date.now();
    const contextId = `developer-runtime-perf-${mode}-${timestamp}-${index}`;
    const title = `Runtime Perf · ${mode} · ${formatRuntimePerfRunIndex(index)}`;
    if (mode === 'diagnostics') {
      return await window.ipcRenderer.chat.createDiagnosticsSession({
        title,
        contextId,
        contextType,
      }) as { id: string };
    }
    return await window.ipcRenderer.chat.createContextSession({
      contextId,
      contextType,
      title,
    }) as { id: string };
  }, []);

  const handleClearRuntimePerfResults = useCallback(() => {
    runtimePerfCollectorRef.current = null;
    setActiveRuntimePerfRunId('');
    setRuntimePerfResults([]);
    setRuntimePerfStatusMessage('');
  }, []);

  const handleRunRuntimePerfBenchmark = useCallback(async () => {
    const trimmedMessage = runtimePerfMessage.trim();
    if (!trimmedMessage || isRuntimePerfRunning) return;

    setIsRuntimePerfRunning(true);
    setRuntimePerfStatusMessage(`准备执行 ${runtimePerfIterations} 轮 runtime benchmark...`);

    try {
      for (let iterationIndex = 0; iterationIndex < runtimePerfIterations; iterationIndex += 1) {
        const runNumber = ++runtimePerfRunCounterRef.current;
        const session = await ensureRuntimePerfSession(runtimePerfMode, runNumber);
        const sessionId = String(session?.id || '').trim();
        if (!sessionId) {
          throw new Error('性能测试未拿到有效 sessionId');
        }

        const startedAt = Date.now();
        const runId = `runtime-perf-${startedAt}-${runNumber}`;
        const collector: RuntimePerfCollector = {
          runId,
          sessionId,
          startedAt,
          toolCalls: 0,
          toolSuccessCount: 0,
          toolFailureCount: 0,
          checkpointCount: 0,
          checkpointTypes: [],
          timeline: [],
        };
        runtimePerfCollectorRef.current = collector;
        setActiveRuntimePerfRunId(runId);
        setSelectedRuntimeSessionId(sessionId);
        appendRuntimePerfTimeline(collector, {
          at: startedAt,
          eventType: 'run:start',
          label: '测试开始',
          detail: `${runtimePerfMode} · ${formatRuntimePerfRunIndex(runNumber)}`,
          tone: 'neutral',
          offsetMs: 0,
        });
        const pendingRun: RuntimePerfRunResult = {
          id: runId,
          index: runNumber,
          runtimeMode: runtimePerfMode,
          sessionId,
          presetId: runtimePerfPresetId,
          message: trimmedMessage,
          status: 'running',
          startedAt,
          toolCalls: 0,
          toolSuccessCount: 0,
          toolFailureCount: 0,
          checkpointCount: 0,
          checkpointTypes: [],
          timeline: [...collector.timeline],
        };
        setRuntimePerfResults((prev) => [
          pendingRun,
          ...prev,
        ].slice(0, RUNTIME_PERF_HISTORY_LIMIT));

        setRuntimePerfStatusMessage(`执行中：第 ${iterationIndex + 1}/${runtimePerfIterations} 轮`);

        let finalStatus: RuntimePerfRunResult['status'] = 'completed';
        let finalError = '';
        let finalResponseChars = 0;
        let routeValue: unknown = null;
        let orchestrationValue: unknown = null;

        try {
          const result = await window.ipcRenderer.runtime.query({
            sessionId,
            message: trimmedMessage,
          }) as {
            success?: boolean;
            response?: string;
            route?: unknown;
            orchestration?: unknown;
          };
          if (result?.success === false) {
            throw new Error('runtime query returned success=false');
          }
          finalResponseChars = String(result?.response || '').length;
          routeValue = result?.route;
          orchestrationValue = result?.orchestration;
        } catch (error) {
          finalStatus = 'failed';
          finalError = error instanceof Error ? error.message : String(error);
        }

        const completedAt = Date.now();
        appendRuntimePerfTimeline(collector, {
          at: completedAt,
          eventType: 'run:finish',
          label: finalStatus === 'completed' ? '测试完成' : '测试失败',
          detail: finalError || undefined,
          tone: finalStatus === 'completed' ? 'success' : 'error',
        });

        const [summary, checkpoints, toolResults] = await Promise.all([
          window.ipcRenderer.debug.getRuntimeSummary(),
          window.ipcRenderer.runtime.getCheckpoints({ sessionId, limit: 120 }),
          window.ipcRenderer.runtime.getToolResults({ sessionId, limit: 120 }),
        ]);
        setRuntimeDiagnosticsSummary(summary || null);

        const checkpointRows = (Array.isArray(checkpoints) ? checkpoints : []).filter((item) => {
          const createdAt = toRuntimePerfNumber((item as Record<string, unknown>)?.createdAt) || 0;
          return createdAt >= (startedAt - RUNTIME_PERF_CHECKPOINT_WINDOW_MS);
        }) as RuntimeSessionCheckpointItem[];
        const toolRows = (Array.isArray(toolResults) ? toolResults : []).filter((item) => {
          const createdAt = toRuntimePerfNumber((item as Record<string, unknown>)?.createdAt) || 0;
          return createdAt >= (startedAt - RUNTIME_PERF_CHECKPOINT_WINDOW_MS);
        }) as RuntimeSessionToolResultItem[];
        const recentRuntimeMetrics = Array.isArray(summary?.phase0?.runtimeQueries?.recent)
          ? summary.phase0.runtimeQueries.recent as Array<Record<string, unknown>>
          : [];
        const matchingMetric = recentRuntimeMetrics.find((item) =>
          String(item.sessionId || '').trim() === sessionId
          && (toRuntimePerfNumber(item.createdAt) || 0) >= (startedAt - RUNTIME_PERF_CHECKPOINT_WINDOW_MS)
        );

        const toolSuccessCount = toolRows.filter((item) => Boolean(item.success)).length;
        const toolFailureCount = toolRows.length - toolSuccessCount;
        const checkpointTypes = checkpointRows
          .map((item) => String(item.checkpointType || '').trim())
          .filter(Boolean);

        collector.responseChars = collector.responseChars ?? finalResponseChars;
        collector.toolCalls = Math.max(collector.toolCalls, toolRows.length);
        collector.toolSuccessCount = Math.max(collector.toolSuccessCount, toolSuccessCount);
        collector.toolFailureCount = Math.max(collector.toolFailureCount, toolFailureCount);
        collector.checkpointCount = Math.max(collector.checkpointCount, checkpointRows.length);
        collector.checkpointTypes = checkpointTypes.length ? checkpointTypes : collector.checkpointTypes;

        updateRuntimePerfRun(runId, (run) => ({
          ...run,
          status: finalStatus,
          completedAt,
          totalElapsedMs: Math.max(0, completedAt - startedAt),
          promptChars: toRuntimePerfNumber(matchingMetric?.promptChars),
          activeSkillCount: toRuntimePerfNumber(matchingMetric?.activeSkillCount),
          responseChars: collector.responseChars ?? finalResponseChars,
          toolCalls: collector.toolCalls,
          toolSuccessCount: collector.toolSuccessCount,
          toolFailureCount: collector.toolFailureCount,
          checkpointCount: collector.checkpointCount,
          checkpointTypes: [...collector.checkpointTypes],
          route: routeValue,
          orchestration: orchestrationValue,
          error: finalError || undefined,
          ...snapshotRuntimePerfCollector(collector),
        }));

        runtimePerfCollectorRef.current = null;
        setActiveRuntimePerfRunId('');
        await loadRuntimeSessions();
        await loadRuntimeSessionDetails(sessionId);
      }
      setRuntimePerfStatusMessage(`已完成 ${runtimePerfIterations} 轮 runtime benchmark`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setRuntimePerfStatusMessage(`runtime benchmark 失败：${message}`);
      const activeCollector = runtimePerfCollectorRef.current;
      if (activeCollector) {
        const completedAt = Date.now();
        appendRuntimePerfTimeline(activeCollector, {
          at: completedAt,
          eventType: 'run:error',
          label: '执行异常',
          detail: message,
          tone: 'error',
        });
        updateRuntimePerfRun(activeCollector.runId, (run) => ({
          ...run,
          status: 'failed',
          completedAt,
          totalElapsedMs: Math.max(0, completedAt - run.startedAt),
          error: message,
          ...snapshotRuntimePerfCollector(activeCollector),
        }));
      }
    } finally {
      runtimePerfCollectorRef.current = null;
      setActiveRuntimePerfRunId('');
      setIsRuntimePerfRunning(false);
    }
  }, [
    appendRuntimePerfTimeline,
    ensureRuntimePerfSession,
    isRuntimePerfRunning,
    loadRuntimeSessionDetails,
    loadRuntimeSessions,
    runtimePerfIterations,
    runtimePerfMessage,
    runtimePerfMode,
    runtimePerfPresetId,
    snapshotRuntimePerfCollector,
    updateRuntimePerfRun,
  ]);

  const handleCreateRuntimeTask = async () => {
    setIsRuntimeCreating(true);
    try {
      const created = await window.ipcRenderer.tasks.create({
        runtimeMode: runtimeDraftMode,
        sessionId: `dev_task_${Date.now()}`,
        userInput: runtimeDraftInput.trim() || '开发者手动创建任务',
        metadata: {
          source: 'settings-developer-runtime',
        },
      });
      setRuntimeDraftInput('');
      if (created?.id) {
        setSelectedRuntimeTaskId(created.id);
        await loadRuntimeTasks(false);
        await loadRuntimeTaskTraces(created.id);
      } else {
        await loadRuntimeTasks(false);
      }
    } catch (e) {
      console.error('Failed to create runtime task', e);
    } finally {
      setIsRuntimeCreating(false);
    }
  };

  const handleResumeRuntimeTask = async (taskId: string) => {
    setRuntimeTaskActionRunning((prev) => ({ ...prev, [taskId]: 'resume' }));
    try {
      await window.ipcRenderer.tasks.resume({ taskId });
      await loadRuntimeTasks();
      await loadRuntimeTaskTraces(taskId);
    } catch (e) {
      console.error('Failed to resume runtime task', e);
    } finally {
      setRuntimeTaskActionRunning((prev) => ({ ...prev, [taskId]: undefined }));
    }
  };

  const handleCancelRuntimeTask = async (taskId: string) => {
    setRuntimeTaskActionRunning((prev) => ({ ...prev, [taskId]: 'cancel' }));
    try {
      await window.ipcRenderer.tasks.cancel({ taskId });
      await loadRuntimeTasks();
      await loadRuntimeTaskTraces(taskId);
    } catch (e) {
      console.error('Failed to cancel runtime task', e);
    } finally {
      setRuntimeTaskActionRunning((prev) => ({ ...prev, [taskId]: undefined }));
    }
  };

  const handleCancelBackgroundTask = async (taskId: string) => {
    setBackgroundTaskActionRunning((prev) => ({ ...prev, [taskId]: 'cancel' }));
    try {
      await window.ipcRenderer.backgroundTasks.cancel(taskId);
      await loadBackgroundTasks();
    } catch (e) {
      console.error('Failed to cancel background task', e);
    } finally {
      setBackgroundTaskActionRunning((prev) => ({ ...prev, [taskId]: undefined }));
    }
  };

  const runToolDiagnostic = async (toolName: string, mode: 'direct' | 'ai') => {
    setToolDiagnosticRunning((prev) => ({ ...prev, [toolName]: mode }));
    try {
      const result = mode === 'direct'
        ? await window.ipcRenderer.toolDiagnostics.runDirect(toolName)
        : await window.ipcRenderer.toolDiagnostics.runAi(toolName);
      setToolDiagnosticResults((prev) => ({ ...prev, [toolName]: result }));
      await loadRecentDebugLogs();
    } catch (e) {
      const errorMessage = e instanceof Error ? e.message : String(e);
      setToolDiagnosticResults((prev) => ({
        ...prev,
        [toolName]: {
          success: false,
          mode,
          toolName,
          request: null,
          error: errorMessage,
        },
      }));
    } finally {
      setToolDiagnosticRunning((prev) => ({ ...prev, [toolName]: undefined }));
    }
  };

  const runAllToolDiagnostics = async (mode: 'direct' | 'ai') => {
    const candidates = toolDiagnostics.filter((tool) => tool.availabilityStatus === 'available');
    for (const tool of candidates) {
      // eslint-disable-next-line no-await-in-loop
      await runToolDiagnostic(tool.name, mode);
    }
  };
  return {
    loadToolDiagnostics,
    loadRuntimeDeveloperData,
    loadRecentDebugLogs,
    loadRuntimeRoles,
    loadBackgroundTasks,
    loadBackgroundWorkerPool,
    loadRuntimeHooks,
    loadRuntimeSessions,
    loadRuntimeTasks,
    runtimeTasks,
    runtimeSessions,
    backgroundTasks,
    recentDebugLogs,
    isDebugLogsLoading,
    openDebugLogDirectory,
    toolDiagnostics,
    toolDiagnosticResults,
    toolDiagnosticRunning,
    runToolDiagnostic,
    runAllToolDiagnostics,
    runtimePerfMode,
    setRuntimePerfMode,
    runtimePerfPresetId,
    setRuntimePerfPresetId,
    runtimePerfMessage,
    setRuntimePerfMessage,
    runtimePerfIterations,
    setRuntimePerfIterations,
    runtimePerfResults,
    activeRuntimePerfRunId,
    isRuntimePerfRunning,
    runtimePerfStatusMessage,
    handleApplyRuntimePerfPreset,
    handleRunRuntimePerfBenchmark,
    handleClearRuntimePerfResults,
    runtimeRoles,
    runtimeDiagnosticsSummary,
    backgroundWorkerPool,
    selectedRuntimeTaskId,
    setSelectedRuntimeTaskId,
    selectedRuntimeSessionId,
    setSelectedRuntimeSessionId,
    selectedBackgroundTaskId,
    setSelectedBackgroundTaskId,
    selectedBackgroundTaskDetail,
    runtimeTaskTraces,
    runtimeSessionTranscript,
    runtimeSessionCheckpoints,
    runtimeSessionToolResults,
    runtimeHooks,
    runtimeDraftInput,
    setRuntimeDraftInput,
    runtimeDraftMode,
    setRuntimeDraftMode,
    isRuntimeLoading,
    isRuntimeTraceLoading,
    isRuntimeSessionLoading,
    isBackgroundTasksLoading,
    isRuntimeCreating,
    runtimeTaskActionRunning,
    backgroundTaskActionRunning,
    handleCreateRuntimeTask,
    handleResumeRuntimeTask,
    handleCancelRuntimeTask,
    handleCancelBackgroundTask,
  };
}
