import clsx from 'clsx';
import {
  ArrowLeft,
  Cpu,
  FileText,
  FlaskConical,
  LayoutGrid,
  MessageSquareText,
  RefreshCw,
  Server,
  Star,
  Store,
  Trash2,
  Users,
  Wrench,
} from 'lucide-react';
import { useCallback, useEffect, useMemo, useRef, useState, type ComponentType, type SetStateAction } from 'react';
import {
  REDBOX_OFFICIAL_VIDEO_BASE_URL,
  REDBOX_OFFICIAL_VIDEO_MODEL_LIST,
  REDBOX_OFFICIAL_VIDEO_MODELS,
} from "../../../shared/redboxVideo";
import { subscribeSettingsUpdated } from "../../bridge/appEvents";
import { isTauriRuntime } from "../../bridge/browserHost";
import {
  canonicalizeOfficialAutoSourceId,
  findAiPresetById,
  inferPresetIdByEndpoint,
  isOfficialAutoSourceId,
  OFFICIAL_AUTO_SOURCE_ID,
  type AiSourceConfig,
} from "../../config/aiSources";
import { APP_BRAND } from "../../config/brand";
import {
  createDefaultEcommercePlatformsSettings,
  normalizeEcommercePlatformsSettings,
  serializeEcommercePlatformsSettings,
} from "../../features/ecommerce-platforms/catalog";
import { loadOfficialAiPanelModule } from "../../features/official";
import {
  AiModelRouteConfig,
  AiModelRouteMode,
  AiModelRoutes,
  AiModelRouteScope,
  AiPricingCatalog,
  AiPricingModel,
  cloneModelForVoiceTtsModel,
  DEFAULT_AI_MODEL_ROUTES,
  DEFAULT_VIDEO_ANALYSIS_ENABLED,
  DEFAULT_VISUAL_INDEX_ENABLED,
  DEFAULT_VISUAL_INDEX_PROMPT_VERSION,
  DEFAULT_VOICE_CLONE_MODEL,
  DEFAULT_VOICE_TTS_MODEL,
  FILE_INDEX_DASHBOARD_CACHE_TTL_MS,
  FILE_INDEX_DASHBOARD_POLL_MS,
  formatPricingUpdatedAt,
  formatSettingsSkillSource,
  hasMeaningfulPricingValue,
  mcpDraftFromServer,
  McpServerDraft,
  mcpServerFromDraft,
  normalizeAiModelRoutes,
  normalizeVisualIndexPromptVersion,
  parseAiPricingCatalog,
  pricingModeLabel,
  pricingModelFields,
  pricingRateCellValue,
  pricingRateLabel,
  SettingsNavigationTarget,
  SettingsSkill,
  SettingsTab,
} from "../../features/settings/settingsModel";
import { useI18n, type I18nKey } from "../../i18n";
import { playTestNotificationSound } from "../../notifications/audio";
import { DEFAULT_NOTIFICATION_SETTINGS, parseNotificationSettings } from "../../notifications/types";
import type {
  CodexPluginMarketplaceItem,
  DiagnosticsLogStatus,
  DiagnosticsPendingReport,
  NotificationSettingsPayload,
  ThrivePluginMarketplaceItem,
  ThrivePluginSummary,
  ThriveSkillMarketplaceItem,
} from "../../types";
import { appAlert, appConfirm } from "../../utils/appDialogs";
import { SHOW_CURRENT_RELEASE_NOTES_EVENT } from "../../utils/currentReleaseNotes";
import { type Advisor } from "../Advisors";
import {
  isRedClawOnboardingCompleted,
  type RedclawOnboardingState,
} from "../redclaw/onboardingState";
import {
  type FileIndexDashboard
} from "./SettingsSections";
import {
  AssistantDaemonDraft,
  AssistantDaemonStatus,
  assistantDaemonStatusToDraft,
  AssistantDaemonWeixinLoginState,
  clearCachedFileIndexDashboard,
  createDefaultAssistantDaemonDraft,
  DEFAULT_CHAT_MAX_TOKENS,
  DEFAULT_CHAT_MAX_TOKENS_DEEPSEEK,
  DEFAULT_SPACE_ID,
  DEVELOPER_MODE_TTL_MS,
  DEVELOPER_MODE_UNLOCK_TAP_COUNT,
  EMPTY_REDCLAW_PROFILE_DRAFT,
  normalizeCliRuntimeExecutionMode,
  readCachedFileIndexDashboard,
  RedclawProfileDraft,
  sanitizeChatMaxTokensInput,
  SETTINGS_ACTIVATION_DEBOUNCE_MS,
  SETTINGS_TAB_POLL_DELAY_MS,
  WorkspaceSpace,
  writeCachedFileIndexDashboard,
} from "./settingsSupport";
import {
  DASHSCOPE_LOCKED_IMAGE_MODEL,
  generateAiSourceId,
  inferImageTemplateByProvider,
  normalizeAiModelDescriptors,
  normalizeSourceModels,
  parseAiSources,
  parseEnvText,
  parseMcpServers,
  resolveDefaultImageEndpoint,
  type AiProtocol,
  type McpServerConfig,
  type McpServerRuntimeItem,
  type McpSessionState,
} from "./shared";
import { useSettingsAi } from "./useSettingsAi";
import { useSettingsCliRuntime } from "./useSettingsCliRuntime";
import { useSettingsRuntime } from "./useSettingsRuntime";
import { useSettingsTeam } from "./useSettingsTeam";

export function useSettingsController({
  isActive = true,
  onOpenAppOnboarding,
  onOpenRedClawOnboarding,
  redclawOnboardingVersion = 0,
  navigationTarget,
  onReturn,
}: {
  isActive?: boolean;
  onOpenAppOnboarding?: () => void;
  onOpenRedClawOnboarding?: () => void;
  redclawOnboardingVersion?: number;
  navigationTarget?: SettingsNavigationTarget | null;
  onReturn?: () => void;
}) {
  const { t } = useI18n();
  const [activeTab, setActiveTab] = useState<SettingsTab>('ai');
  /**
   * 是否管理员（桌面端恒 true；web 普通用户 false）。
   *
   * 「AI 模型」是服务器级配置：改它影响所有 web 用户，且普通用户在官方源未登录时
   * 无论怎么点都只会看到「请先登录官方账号」。所以这个 tab 只对管理员开放。
   * 失败时按桌面端处理（fail-open），避免一过性错误把桌面端的设置页阉了。
   */
  const [isAdminUser, setIsAdminUser] = useState(true);
  const [settingsSkills, setSettingsSkills] = useState<SettingsSkill[]>([]);
  const [isSettingsSkillsLoading, setIsSettingsSkillsLoading] = useState(false);
  const [settingsSkillBusyName, setSettingsSkillBusyName] = useState('');
  const [settingsSkillStatusMessage, setSettingsSkillStatusMessage] = useState('');
  const [areBuiltinSkillsExpanded, setAreBuiltinSkillsExpanded] = useState(false);
  const [isSkillMarketplaceOpen, setIsSkillMarketplaceOpen] = useState(false);
  const [skillMarketplaceItems, setSkillMarketplaceItems] = useState<ThriveSkillMarketplaceItem[]>([]);
  const [isSkillMarketplaceLoading, setIsSkillMarketplaceLoading] = useState(false);
  const [skillMarketplaceBusyId, setSkillMarketplaceBusyId] = useState('');
  const initialFileIndexDashboardCache = useMemo(() => readCachedFileIndexDashboard(), []);
  const [baseSettingsLoadedRevision, setBaseSettingsLoadedRevision] = useState(0);
  const [settingsSubView, setSettingsSubView] = useState<'main' | 'ai-pricing'>('main');
  const [aiPricingCatalog, setAiPricingCatalog] = useState<AiPricingCatalog | null>(null);
  const [aiPricingLoading, setAiPricingLoading] = useState(false);
  const [aiPricingError, setAiPricingError] = useState('');
  const [aiPricingActiveGroup, setAiPricingActiveGroup] = useState('');
  const [aiPricingSearch, setAiPricingSearch] = useState('');
  const builtinSettingsSkills = useMemo(
    () => settingsSkills.filter((skill) => skill.isBuiltin),
    [settingsSkills]
  );
  const editableSettingsSkills = useMemo(
    () => settingsSkills.filter((skill) => !skill.isBuiltin),
    [settingsSkills]
  );
  const [formData, setFormData] = useState<any>({
    api_endpoint: '',
    api_key: '',
    model_name: '',
    workspace_dir: '',
    transcription_model: '',
    transcription_endpoint: '',
    transcription_key: '',
    embedding_endpoint: '',
    embedding_key: '',
    embedding_model: '',
    visual_index_enabled: DEFAULT_VISUAL_INDEX_ENABLED,
    visual_index_provider: 'openai-compatible',
    visual_index_endpoint: '',
    visual_index_api_key: '',
    visual_index_model: '',
    visual_index_prompt_version: DEFAULT_VISUAL_INDEX_PROMPT_VERSION,
    visual_index_timeout_seconds: '90',
    visual_index_max_image_edge: '1536',
    visual_index_skip_small_images: true,
    visual_index_pdf_max_pages: '12',
    visual_index_pdf_render_dpi: '144',
    visual_index_concurrency: '1',
    video_analysis_enabled: DEFAULT_VIDEO_ANALYSIS_ENABLED,
    video_analysis_endpoint: '',
    video_analysis_api_key: '',
    video_analysis_model: '',
    video_analysis_protocol: 'gemini',
    video_analysis_max_direct_video_bytes: String(64 * 1024 * 1024),
    docling_endpoint: '',
    tika_endpoint: '',
    unstructured_endpoint: '',
    parser_api_key: '',
    parser_timeout_seconds: '90',
    rerank_endpoint: '',
    rerank_api_key: '',
    rerank_model: '',
    rerank_timeout_seconds: '30',
    image_provider: 'openai-compatible',
    image_endpoint: '',
    image_api_key: '',
    image_model: '',
    voice_endpoint: '',
    voice_api_key: '',
    voice_tts_model: DEFAULT_VOICE_TTS_MODEL,
    tts_model: DEFAULT_VOICE_TTS_MODEL,
    voice_clone_model: DEFAULT_VOICE_CLONE_MODEL,
    video_endpoint: '',
    video_api_key: '',
    video_model: String(REDBOX_OFFICIAL_VIDEO_MODELS['text-to-video']),
    image_provider_template: 'openai-images',
    image_aspect_ratio: '3:4',
    image_size: '',
    image_quality: 'medium',
    model_name_wander: '',
    model_name_chatroom: '',
    model_name_knowledge: '',
    model_name_redclaw: '',
    proxy_enabled: false,
    proxy_url: '',
    proxy_bypass: 'localhost,127.0.0.1,::1',
    redclaw_compact_target_tokens: '256000',
    chat_max_tokens_default: String(DEFAULT_CHAT_MAX_TOKENS),
    chat_max_tokens_deepseek: String(DEFAULT_CHAT_MAX_TOKENS_DEEPSEEK),
    wander_deep_think_enabled: false,
    debug_log_enabled: false,
    diagnostics_upload_consent: 'approved',
    diagnostics_include_advanced_context: false,
    diagnostics_auto_send_same_crash: false,
    diagnostics_last_prompted_at: '',
    analytics_consent: 'approved',
    release_log_retention_days: '7',
    release_log_max_file_mb: '10',
    cli_runtime_execution_mode: 'host_compatible',
    developer_mode_enabled: false,
    developer_mode_unlocked_at: '',
    ai_model_routes_json: JSON.stringify(DEFAULT_AI_MODEL_ROUTES),
    ecommerce_platforms_json: serializeEcommercePlatformsSettings(createDefaultEcommercePlatformsSettings()),
  });
  const {
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
  } = useSettingsCliRuntime({ setFormData, formData });

  const {
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
  } = useSettingsRuntime({ isActive, activeTab, formData });
  const [testStatus, setTestStatus] = useState<'idle' | 'success' | 'error'>('idle');
  const [testMsg, setTestMsg] = useState('');
  const [fileIndexDashboard, setFileIndexDashboard] = useState<FileIndexDashboard | null>(
    () => initialFileIndexDashboardCache?.dashboard ?? null,
  );
  const [isFileIndexDashboardLoading, setIsFileIndexDashboardLoading] = useState(false);
  const [logStatus, setLogStatus] = useState<DiagnosticsLogStatus | null>(null);
  const [pendingDiagnosticReports, setPendingDiagnosticReports] = useState<DiagnosticsPendingReport[]>([]);
  const [diagnosticsActionBusy, setDiagnosticsActionBusy] = useState<string | null>(null);
  const [status, setStatus] = useState<'idle' | 'saving' | 'saved' | 'error'>('idle');
  const {
    refreshTeamAdvisor,
    loadTeamAdvisors,
    teamAdvisors,
    isTeamAdvisorsLoading,
    teamAdvisorBusyId,
    draggingTeamAdvisorId,
    handleCreateTeamAdvisor,
    handleToggleTeamAdvisorVisible,
    handleOpenTeamAdvisorSettings,
    handleTeamAdvisorDragStart,
    handleTeamAdvisorDragOver,
    handleTeamAdvisorDragEnd,
    editingTeamAdvisor,
    isCreatingTeamAdvisor,
    handleSaveTeamAdvisor,
    setEditingTeamAdvisor,
    setIsCreatingTeamAdvisor,
    settingsTeamAdvisor,
    setSettingsTeamAdvisor,
    isTeamSystemPromptExpanded,
    setIsTeamSystemPromptExpanded,
    isTeamOptimizingPrompt,
    handleOptimizeTeamAdvisorPrompt,
    handleUploadTeamAdvisorKnowledge,
    handleDeleteTeamAdvisorKnowledge,
    handlePromoteTeamMemberSkillCandidate,
    handleDiscardTeamMemberSkillCandidate,
    handleRefreshTeamMemberSkill,
    handleDeleteTeamAdvisor,
  } = useSettingsTeam({ setTestMsg, setStatus });

  const [redclawProfileDraft, setRedclawProfileDraft] = useState<RedclawProfileDraft>(EMPTY_REDCLAW_PROFILE_DRAFT);
  const [savedRedclawProfileDraft, setSavedRedclawProfileDraft] = useState<RedclawProfileDraft>(EMPTY_REDCLAW_PROFILE_DRAFT);
  const [redclawProfileRoot, setRedclawProfileRoot] = useState('');
  const [isRedclawProfileLoading, setIsRedclawProfileLoading] = useState(false);
  const [redclawProfileDirty, setRedclawProfileDirty] = useState(false);
  const [redclawProfileMessage, setRedclawProfileMessage] = useState<{ tone: 'error' | 'success'; text: string } | null>(null);
  const [redclawOnboardingState, setRedclawOnboardingState] = useState<RedclawOnboardingState>(null);
  const [currentSpaceId, setCurrentSpaceId] = useState(DEFAULT_SPACE_ID);
  const [spaces, setSpaces] = useState<WorkspaceSpace[]>([]);
  const [assistantDaemonStatus, setAssistantDaemonStatus] = useState<AssistantDaemonStatus | null>(null);
  const [assistantDaemonDraft, setAssistantDaemonDraftState] = useState<AssistantDaemonDraft>(() => createDefaultAssistantDaemonDraft());
  const [assistantDaemonLogs, setAssistantDaemonLogs] = useState<string[]>([]);
  const [assistantDaemonBusy, setAssistantDaemonBusy] = useState(false);
  const [assistantDaemonDraftDirty, setAssistantDaemonDraftDirty] = useState(false);
  const [assistantDaemonAcpToken, setAssistantDaemonAcpToken] = useState('');
  const [assistantDaemonWeixinLoginBusy, setAssistantDaemonWeixinLoginBusy] = useState(false);
  const [assistantDaemonWeixinLogin, setAssistantDaemonWeixinLogin] = useState<AssistantDaemonWeixinLoginState | null>(null);
  const [developerVersionTapCount, setDeveloperVersionTapCount] = useState(0);
  const [notificationSettings, setNotificationSettings] = useState<NotificationSettingsPayload>(DEFAULT_NOTIFICATION_SETTINGS);
  const redclawOnboardingCompleted = useMemo(
    () => isRedClawOnboardingCompleted(redclawOnboardingState),
    [redclawOnboardingState],
  );
  const currentSpaceName = useMemo(
    () => spaces.find((space) => space.id === currentSpaceId)?.name || currentSpaceId,
    [currentSpaceId, spaces],
  );
  const ecommercePlatformsSettings = useMemo(
    () => normalizeEcommercePlatformsSettings(formData.ecommerce_platforms_json),
    [formData.ecommerce_platforms_json],
  );

  const handleToggleEcommercePlatform = useCallback((platformId: string, enabled: boolean) => {
    setFormData((prev: any) => {
      const nextSettings = normalizeEcommercePlatformsSettings(prev.ecommerce_platforms_json);
      nextSettings.enabledById = {
        ...nextSettings.enabledById,
        [platformId]: enabled,
      };
      return {
        ...prev,
        ecommerce_platforms_json: serializeEcommercePlatformsSettings(nextSettings),
      };
    });
  }, []);

  const buildWeixinQrImageUrl = useCallback(async (rawUrl?: string): Promise<string | undefined> => {
    const text = String(rawUrl || '').trim();
    if (!text) return undefined;
    try {
      const QRCode = await import('qrcode');
      return await QRCode.toDataURL(text, {
        errorCorrectionLevel: 'M',
        margin: 1,
        width: 320,
      });
    } catch (error) {
      console.error('Failed to build Weixin QR image', error);
      return undefined;
    }
  }, []);

  const setRedclawProfileDirtyState = useCallback((next: boolean) => {
    redclawProfileDirtyRef.current = next;
    setRedclawProfileDirty(next);
  }, []);

  const setCurrentSpaceState = useCallback((spaceId?: string | null) => {
    const normalized = String(spaceId || '').trim() || DEFAULT_SPACE_ID;
    currentSpaceIdRef.current = normalized;
    setCurrentSpaceId(normalized);
    return normalized;
  }, []);

  const loadSpaceContext = useCallback(async () => {
    try {
      const result = await window.ipcRenderer.spaces.list() as { spaces?: WorkspaceSpace[]; activeSpaceId?: string } | null;
      const nextSpaces = Array.isArray(result?.spaces) ? result.spaces : [];
      setSpaces(nextSpaces);
      if (result?.activeSpaceId) {
        setCurrentSpaceState(result.activeSpaceId);
      }
    } catch (error) {
      console.error('Failed to load settings spaces:', error);
    }
  }, [setCurrentSpaceState]);

  const resetRedclawProfileState = useCallback(() => {
    redclawProfileLoadRequestRef.current += 1;
    setRedclawProfileRoot('');
    setSavedRedclawProfileDraft(EMPTY_REDCLAW_PROFILE_DRAFT);
    setRedclawProfileDraft(EMPTY_REDCLAW_PROFILE_DRAFT);
    setRedclawOnboardingState(null);
    setRedclawProfileDirtyState(false);
    setRedclawProfileMessage(null);
    setIsRedclawProfileLoading(false);
  }, [setRedclawProfileDirtyState]);

  const loadRedclawProfileBundle = useCallback(async (options?: { preserveDraft?: boolean; expectedSpaceId?: string }) => {
    const expectedSpaceId = String(options?.expectedSpaceId || currentSpaceIdRef.current || DEFAULT_SPACE_ID).trim() || DEFAULT_SPACE_ID;
    const requestId = ++redclawProfileLoadRequestRef.current;
    setIsRedclawProfileLoading(true);
    try {
      const bundle = await window.ipcRenderer.redclawProfile.getBundle();
      if (requestId !== redclawProfileLoadRequestRef.current) return;
      const responseSpaceId = String(bundle.activeSpaceId || expectedSpaceId).trim() || DEFAULT_SPACE_ID;
      if (responseSpaceId !== currentSpaceIdRef.current) {
        setCurrentSpaceState(responseSpaceId);
      }
      setRedclawOnboardingState(
        bundle.onboardingState && typeof bundle.onboardingState === 'object'
          ? bundle.onboardingState as Record<string, unknown>
          : null
      );
      if (options?.preserveDraft && redclawProfileDirtyRef.current) {
        setRedclawProfileRoot(String(bundle.profileRoot || '').trim());
        return;
      }
      const files = bundle.files || {};
      const nextDraft: RedclawProfileDraft = {
        user: String(bundle.user || files.user || ''),
        creatorProfile: String(bundle.creatorProfile || files.creatorProfile || ''),
      };
      setRedclawProfileRoot(String(bundle.profileRoot || '').trim());
      setSavedRedclawProfileDraft(nextDraft);
      setRedclawProfileDraft(nextDraft);
      setRedclawProfileDirtyState(false);
      setRedclawProfileMessage(null);
    } catch (error) {
      if (requestId !== redclawProfileLoadRequestRef.current) return;
      console.error('Failed to load AI profile bundle', error);
      setRedclawProfileMessage({
        tone: 'error',
        text: `加载用户档案失败：${error instanceof Error ? error.message : String(error)}`,
      });
    } finally {
      if (requestId === redclawProfileLoadRequestRef.current) {
        setIsRedclawProfileLoading(false);
      }
    }
  }, [setRedclawProfileDirtyState]);

  const handleRedclawProfileDraftChange = useCallback((field: keyof RedclawProfileDraft, value: string) => {
    setRedclawProfileDraft((prev) => {
      const next = {
        ...prev,
        [field]: value,
      };
      const dirty = next.creatorProfile !== savedRedclawProfileDraft.creatorProfile;
      setRedclawProfileDirtyState(dirty);
      return next;
    });
    setRedclawProfileMessage(null);
    setStatus('idle');
  }, [savedRedclawProfileDraft.creatorProfile, savedRedclawProfileDraft.user, setRedclawProfileDirtyState]);

  const settingsLoadRequestRef = useRef(0);
  const fileIndexDashboardLoadRequestRef = useRef(0);
  const fileIndexDashboardInFlightRef = useRef<Promise<FileIndexDashboard | null> | null>(null);
  const fileIndexDashboardRefreshTimerRef = useRef<number | null>(null);
  const fileIndexDashboardCurrentRef = useRef<FileIndexDashboard | null>(
    initialFileIndexDashboardCache?.dashboard ?? null,
  );
  const fileIndexDashboardLoadedAtRef = useRef(initialFileIndexDashboardCache?.savedAt ?? 0);
  const assistantDaemonLogBufferRef = useRef<string[]>([]);
  const assistantDaemonLogFlushTimerRef = useRef<number | null>(null);
  const remoteTabWarmTimerRef = useRef<number | null>(null);
  const settingsActivationTimerRef = useRef<number | null>(null);
  const redclawProfileLoadRequestRef = useRef(0);
  const baseSettingsLoadedRef = useRef(false);
  const {
    setShowAiModelSettings,
    pendingOfficialAiPanelScrollRef,
    officialAiPanelRef,
    aiSourceDraftDirtyRef,
    aiModelSettingsDraftDirtyRef,
    aiSourceEditGenerationRef,
    aiModelSettingsEditGenerationRef,
    isDeprecatedEmptyOpenAiSource,
    officialAiSourcePlaceholder,
    isOfficialManagedSource,
    inferImageRoutingFromSource,
    pickCapabilityModelForSource,
    setAiSources,
    setDefaultAiSourceId,
    setActiveAiSourceId,
    setDetectedAiProtocol,
    setTranscriptionSourceId,
    setEmbeddingSourceId,
    setVisualIndexSourceId,
    setVideoAnalysisSourceId,
    setImageSourceId,
    setVoiceSourceId,
    setAiModelRoutes,
    clearAiSourceDraftDirty,
    clearAiModelSettingsDraftDirty,
    officialAiPanelEnabled,
    OfficialAiPanelComponent,
    setOfficialAiPanelComponent,
    aiSourceAutosaveTimerRef,
    buildAiSourcePersistenceSnapshot,
    officialAiSource,
    getAiSourceById,
    transcriptionSourceId,
    embeddingSourceId,
    visualIndexSourceId,
    videoAnalysisSourceId,
    imageSourceId,
    voiceSourceId,
    pickBestModelForSource,
    pickBestVisualIndexModelForSource,
    filterVideoAnalysisModels,
    getSourceModelList,
    pickBestVideoAnalysisModelForSource,
    aiModelRoutes,
    getRouteSource,
    showAiModelSettings,
    openCreateAiSourceModal,
    displayedAiSources,
    aiSourceExpandState,
    hasOfficialManagedSource,
    aiSourceModelExpandState,
    getAddedSourceModelList,
    officialAuthPending,
    officialAuthLoggedIn,
    getLocalGuideForSource,
    isLocalAiSource,
    handleToggleAiSourceExpand,
    handleDeleteAiSource,
    officialAuthNeedsLogin,
    updateAiSource,
    groupedAiPresets,
    handleToggleAiSourceModelExpand,
    openAddModelModal,
    handleSetSourceDefaultModel,
    handleRemoveSourceModel,
    activeAiSourceId,
    detectedAiProtocol,
    renderModelSettingsTableRow,
    chatSourceSelectValue,
    handleChatSourceChange,
    effectiveRouteModelValue,
    applyRouteModel,
    routeModelSelectOptions,
    chatRouteSourceModels,
    chatRouteSource,
    aiSources,
    handleLinkedSourceChange,
    setAiModelFormField,
    transcriptionSourceModels,
    routeScopeSourceValue,
    handleRouteScopeSourceChange,
    routeScopeSourceModels,
    routeScopeSource,
    isDashscopeImageTemplate,
    isCreateAiSourceModalOpen,
    closeCreateAiSourceModal,
    createAiSourceDraft,
    setCreateAiSourceDraft,
    handleCreateAiSource,
    addModelModalSource,
    closeAddModelModal,
    addModelModalRemoteModels,
    addModelModalDraft,
    setSourceModelDrafts,
    addModelModalCapability,
    setSourceModelCapabilityDrafts,
    handleAddSourceModel,
    addModelModalDraftTrimmed,
  } = useSettingsAi({ setFormData, formData, activeTab, setTestStatus, setTestMsg, baseSettingsLoadedRef, setStatus, baseSettingsLoadedRevision });

  const baseSettingsInFlightRef = useRef(false);
  const redclawProfileDirtyRef = useRef(false);
  const currentSpaceIdRef = useRef(DEFAULT_SPACE_ID);
  const tabWarmRef = useRef<Record<SettingsTab, boolean>>({
    general: false,
    ai: false,
    team: false,
    platforms: false,
    skills: false,
    mcp: false,
    tools: false,
    profile: false,
    remote: false,
    experimental: false,
  });
  const tabInFlightRef = useRef<Record<SettingsTab, boolean>>({
    general: false,
    ai: false,
    team: false,
    platforms: false,
    skills: false,
    mcp: false,
    tools: false,
    profile: false,
    remote: false,
    experimental: false,
  });

  // Tools State
  const [thrivePlugins, setThrivePlugins] = useState<ThrivePluginSummary[]>([]);
  const [thrivePluginMarketplace, setThrivePluginMarketplace] = useState<ThrivePluginMarketplaceItem[]>([]);
  const [codexPluginMarketplace, setCodexPluginMarketplace] = useState<CodexPluginMarketplaceItem[]>([]);
  const [thrivePluginMarketplaceLoading, setThrivePluginMarketplaceLoading] = useState(false);
  const [codexPluginMarketplaceLoading, setCodexPluginMarketplaceLoading] = useState(false);
  const [thrivePluginsLoading, setThrivePluginsLoading] = useState(false);
  const [thrivePluginBusyId, setThrivePluginBusyId] = useState('');
  const [thrivePluginStatusMessage, setThrivePluginStatusMessage] = useState('');
  const [thrivePluginRepoInput, setThrivePluginRepoInput] = useState('');
  const [mcpServers, setMcpServers] = useState<McpServerConfig[]>([]);
  const [mcpStatusMessage, setMcpStatusMessage] = useState('');
  const [isSyncingMcp, setIsSyncingMcp] = useState(false);
  const [mcpTestingId, setMcpTestingId] = useState('');
  const [mcpOauthState, setMcpOauthState] = useState<Record<string, { connected: boolean; tokenPath?: string }>>({});
  const [mcpLiveSessions, setMcpLiveSessions] = useState<McpSessionState[]>([]);
  const [mcpRuntimeItems, setMcpRuntimeItems] = useState<McpServerRuntimeItem[]>([]);
  const [mcpInspectingId, setMcpInspectingId] = useState('');
  const [mcpDraft, setMcpDraft] = useState<McpServerDraft | null>(null);
  const [mcpDraftOriginalId, setMcpDraftOriginalId] = useState('');
  const settingsMcpRuntimeMap = useMemo(
    () => Object.fromEntries(mcpRuntimeItems.map((item) => [item.server.id, item.session || null])) as Record<string, McpSessionState | null>,
    [mcpRuntimeItems],
  );

  // Update State
  const [appVersion, setAppVersion] = useState<string | null>(null);

  useEffect(() => {
    // 桌面端没有 web 身份，恒为管理员，跳过这次探测
    if (isTauriRuntime()) return;
    let cancelled = false;
    void (async () => {
      try {
        const flags = await window.ipcRenderer.runtimeFlags();
        if (!cancelled) setIsAdminUser(flags?.isAdmin !== false);
      } catch {
        if (!cancelled) setIsAdminUser(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!navigationTarget) return;
    // 普通用户没有「AI 模型」这一页：外部跳转（如侧栏「配置模型」）落到它时忽略
    const wantsAiTab = navigationTarget.tab === 'ai';
    if (navigationTarget.tab && !(wantsAiTab && !isAdminUser)) {
      setActiveTab(navigationTarget.tab);
    }
    if (!isAdminUser) return;
    if (wantsAiTab && navigationTarget.aiModelSubTab === 'custom') {
      setShowAiModelSettings(true);
    }
    if (wantsAiTab && navigationTarget.aiModelSubTab === 'login') {
      setShowAiModelSettings(false);
      pendingOfficialAiPanelScrollRef.current = true;
      window.setTimeout(() => {
        officialAiPanelRef.current?.scrollIntoView({ block: 'start', behavior: 'smooth' });
      }, 80);
    }
  }, [navigationTarget, isAdminUser]);

  const setAssistantDaemonDraft = useCallback((updater: SetStateAction<AssistantDaemonDraft>) => {
    setAssistantDaemonDraftDirty(true);
    setAssistantDaemonDraftState(updater);
  }, []);

  const replaceAssistantDaemonDraft = useCallback((nextDraft: AssistantDaemonDraft) => {
    setAssistantDaemonDraftDirty(false);
    setAssistantDaemonDraftState(nextDraft);
  }, []);

  useEffect(() => {
    if (!isActive) {
      return;
    }
    if (activeTab !== 'remote') {
      return;
    }

    const flushAssistantDaemonLogs = () => {
      assistantDaemonLogFlushTimerRef.current = null;
      const nextLines = assistantDaemonLogBufferRef.current;
      assistantDaemonLogBufferRef.current = [];
      if (!nextLines.length) return;
      setAssistantDaemonLogs((prev) => [...nextLines.reverse(), ...prev].slice(0, 20));
    };

    const handleDaemonStatus = (_: unknown, status: AssistantDaemonStatus) => {
      setAssistantDaemonStatus(status);
      setAssistantDaemonDraftState((prev) => {
        if (assistantDaemonBusy || assistantDaemonDraftDirty) return prev;
        return assistantDaemonStatusToDraft(status);
      });
    };
    const handleDaemonLog = (_: unknown, payload: { at?: string; level?: string; message?: string; details?: Record<string, unknown> }) => {
      const line = [
        payload?.at || new Date().toISOString(),
        payload?.level || 'info',
        payload?.message || '',
        payload?.details ? JSON.stringify(payload.details) : '',
      ].filter(Boolean).join(' | ');
      assistantDaemonLogBufferRef.current.push(line);
      if (assistantDaemonLogFlushTimerRef.current == null) {
        assistantDaemonLogFlushTimerRef.current = window.setTimeout(flushAssistantDaemonLogs, 300);
      }
    };
    window.ipcRenderer.assistantDaemon.onStatus(handleDaemonStatus);
    window.ipcRenderer.assistantDaemon.onLog(handleDaemonLog);
    return () => {
      window.ipcRenderer.assistantDaemon.offStatus(handleDaemonStatus);
      window.ipcRenderer.assistantDaemon.offLog(handleDaemonLog);
      if (assistantDaemonLogFlushTimerRef.current != null) {
        window.clearTimeout(assistantDaemonLogFlushTimerRef.current);
        assistantDaemonLogFlushTimerRef.current = null;
      }
      assistantDaemonLogBufferRef.current = [];
    };
  }, [activeTab, assistantDaemonBusy, assistantDaemonDraftDirty, isActive]);

  useEffect(() => {
    if (!mcpStatusMessage) return;
    const timer = window.setTimeout(() => setMcpStatusMessage(''), 2800);
    return () => window.clearTimeout(timer);
  }, [mcpStatusMessage]);

  useEffect(() => {
    if (activeTab !== 'mcp') return;
    void loadMcpRuntimeData();
    for (const server of mcpServers) {
      void handleRefreshMcpOAuth(server);
    }
  }, [activeTab, mcpServers]);

  const loadMcpRuntimeData = useCallback(async () => {
    try {
      const result = await window.ipcRenderer.mcp.list();
      if (!result?.success) return null;
      const servers = Array.isArray(result.servers) ? (result.servers as McpServerConfig[]) : [];
      setMcpServers((prev) => (
        JSON.stringify(prev) === JSON.stringify(servers) ? prev : servers
      ));
      setMcpLiveSessions(Array.isArray(result.sessions) ? (result.sessions as McpSessionState[]) : []);
      setMcpRuntimeItems(Array.isArray(result.items) ? (result.items as McpServerRuntimeItem[]) : []);
      return servers;
    } catch (error) {
      console.error('Failed to load MCP runtime state:', error);
      return null;
    }
  }, []);

  const persistMcpServers = useCallback(async (nextServers: McpServerConfig[], tip?: string) => {
    setIsSyncingMcp(true);
    try {
      const result = await window.ipcRenderer.mcp.save(nextServers);
      if (!result?.success) {
        setMcpStatusMessage(result?.error || 'MCP 配置保存失败');
        return false;
      }
      setMcpServers((result.servers || nextServers) as McpServerConfig[]);
      await loadMcpRuntimeData();
      if (tip) setMcpStatusMessage(tip);
      return true;
    } catch (error) {
      console.error('Failed to persist MCP servers:', error);
      setMcpStatusMessage('MCP 配置保存失败');
      return false;
    } finally {
      setIsSyncingMcp(false);
    }
  }, [loadMcpRuntimeData]);

  const handleAddMcpServer = () => {
    setMcpDraft(mcpDraftFromServer());
    setMcpDraftOriginalId('');
  };

  const handleDeleteMcpServer = async (serverId: string) => {
    const server = mcpServers.find((item) => item.id === serverId);
    if (server && !(await appConfirm(`确定删除 MCP Server "${server.name || server.id}" 吗？`, {
      title: '删除 MCP Server',
      confirmLabel: '删除',
      tone: 'danger',
    }))) {
      return;
    }
    const next = mcpServers.filter((item) => item.id !== serverId);
    await persistMcpServers(next, '已删除 MCP Server');
  };

  const handleUpdateMcpServer = (serverId: string, updater: (server: McpServerConfig) => McpServerConfig) => {
    setMcpServers((prev) => prev.map((server) => (server.id === serverId ? updater(server) : server)));
  };

  const handleToggleMcpServer = useCallback(async (server: McpServerConfig) => {
    const nextEnabled = !(server.enabled !== false);
    const next = mcpServers.map((item) => (
      item.id === server.id ? { ...item, enabled: nextEnabled } : item
    ));
    setMcpServers(next);
    const saved = await persistMcpServers(next, `${server.name || server.id} 已${nextEnabled ? '打开' : '关闭'}`);
    if (!saved) {
      setMcpServers(mcpServers);
    }
  }, [mcpServers, persistMcpServers]);

  const handleEditMcpServer = useCallback((server: McpServerConfig) => {
    setMcpDraft(mcpDraftFromServer(server));
    setMcpDraftOriginalId(server.id);
  }, []);

  const handleSaveMcpDraft = useCallback(async () => {
    if (!mcpDraft) return;
    const server = mcpServerFromDraft(mcpDraft);
    const next = mcpDraftOriginalId
      ? mcpServers.map((item) => (item.id === mcpDraftOriginalId ? server : item))
      : [...mcpServers, server];
    const saved = await persistMcpServers(next, mcpDraftOriginalId ? 'MCP Server 已保存' : '已新增 MCP Server');
    if (saved) {
      setMcpDraft(null);
      setMcpDraftOriginalId('');
    }
  }, [mcpDraft, mcpDraftOriginalId, mcpServers, persistMcpServers]);

  const handleCancelMcpDraft = useCallback(() => {
    setMcpDraft(null);
    setMcpDraftOriginalId('');
  }, []);

  const handleSaveMcpServers = async () => {
    await persistMcpServers(mcpServers, 'MCP 配置已保存');
  };

  const handleDiscoverAndImportMcp = useCallback(async () => {
    setIsSyncingMcp(true);
    try {
      const result = await window.ipcRenderer.mcp.importLocal();
      if (!result?.success) {
        setMcpStatusMessage(result?.error || '导入本机 MCP 配置失败');
        return;
      }
      setMcpServers((result.servers || []) as McpServerConfig[]);
      await loadMcpRuntimeData();
      setMcpStatusMessage(`已导入 ${result.imported || 0} 个 MCP Server（共 ${result.total || 0} 个）`);
    } catch (error) {
      console.error('Failed to import local MCP configs:', error);
      setMcpStatusMessage('导入本机 MCP 配置失败');
    } finally {
      setIsSyncingMcp(false);
    }
  }, [loadMcpRuntimeData]);

  const handleTestMcpServer = useCallback(async (server: McpServerConfig) => {
    setMcpTestingId(server.id);
    try {
      const result = await window.ipcRenderer.mcp.test(server);
      setMcpStatusMessage(`${server.name}：${result.message}`);
      await loadMcpRuntimeData();
    } catch (error) {
      console.error('Failed to test MCP server:', error);
      setMcpStatusMessage(`${server.name}：测试失败`);
    } finally {
      setMcpTestingId('');
    }
  }, [loadMcpRuntimeData]);

  const handleDisconnectMcpServer = useCallback(async (server: McpServerConfig) => {
    setMcpInspectingId(server.id);
    try {
      const result = await window.ipcRenderer.mcp.disconnect(server);
      if (result?.success) {
        setMcpLiveSessions(Array.isArray(result.sessions) ? (result.sessions as McpSessionState[]) : []);
        await loadMcpRuntimeData();
        setMcpStatusMessage(`${server.name}：连接已断开`);
      }
    } catch (error) {
      console.error('Failed to disconnect MCP server:', error);
      setMcpStatusMessage(`${server.name}：断开连接失败`);
    } finally {
      setMcpInspectingId('');
    }
  }, [loadMcpRuntimeData]);

  const handleDisconnectAllMcpSessions = useCallback(async () => {
    setMcpInspectingId('__all__');
    try {
      const result = await window.ipcRenderer.mcp.disconnectAll();
      if (result?.success) {
        setMcpLiveSessions(Array.isArray(result.sessions) ? (result.sessions as McpSessionState[]) : []);
        await loadMcpRuntimeData();
        setMcpStatusMessage('已断开全部 MCP 会话');
      }
    } catch (error) {
      console.error('Failed to disconnect all MCP sessions:', error);
      setMcpStatusMessage('断开全部 MCP 会话失败');
    } finally {
      setMcpInspectingId('');
    }
  }, [loadMcpRuntimeData]);

  const handleRefreshMcpOAuth = async (server: McpServerConfig) => {
    try {
      const result = await window.ipcRenderer.mcp.oauthStatus(server.id);
      if (!result?.success) return;
      setMcpOauthState((prev) => ({
        ...prev,
        [server.id]: {
          connected: Boolean(result.connected),
          tokenPath: result.tokenPath,
        },
      }));
    } catch (error) {
      console.error('Failed to query MCP oauth status:', error);
    }
  };

  const loadAppVersion = useCallback(async () => {
    try {
      const version = await window.ipcRenderer.getAppVersion();
      const normalizedVersion = typeof version === 'string'
        ? version.trim()
        : String(version || '').trim();
      setAppVersion(normalizedVersion || '未读取到版本号');
    } catch (e) {
      console.error('Failed to load app version:', e);
      setAppVersion('读取失败');
    }
  }, []);

  const isEmptyFileIndexDashboardFallback = useCallback((dashboard: FileIndexDashboard | null | undefined): boolean => {
    if (!dashboard) return true;
    const overall = dashboard.overall;
    return (dashboard.lanes || []).length === 0
      && (dashboard.scopes || []).length === 0
      && (!overall
        || (
          Number(overall.indexedFiles || 0) === 0
          && Number(overall.totalFiles || 0) === 0
          && Number(overall.failedFiles || 0) === 0
        ));
  }, []);

  const loadFileIndexDashboard = useCallback(async (options: { force?: boolean; background?: boolean } = {}) => {
    const cachedDashboard = fileIndexDashboardCurrentRef.current;
    const cacheAge = Date.now() - fileIndexDashboardLoadedAtRef.current;
    if (
      !options.force
      && cachedDashboard
      && cacheAge >= 0
      && cacheAge < FILE_INDEX_DASHBOARD_CACHE_TTL_MS
    ) {
      return cachedDashboard;
    }
    if (fileIndexDashboardInFlightRef.current) {
      return fileIndexDashboardInFlightRef.current;
    }

    const requestId = ++fileIndexDashboardLoadRequestRef.current;
    const shouldShowLoading = !options.background || !cachedDashboard;
    if (shouldShowLoading) {
      setIsFileIndexDashboardLoading(true);
    }

    let request: Promise<FileIndexDashboard | null>;
    request = (async () => {
      try {
        const dashboard = await window.ipcRenderer.knowledge.getFileIndexDashboard<FileIndexDashboard>();
        if (requestId !== fileIndexDashboardLoadRequestRef.current) {
          return fileIndexDashboardCurrentRef.current;
        }

        let nextDashboard = dashboard || fileIndexDashboardCurrentRef.current || null;
        if (
          isEmptyFileIndexDashboardFallback(dashboard)
          && fileIndexDashboardCurrentRef.current
          && !isEmptyFileIndexDashboardFallback(fileIndexDashboardCurrentRef.current)
        ) {
          nextDashboard = fileIndexDashboardCurrentRef.current;
        }

        if (nextDashboard) {
          fileIndexDashboardCurrentRef.current = nextDashboard;
          fileIndexDashboardLoadedAtRef.current = writeCachedFileIndexDashboard(nextDashboard);
          setFileIndexDashboard(nextDashboard);
        }
        return nextDashboard;
      } catch (error) {
        if (requestId === fileIndexDashboardLoadRequestRef.current) {
          console.error('Failed to load file index dashboard:', error);
        }
        return fileIndexDashboardCurrentRef.current;
      } finally {
        if (fileIndexDashboardInFlightRef.current === request) {
          fileIndexDashboardInFlightRef.current = null;
          if (shouldShowLoading) {
            setIsFileIndexDashboardLoading(false);
          }
        }
      }
    })();

    fileIndexDashboardInFlightRef.current = request;
    return request;
  }, [isEmptyFileIndexDashboardFallback]);

  const loadLoggingStatus = useCallback(async () => {
    try {
      const result = await window.ipcRenderer.logs.getStatus();
      setLogStatus(result || null);
    } catch (error) {
      console.error('Failed to load logging status', error);
    }
  }, []);

  const loadPendingDiagnosticReports = useCallback(async () => {
    try {
      const result = await window.ipcRenderer.logs.listPendingReports();
      setPendingDiagnosticReports(Array.isArray(result) ? result : []);
    } catch (error) {
      console.error('Failed to load pending diagnostic reports', error);
    }
  }, []);

  const handleExportDiagnosticBundle = useCallback(async (reportId?: string) => {
    setDiagnosticsActionBusy(reportId || 'manual-export');
    try {
      const result = await window.ipcRenderer.logs.exportBundle(reportId, {
        includeAdvancedContext: Boolean(formData.debug_log_enabled || formData.diagnostics_include_advanced_context),
      });
      if (!result?.success) {
        throw new Error(result?.error || '导出诊断包失败');
      }
      await appAlert(`诊断包已导出到：\n${result.path}`);
      await Promise.all([loadLoggingStatus(), loadPendingDiagnosticReports()]);
    } catch (error) {
      void appAlert(`导出诊断包失败：${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setDiagnosticsActionBusy(null);
    }
  }, [formData.debug_log_enabled, formData.diagnostics_include_advanced_context, loadLoggingStatus, loadPendingDiagnosticReports]);

  const handleOpenFeedbackReport = useCallback(() => {
    window.dispatchEvent(new CustomEvent('redbox:open-feedback-report', {
      detail: {
        sourcePage: 'settings',
        operation: 'manual_feedback',
      },
    }));
  }, []);

  const handleUploadPendingReport = useCallback(async (reportId: string) => {
    setDiagnosticsActionBusy(reportId);
    try {
      const result = await window.ipcRenderer.logs.uploadReport(reportId);
      if (!result?.success) {
        throw new Error(result?.error || '上传诊断报告失败');
      }
      await appAlert('诊断报告已上传。');
      await Promise.all([loadLoggingStatus(), loadPendingDiagnosticReports()]);
    } catch (error) {
      void appAlert(`上传诊断报告失败：${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setDiagnosticsActionBusy(null);
    }
  }, [loadLoggingStatus, loadPendingDiagnosticReports]);

  const handleDismissPendingReport = useCallback(async (reportId: string) => {
    setDiagnosticsActionBusy(reportId);
    try {
      const result = await window.ipcRenderer.logs.dismissReport(reportId);
      if (!result?.success) {
        throw new Error(result?.error || '删除待发送报告失败');
      }
      await Promise.all([loadLoggingStatus(), loadPendingDiagnosticReports()]);
    } catch (error) {
      void appAlert(`删除待发送报告失败：${error instanceof Error ? error.message : String(error)}`);
    } finally {
      setDiagnosticsActionBusy(null);
    }
  }, [loadLoggingStatus, loadPendingDiagnosticReports]);

  const persistDeveloperModeState = useCallback(async (enabled: boolean, unlockedAt: string | null) => {
    await window.ipcRenderer.saveSettings({
      developer_mode_enabled: enabled,
      developer_mode_unlocked_at: unlockedAt,
    });
  }, []);

  const expireDeveloperMode = useCallback(async () => {
    setFormData((prev) => ({
      ...prev,
      developer_mode_enabled: false,
      developer_mode_unlocked_at: '',
    }));
    try {
      await persistDeveloperModeState(false, null);
    } catch (error) {
      console.error('Failed to persist developer mode expiration', error);
    }
  }, [persistDeveloperModeState]);

  const handleVersionTap = useCallback(() => {
    setDeveloperVersionTapCount((prev) => {
      const next = prev + 1;
      if (next < DEVELOPER_MODE_UNLOCK_TAP_COUNT) {
        return next;
      }

      const unlockedAt = new Date().toISOString();
      setFormData((current) => ({
        ...current,
        developer_mode_enabled: true,
        developer_mode_unlocked_at: unlockedAt,
      }));
      void persistDeveloperModeState(true, unlockedAt);
      if (activeTab === 'tools') {
        void loadToolDiagnostics();
        void loadRuntimeDeveloperData();
      }
      void appAlert('开发者模式已开启（24 小时内有效）');
      return 0;
    });
  }, [activeTab, persistDeveloperModeState]);

  const handleShowCurrentReleaseNotes = useCallback(() => {
    window.dispatchEvent(new CustomEvent(SHOW_CURRENT_RELEASE_NOTES_EVENT, {
      detail: { version: appVersion || '2.0.0' },
    }));
  }, [appVersion]);

  const handleOpenDownloadPage = useCallback(async () => {
    try {
      const result = await window.ipcRenderer.openAppReleasePage(APP_BRAND.downloadUrl);
      if (!result?.success) {
        void appAlert(result?.error || '打开下载页面失败');
      }
    } catch (error) {
      console.error('Failed to open download page:', error);
      void appAlert('打开下载页面失败');
    }
  }, []);

  const loadSettings = useCallback(async (options?: { preserveViewState?: boolean; preserveRemoteModels?: boolean }) => {
    const preserveViewState = Boolean(options?.preserveViewState);
    const requestId = ++settingsLoadRequestRef.current;
    const hadLocalAiSourceDraft = aiSourceDraftDirtyRef.current;
    const hadLocalAiModelSettingsDraft = aiModelSettingsDraftDirtyRef.current;
    const requestAiSourceEditGeneration = aiSourceEditGenerationRef.current;
    const requestAiModelSettingsEditGeneration = aiModelSettingsEditGenerationRef.current;
    try {
      const settings = await window.ipcRenderer.getSettings();
      if (requestId !== settingsLoadRequestRef.current) return;
      if (
        preserveViewState
        && (
          hadLocalAiSourceDraft
          || hadLocalAiModelSettingsDraft
          || aiSourceDraftDirtyRef.current
          || aiModelSettingsDraftDirtyRef.current
          || requestAiSourceEditGeneration !== aiSourceEditGenerationRef.current
          || requestAiModelSettingsEditGeneration !== aiModelSettingsEditGenerationRef.current
        )
      ) {
        return;
      }
      if (settings) {
        const resolveLinkedSourceIdFromList = (params: {
          endpoint?: string;
          apiKey?: string;
          model?: string;
          fallbackId?: string;
          /** 是否允许回退到默认源。转写传 false —— 官方源不做转写兜底。 */
          allowFallback?: boolean;
        }): string => {
          const normalizedEndpoint = String(params.endpoint || '').trim();
          const normalizedApiKey = String(params.apiKey || '').trim();
          const normalizedModel = String(params.model || '').trim();
          let bestSourceId = '';
          let bestScore = -1;

          for (const source of sourceList) {
            let score = 0;
            const sourceEndpoint = String(source.baseURL || '').trim();
            const sourceApiKey = String(source.apiKey || '').trim();
            const sourceModels = [
              ...(source.models || []),
              source.model,
              ...(source.modelsMeta || []).map((item) => String(item?.id || '').trim()),
            ].filter(Boolean);
            if (normalizedEndpoint && sourceEndpoint === normalizedEndpoint) score += 4;
            if (normalizedApiKey && sourceApiKey && sourceApiKey === normalizedApiKey) score += 2;
            if (normalizedModel && sourceModels.includes(normalizedModel)) score += 1;
            if (score > bestScore) {
              bestScore = score;
              bestSourceId = source.id;
            }
          }

          if (bestScore > 0 && bestSourceId) return bestSourceId;
          // 回退到默认源：会把「用户手填的独立服务地址」在保存时整个覆盖成
          // 默认源的值（实测：ElevenLabs 被官方源覆盖 → 401/400）。
          // 转写不允许这个回退 —— 官方源不做转写兜底。
          if (params.allowFallback === false) return '';
          const fallbackId = String(params.fallbackId || '').trim();
          if (fallbackId && sourceList.some((source) => source.id === fallbackId)) return fallbackId;
          return normalizedDefaultId;
        };

        const requestedDefaultSourceId = canonicalizeOfficialAutoSourceId(String(settings.default_ai_source_id || '').trim());
        const prefersOfficialDefault = isOfficialAutoSourceId(requestedDefaultSourceId);
        let sourceList = parseAiSources(settings.ai_sources_json).filter((source) => !isDeprecatedEmptyOpenAiSource(source));
        if (!sourceList.length && !prefersOfficialDefault && (settings.api_endpoint || settings.api_key || settings.model_name)) {
          const inferredPresetId = inferPresetIdByEndpoint(settings.api_endpoint || '');
          sourceList = [{
            id: generateAiSourceId(),
            name: findAiPresetById(inferredPresetId)?.label || '导入供应商',
            presetId: inferredPresetId,
            baseURL: settings.api_endpoint || '',
            apiKey: settings.api_key || '',
            models: normalizeSourceModels([settings.model_name || '']),
            modelsMeta: normalizeAiModelDescriptors([settings.model_name || '']),
            model: settings.model_name || '',
            protocol: findAiPresetById(inferredPresetId)?.protocol || 'openai',
          }];
        }

        const loadedDefaultId = requestedDefaultSourceId || sourceList[0]?.id || OFFICIAL_AUTO_SOURCE_ID;
        const normalizedDefaultId = sourceList.some((source) => source.id === loadedDefaultId)
          ? loadedDefaultId
          : (loadedDefaultId === OFFICIAL_AUTO_SOURCE_ID ? OFFICIAL_AUTO_SOURCE_ID : (sourceList[0]?.id || OFFICIAL_AUTO_SOURCE_ID));
        const resolvedDefaultSource = sourceList.find((source) => source.id === normalizedDefaultId)
          || (isOfficialAutoSourceId(normalizedDefaultId) ? officialAiSourcePlaceholder : sourceList[0])
          || null;
        const resolvedTranscriptionSourceId = resolveLinkedSourceIdFromList({
          endpoint: String(settings.transcription_endpoint || settings.api_endpoint || '').trim(),
          apiKey: String(settings.transcription_key || settings.api_key || '').trim(),
          model: String(settings.transcription_model || '').trim(),
          fallbackId: normalizedDefaultId,
          // 转写不走官方源兜底：用户配了 ElevenLabs 这类独立 STT 时，
          // 回退到默认源会在保存时把地址/key 整个覆盖掉。
          allowFallback: false,
        });
        const resolvedEmbeddingSourceId = resolveLinkedSourceIdFromList({
          endpoint: String(settings.embedding_endpoint || settings.api_endpoint || '').trim(),
          apiKey: String(settings.embedding_key || settings.api_key || '').trim(),
          model: String(settings.embedding_model || '').trim(),
          fallbackId: normalizedDefaultId,
        });
        const resolvedVisualIndexSourceId = resolveLinkedSourceIdFromList({
          endpoint: String(settings.visual_index_endpoint || settings.api_endpoint || '').trim(),
          apiKey: String(settings.visual_index_api_key || settings.api_key || '').trim(),
          model: String(settings.visual_index_model || '').trim(),
          fallbackId: normalizedDefaultId,
        });
        const resolvedVideoAnalysisSourceId = resolveLinkedSourceIdFromList({
          endpoint: String(settings.video_analysis_endpoint || settings.api_endpoint || '').trim(),
          apiKey: String(settings.video_analysis_api_key || settings.api_key || '').trim(),
          model: String(settings.video_analysis_model || '').trim(),
          fallbackId: normalizedDefaultId,
        });
        const resolvedImageSourceId = resolveLinkedSourceIdFromList({
          endpoint: String(settings.image_endpoint || settings.api_endpoint || '').trim(),
          apiKey: String(settings.image_api_key || settings.api_key || '').trim(),
          model: String(settings.image_model || '').trim(),
          fallbackId: normalizedDefaultId,
        });
        const resolvedVoiceSourceId = resolveLinkedSourceIdFromList({
          endpoint: String(settings.voice_endpoint || settings.tts_endpoint || settings.api_endpoint || '').trim(),
          apiKey: String(settings.voice_api_key || settings.tts_api_key || settings.api_key || '').trim(),
          model: String(settings.voice_tts_model || settings.tts_model || '').trim(),
          fallbackId: normalizedDefaultId,
        });
        const unlockedAt = String(settings.developer_mode_unlocked_at || '').trim();
        const unlockedAtMs = unlockedAt ? Date.parse(unlockedAt) : NaN;
        const developerModeEnabled = Boolean(settings.developer_mode_enabled)
          && Number.isFinite(unlockedAtMs)
          && (Date.now() - unlockedAtMs) < DEVELOPER_MODE_TTL_MS;
        const loadedCliRuntimeExecutionMode = normalizeCliRuntimeExecutionMode(
          settings.cli_runtime_execution_mode,
        );
        const loadedModelRoutes = normalizeAiModelRoutes(settings.ai_model_routes_json);
        const routeSourceMode = (sourceId: string, fallback: AiModelRouteMode = 'custom'): AiModelRouteMode => {
          const normalizedSourceId = canonicalizeOfficialAutoSourceId(sourceId);
          if (isOfficialAutoSourceId(normalizedSourceId)) return 'official';
          if (fallback === 'disabled') return 'disabled';
          return normalizedSourceId ? 'custom' : fallback;
        };
        const firstCustomSourceIdFromList = sourceList.find((source) => !isOfficialManagedSource(source))?.id || '';
        const loadedChatRouteSourceId = canonicalizeOfficialAutoSourceId(String(loadedModelRoutes.chat.sourceId || '').trim());
        const resolvedChatSourceId = loadedModelRoutes.chat.mode === 'official'
          ? OFFICIAL_AUTO_SOURCE_ID
          : loadedChatRouteSourceId
            || (loadedModelRoutes.chat.mode === 'custom' ? firstCustomSourceIdFromList : '')
            || normalizedDefaultId
            || OFFICIAL_AUTO_SOURCE_ID;
        const resolvedChatSource = sourceList.find((source) => source.id === resolvedChatSourceId)
          || (isOfficialAutoSourceId(resolvedChatSourceId) ? officialAiSourcePlaceholder : null)
          || resolvedDefaultSource
          || null;
        const loadedVoiceTtsModel = String(loadedModelRoutes.voiceTts.model || settings.voice_tts_model || settings.tts_model || DEFAULT_VOICE_TTS_MODEL).trim();
        const loadedVoiceCloneModel = cloneModelForVoiceTtsModel(
          loadedVoiceTtsModel,
          String(loadedModelRoutes.voiceClone.model || settings.voice_clone_model || DEFAULT_VOICE_CLONE_MODEL).trim(),
        );
        const routeModelFirst = (routeModel: string, legacyModel: unknown) => (
          String(routeModel || legacyModel || '').trim()
        );
        const resolvedImageSource = sourceList.find((source) => source.id === resolvedImageSourceId) || null;
        const loadedImageRouting = resolvedImageSource
          ? inferImageRoutingFromSource(resolvedImageSource)
          : {
            provider: settings.image_provider || 'openai-compatible',
            template: settings.image_provider_template || '',
          };
        const loadedImageProvider = loadedImageRouting.provider || settings.image_provider || 'openai-compatible';
        const loadedImageTemplate = inferImageTemplateByProvider(
          loadedImageProvider,
          loadedImageRouting.template || settings.image_provider_template || '',
        );
        const loadedImageModel = pickCapabilityModelForSource(
          resolvedImageSource,
          routeModelFirst(loadedModelRoutes.image.model, settings.image_model),
          'image',
        );
        const nextModelRoutes: AiModelRoutes = {
          ...loadedModelRoutes,
          chat: {
            ...loadedModelRoutes.chat,
            mode: routeSourceMode(resolvedChatSourceId, loadedModelRoutes.chat.mode),
            sourceId: resolvedChatSourceId,
            model: routeModelFirst(loadedModelRoutes.chat.model, resolvedChatSource?.model || settings.model_name),
          },
          wander: {
            ...loadedModelRoutes.wander,
            model: routeModelFirst(loadedModelRoutes.wander.model, settings.model_name_wander),
          },
          team: {
            ...loadedModelRoutes.team,
            model: routeModelFirst(loadedModelRoutes.team.model, settings.model_name_chatroom),
          },
          knowledge: {
            ...loadedModelRoutes.knowledge,
            model: routeModelFirst(loadedModelRoutes.knowledge.model, settings.model_name_knowledge),
          },
          redclaw: {
            ...loadedModelRoutes.redclaw,
            model: routeModelFirst(loadedModelRoutes.redclaw.model, settings.model_name_redclaw),
          },
          transcription: {
            ...loadedModelRoutes.transcription,
            mode: routeSourceMode(resolvedTranscriptionSourceId, loadedModelRoutes.transcription.mode),
            sourceId: resolvedTranscriptionSourceId,
            model: routeModelFirst(loadedModelRoutes.transcription.model, settings.transcription_model),
          },
          embedding: {
            ...loadedModelRoutes.embedding,
            mode: routeSourceMode(resolvedEmbeddingSourceId, loadedModelRoutes.embedding.mode),
            sourceId: resolvedEmbeddingSourceId,
            model: routeModelFirst(loadedModelRoutes.embedding.model, settings.embedding_model),
          },
          image: {
            ...loadedModelRoutes.image,
            mode: routeSourceMode(resolvedImageSourceId, loadedModelRoutes.image.mode),
            sourceId: resolvedImageSourceId,
            model: loadedImageModel,
          },
          visualIndex: {
            ...loadedModelRoutes.visualIndex,
            mode: routeSourceMode(resolvedVisualIndexSourceId, loadedModelRoutes.visualIndex.mode === 'disabled' ? 'official' : loadedModelRoutes.visualIndex.mode),
            sourceId: resolvedVisualIndexSourceId,
            model: routeModelFirst(loadedModelRoutes.visualIndex.model, settings.visual_index_model),
          },
          videoAnalysis: {
            ...loadedModelRoutes.videoAnalysis,
            mode: routeSourceMode(resolvedVideoAnalysisSourceId, loadedModelRoutes.videoAnalysis.mode === 'disabled' ? 'official' : loadedModelRoutes.videoAnalysis.mode),
            sourceId: resolvedVideoAnalysisSourceId,
            model: routeModelFirst(loadedModelRoutes.videoAnalysis.model, settings.video_analysis_model),
          },
          voiceTts: {
            ...loadedModelRoutes.voiceTts,
            mode: routeSourceMode(resolvedVoiceSourceId, loadedModelRoutes.voiceTts.mode),
            sourceId: resolvedVoiceSourceId,
            model: loadedVoiceTtsModel,
          },
          voiceClone: {
            ...loadedModelRoutes.voiceClone,
            mode: 'official',
            sourceId: OFFICIAL_AUTO_SOURCE_ID,
            model: loadedVoiceCloneModel,
          },
        };

        setCurrentSpaceState(
          (settings as { active_space_id?: string; activeSpaceId?: string }).active_space_id
          || (settings as { active_space_id?: string; activeSpaceId?: string }).activeSpaceId
        );

        setAiSources(sourceList);
        setDefaultAiSourceId(normalizedDefaultId);
        setActiveAiSourceId((prevActiveId) => {
          if (!preserveViewState) return resolvedChatSourceId;
          const currentActiveId = canonicalizeOfficialAutoSourceId(prevActiveId);
          if (isOfficialAutoSourceId(currentActiveId)) {
            return currentActiveId;
          }
          if (currentActiveId && sourceList.some((source) => source.id === currentActiveId)) {
            return currentActiveId;
          }
          return resolvedChatSourceId;
        });
        setDetectedAiProtocol((resolvedChatSource?.protocol || findAiPresetById(resolvedChatSource?.presetId || '')?.protocol || 'openai') as AiProtocol);
        const registryMcpServers = await loadMcpRuntimeData();
        if (!registryMcpServers) {
          setMcpServers(parseMcpServers(settings.mcp_servers_json));
        }
        setCliRuntimeExecutionMode(loadedCliRuntimeExecutionMode);
        setTranscriptionSourceId(resolvedTranscriptionSourceId);
        setEmbeddingSourceId(resolvedEmbeddingSourceId);
        setVisualIndexSourceId(resolvedVisualIndexSourceId);
        setVideoAnalysisSourceId(resolvedVideoAnalysisSourceId);
        setImageSourceId(resolvedImageSourceId);
        setVoiceSourceId(resolvedVoiceSourceId);
        setAiModelRoutes(nextModelRoutes);
        setNotificationSettings(parseNotificationSettings(settings.notifications_json));
        clearAiSourceDraftDirty();
        clearAiModelSettingsDraftDirty();

        setFormData({
          api_endpoint: resolvedChatSource?.baseURL || '',
          api_key: resolvedChatSource?.apiKey || '',
          model_name: nextModelRoutes.chat.model || resolvedChatSource?.model || '',
          workspace_dir: settings.workspace_dir || '',
          transcription_model: nextModelRoutes.transcription.model || '',
          transcription_endpoint: settings.transcription_endpoint || '',
          transcription_key: settings.transcription_key || '',
          embedding_endpoint: settings.embedding_endpoint || '',
          embedding_key: settings.embedding_key || '',
          embedding_model: nextModelRoutes.embedding.model || '',
          visual_index_enabled: Boolean(settings.visual_index_enabled),
          visual_index_provider: settings.visual_index_provider || 'openai-compatible',
          visual_index_endpoint: settings.visual_index_endpoint || '',
          visual_index_api_key: settings.visual_index_api_key || '',
          visual_index_model: nextModelRoutes.visualIndex.model || '',
          visual_index_prompt_version: normalizeVisualIndexPromptVersion(settings.visual_index_prompt_version),
          visual_index_timeout_seconds: String(settings.visual_index_timeout_seconds || 90),
          visual_index_max_image_edge: String(settings.visual_index_max_image_edge || 1536),
          visual_index_skip_small_images: settings.visual_index_skip_small_images !== false,
          visual_index_pdf_max_pages: String(settings.visual_index_pdf_max_pages || 12),
          visual_index_pdf_render_dpi: String(settings.visual_index_pdf_render_dpi || 144),
          visual_index_concurrency: String(settings.visual_index_concurrency || 1),
          video_analysis_enabled: DEFAULT_VIDEO_ANALYSIS_ENABLED,
          video_analysis_endpoint: settings.video_analysis_endpoint || '',
          video_analysis_api_key: settings.video_analysis_api_key || '',
          video_analysis_model: nextModelRoutes.videoAnalysis.model || '',
          video_analysis_protocol: settings.video_analysis_protocol || 'gemini',
          video_analysis_max_direct_video_bytes: String(settings.video_analysis_max_direct_video_bytes || 64 * 1024 * 1024),
          docling_endpoint: settings.docling_endpoint || settings.parser_docling_endpoint || '',
          tika_endpoint: settings.tika_endpoint || settings.parser_tika_endpoint || '',
          unstructured_endpoint: settings.unstructured_endpoint || settings.parser_unstructured_endpoint || '',
          parser_api_key: settings.parser_api_key || '',
          parser_timeout_seconds: String(settings.parser_timeout_seconds || 90),
          rerank_endpoint: settings.rerank_endpoint || settings.cross_encoder_rerank_endpoint || '',
          rerank_api_key: settings.rerank_api_key || '',
          rerank_model: settings.rerank_model || '',
          rerank_timeout_seconds: String(settings.rerank_timeout_seconds || 30),
          image_provider: loadedImageProvider,
          image_endpoint: settings.image_endpoint || resolveDefaultImageEndpoint(
            loadedImageProvider,
            loadedImageTemplate || 'openai-images'
          ),
          image_api_key: settings.image_api_key || '',
          image_provider_template: loadedImageTemplate,
          image_model: (() => {
            if (loadedImageTemplate === 'dashscope-wan-native') {
              return DASHSCOPE_LOCKED_IMAGE_MODEL;
            }
            return loadedImageModel;
          })(),
          voice_endpoint: settings.voice_endpoint || settings.tts_endpoint || '',
          voice_api_key: settings.voice_api_key || settings.tts_api_key || '',
          voice_tts_model: loadedVoiceTtsModel,
          tts_model: loadedVoiceTtsModel,
          voice_clone_model: loadedVoiceCloneModel,
          video_endpoint: REDBOX_OFFICIAL_VIDEO_BASE_URL,
          video_api_key: settings.video_api_key || '',
          video_model: settings.video_model || REDBOX_OFFICIAL_VIDEO_MODELS['text-to-video'],
          image_aspect_ratio: settings.image_aspect_ratio || '3:4',
          image_size: '',
          image_quality: settings.image_quality === 'low' || settings.image_quality === 'medium' || settings.image_quality === 'high' ? settings.image_quality : 'medium',
          model_name_wander: nextModelRoutes.wander.model || '',
          model_name_chatroom: nextModelRoutes.team.model || '',
          model_name_knowledge: nextModelRoutes.knowledge.model || '',
          model_name_redclaw: nextModelRoutes.redclaw.model || '',
          proxy_enabled: Boolean(settings.proxy_enabled),
          proxy_url: settings.proxy_url || '',
          proxy_bypass: settings.proxy_bypass || 'localhost,127.0.0.1,::1',
          redclaw_compact_target_tokens: String(settings.redclaw_compact_target_tokens || 256000),
          chat_max_tokens_default: sanitizeChatMaxTokensInput(String(settings.chat_max_tokens_default || DEFAULT_CHAT_MAX_TOKENS), DEFAULT_CHAT_MAX_TOKENS),
          chat_max_tokens_deepseek: sanitizeChatMaxTokensInput(String(settings.chat_max_tokens_deepseek || DEFAULT_CHAT_MAX_TOKENS_DEEPSEEK), DEFAULT_CHAT_MAX_TOKENS_DEEPSEEK),
          wander_deep_think_enabled: Boolean(settings.wander_deep_think_enabled),
          debug_log_enabled: Boolean(settings.debug_log_enabled),
          diagnostics_upload_consent: settings.diagnostics_upload_consent === 'none' ? 'none' : 'approved',
          diagnostics_include_advanced_context: Boolean(settings.diagnostics_include_advanced_context),
          diagnostics_auto_send_same_crash: Boolean(settings.diagnostics_auto_send_same_crash),
          diagnostics_last_prompted_at: String(settings.diagnostics_last_prompted_at || ''),
          analytics_consent: settings.analytics_consent === 'none' ? 'none' : 'approved',
          release_log_retention_days: String(settings.release_log_retention_days || 7),
          release_log_max_file_mb: String(settings.release_log_max_file_mb || 10),
          cli_runtime_execution_mode: loadedCliRuntimeExecutionMode,
          developer_mode_enabled: developerModeEnabled,
          developer_mode_unlocked_at: developerModeEnabled ? unlockedAt : '',
          ai_model_routes_json: JSON.stringify(nextModelRoutes),
          ecommerce_platforms_json: serializeEcommercePlatformsSettings(
            normalizeEcommercePlatformsSettings(settings.ecommerce_platforms_json)
          ),
        });

        if (Boolean(settings.developer_mode_enabled) && !developerModeEnabled) {
          void persistDeveloperModeState(false, null);
        }
      } else {
        if (requestId !== settingsLoadRequestRef.current) return;
        setCurrentSpaceState(DEFAULT_SPACE_ID);
        setAiSources([]);
        setDefaultAiSourceId(OFFICIAL_AUTO_SOURCE_ID);
        setActiveAiSourceId((prevActiveId) => {
          const currentActiveId = canonicalizeOfficialAutoSourceId(prevActiveId);
          if (preserveViewState && isOfficialAutoSourceId(currentActiveId)) {
            return currentActiveId;
          }
          return OFFICIAL_AUTO_SOURCE_ID;
        });
        setDetectedAiProtocol('openai');
        setMcpServers([]);
        setCliRuntimeExecutionMode('host_compatible');
        setAiModelRoutes(DEFAULT_AI_MODEL_ROUTES);
        setNotificationSettings(DEFAULT_NOTIFICATION_SETTINGS);
        clearAiSourceDraftDirty();
        clearAiModelSettingsDraftDirty();
      }
    } catch (e) {
      if (requestId !== settingsLoadRequestRef.current) return;
      console.error("Failed to load settings", e);
    }
  }, [clearAiModelSettingsDraftDirty, clearAiSourceDraftDirty, inferImageRoutingFromSource, isDeprecatedEmptyOpenAiSource, loadMcpRuntimeData, officialAiSourcePlaceholder, persistDeveloperModeState, pickCapabilityModelForSource, setCurrentSpaceState]);

  const reloadCustomAiSettings = useCallback(async (options?: { preserveViewState?: boolean; preserveRemoteModels?: boolean }) => {
    await loadSettings({
      preserveViewState: true,
      preserveRemoteModels: true,
      ...options,
    });
  }, [loadSettings]);

  useEffect(() => {
    if (!formData.developer_mode_enabled || !formData.developer_mode_unlocked_at) {
      return;
    }
    const unlockedAtMs = Date.parse(formData.developer_mode_unlocked_at);
    if (!Number.isFinite(unlockedAtMs)) {
      void expireDeveloperMode();
      return;
    }
    const remaining = DEVELOPER_MODE_TTL_MS - (Date.now() - unlockedAtMs);
    if (remaining <= 0) {
      void expireDeveloperMode();
      return;
    }
    const timer = window.setTimeout(() => {
      void expireDeveloperMode();
    }, remaining);
    return () => window.clearTimeout(timer);
  }, [expireDeveloperMode, formData.developer_mode_enabled, formData.developer_mode_unlocked_at]);

  const loadThrivePlugins = useCallback(async () => {
    setThrivePluginsLoading(true);
    try {
      const result = await window.ipcRenderer.plugins.list();
      setThrivePlugins(Array.isArray(result.plugins) ? result.plugins : []);
      setThrivePluginStatusMessage(result.success === false && result.error ? result.error : '');
    } catch (error) {
      console.error('Failed to load Thrive plugins', error);
      setThrivePluginStatusMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setThrivePluginsLoading(false);
    }
  }, []);

  const loadThrivePluginMarketplace = useCallback(async () => {
    setThrivePluginMarketplaceLoading(true);
    try {
      const result = await window.ipcRenderer.plugins.marketplace();
      if (result.success === false) {
        throw new Error(result.error || '插件市场加载失败');
      }
      setThrivePluginMarketplace(Array.isArray(result.plugins) ? result.plugins : []);
      setThrivePluginStatusMessage('');
    } catch (error) {
      console.error('Failed to load Thrive plugin marketplace', error);
      setThrivePluginMarketplace([]);
      setThrivePluginStatusMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setThrivePluginMarketplaceLoading(false);
    }
  }, []);

  const loadCodexPluginMarketplace = useCallback(async () => {
    setCodexPluginMarketplaceLoading(true);
    try {
      const result = await window.ipcRenderer.plugins.codexMarketplace();
      if (result.success === false) {
        throw new Error(result.error || 'Codex 插件市场加载失败');
      }
      setCodexPluginMarketplace(Array.isArray(result.plugins) ? result.plugins : []);
      setThrivePluginStatusMessage('');
    } catch (error) {
      console.error('Failed to load Codex plugin marketplace', error);
      setCodexPluginMarketplace([]);
      setThrivePluginStatusMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setCodexPluginMarketplaceLoading(false);
    }
  }, []);

  const handleInstallThriveMarketplacePlugin = useCallback(async (plugin: ThrivePluginMarketplaceItem) => {
    setThrivePluginBusyId(plugin.installedPluginId || plugin.id);
    setThrivePluginStatusMessage(`正在安装 ${plugin.displayName || plugin.name}`);
    try {
      const result = await window.ipcRenderer.plugins.installMarketplace({
        id: plugin.id,
        repo: plugin.repo,
        version: plugin.version || undefined,
        packageUrl: plugin.packageUrl || undefined,
      });
      if (result.success === false) {
        throw new Error(result.error || '插件安装失败');
      }
      setThrivePluginStatusMessage(result.plugin ? `已安装 ${result.plugin.displayName}` : '插件已安装');
      await Promise.all([
        loadThrivePlugins(),
        loadThrivePluginMarketplace(),
      ]);
    } catch (error) {
      console.error('Failed to install Thrive marketplace plugin', error);
      setThrivePluginStatusMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setThrivePluginBusyId('');
    }
  }, [loadThrivePluginMarketplace, loadThrivePlugins]);

  const handleInstallCodexMarketplacePlugin = useCallback(async (plugin: CodexPluginMarketplaceItem) => {
    if (!plugin.sourceRoot && !plugin.remotePluginId) {
      setThrivePluginStatusMessage('Codex 插件缺少可安装来源');
      return;
    }
    setThrivePluginBusyId(plugin.installedPluginId || plugin.id);
    setThrivePluginStatusMessage(`正在安装 ${plugin.displayName || plugin.name}`);
    try {
      const result = plugin.sourceRoot
        ? await window.ipcRenderer.plugins.installCodex({
          path: plugin.sourceRoot,
          pluginName: plugin.name,
        })
        : await window.ipcRenderer.plugins.installCodex({
          remotePluginId: plugin.remotePluginId || undefined,
          remoteMarketplaceName: plugin.sourceLabel,
          pluginName: plugin.name,
        });
      if (result.success === false) {
        throw new Error(result.error || 'Codex 插件安装失败');
      }
      setThrivePluginStatusMessage(result.plugin ? `已安装 ${result.plugin.displayName}` : 'Codex 插件已安装');
      await Promise.all([
        loadThrivePlugins(),
        loadCodexPluginMarketplace(),
      ]);
    } catch (error) {
      console.error('Failed to install Codex marketplace plugin', error);
      setThrivePluginStatusMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setThrivePluginBusyId('');
    }
  }, [loadCodexPluginMarketplace, loadThrivePlugins]);

  const handleInstallThrivePluginFromRepo = useCallback(async () => {
    const repo = thrivePluginRepoInput
      .trim()
      .replace(/^https:\/\/github\.com\//, '')
      .replace(/\/$/, '')
      .replace(/\.git$/, '');
    if (!repo) {
      setThrivePluginStatusMessage('请输入 GitHub 仓库，例如 owner/codex-plugin');
      return;
    }
    const id = repo.split('/').pop() || repo;
    setThrivePluginBusyId(`repo:${repo}`);
    setThrivePluginStatusMessage(`正在安装 ${repo}`);
    try {
      const result = await window.ipcRenderer.plugins.installMarketplace({ id, repo });
      if (result.success === false) {
        throw new Error(result.error || '插件安装失败');
      }
      setThrivePluginRepoInput('');
      setThrivePluginStatusMessage(result.plugin ? `已安装 ${result.plugin.displayName}` : '插件已安装');
      await Promise.all([
        loadThrivePlugins(),
        loadThrivePluginMarketplace(),
      ]);
    } catch (error) {
      console.error('Failed to install Thrive plugin from repo', error);
      setThrivePluginStatusMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setThrivePluginBusyId('');
    }
  }, [loadThrivePluginMarketplace, loadThrivePlugins, thrivePluginRepoInput]);

  const handleToggleThrivePlugin = useCallback(async (plugin: ThrivePluginSummary) => {
    setThrivePluginBusyId(plugin.id);
    setThrivePluginStatusMessage(plugin.enabled ? `正在停用 ${plugin.displayName}` : `正在启用 ${plugin.displayName}`);
    try {
      const result = await window.ipcRenderer.plugins.setEnabled({
        pluginId: plugin.id,
        enabled: !plugin.enabled,
      });
      if (result.success === false) {
        throw new Error(result.error || '插件状态更新失败');
      }
      setThrivePluginStatusMessage(result.plugin ? `${result.plugin.displayName} 已${result.plugin.enabled ? '启用' : '停用'}` : '插件状态已更新');
      await loadThrivePlugins();
    } catch (error) {
      console.error('Failed to toggle Thrive plugin', error);
      setThrivePluginStatusMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setThrivePluginBusyId('');
    }
  }, [loadThrivePlugins]);

  const handleUninstallThrivePlugin = useCallback(async (plugin: ThrivePluginSummary) => {
    const confirmed = await appConfirm(`卸载 ${APP_BRAND.displayName} 插件”${plugin.displayName}”？\n\n插件缓存会被删除，插件数据目录会保留。`);
    if (!confirmed) return;
    setThrivePluginBusyId(plugin.id);
    setThrivePluginStatusMessage(`正在卸载 ${plugin.displayName}`);
    try {
      const result = await window.ipcRenderer.plugins.uninstall({ pluginId: plugin.id });
      if (result.success === false) {
        throw new Error(result.error || '插件卸载失败');
      }
      setThrivePluginStatusMessage(`${plugin.displayName} 已卸载`);
      await loadThrivePlugins();
    } catch (error) {
      console.error('Failed to uninstall Thrive plugin', error);
      setThrivePluginStatusMessage(error instanceof Error ? error.message : String(error));
    } finally {
      setThrivePluginBusyId('');
    }
  }, [loadThrivePlugins]);

  const handleOpenThrivePluginDataDir = useCallback(async (pluginId?: string) => {
    try {
      const result = await window.ipcRenderer.plugins.openDataDir(pluginId ? { pluginId } : {});
      if (result.success === false) {
        throw new Error(result.error || '打开插件数据目录失败');
      }
      setThrivePluginStatusMessage(result.path ? `已打开 ${result.path}` : '已打开插件数据目录');
    } catch (error) {
      console.error('Failed to open Thrive plugin data dir', error);
      setThrivePluginStatusMessage(error instanceof Error ? error.message : String(error));
    }
  }, []);

  useEffect(() => {
    if (!isActive || activeTab !== 'tools') return;
    let refreshTimer: number | null = null;
    const scheduleRefresh = () => {
      if (refreshTimer != null) {
        window.clearTimeout(refreshTimer);
      }
      refreshTimer = window.setTimeout(() => {
        refreshTimer = null;
        void loadCliRuntimeDashboard({ silent: true });
      }, 450);
    };
    const handleRuntimeEvent = (_event: unknown, envelope?: unknown) => {
      const record = envelope && typeof envelope === 'object' ? envelope as Record<string, unknown> : {};
      const eventType = String(record.eventType || '').trim();
      if (eventType.startsWith('runtime:cli-')) {
        scheduleRefresh();
      }
    };
    window.ipcRenderer.runtime.onEvent(handleRuntimeEvent as (...args: unknown[]) => void);
    return () => {
      window.ipcRenderer.runtime.offEvent(handleRuntimeEvent as (...args: unknown[]) => void);
      if (refreshTimer != null) {
        window.clearTimeout(refreshTimer);
      }
    };
  }, [activeTab, isActive, loadCliRuntimeDashboard]);

  const withTimeout = useCallback(<T,>(task: Promise<T>, timeoutMs: number, label: string): Promise<T> => {
    return new Promise<T>((resolve, reject) => {
      const timer = window.setTimeout(() => {
        reject(new Error(label));
      }, timeoutMs);
      task.then((value) => {
        window.clearTimeout(timer);
        resolve(value);
      }).catch((error) => {
        window.clearTimeout(timer);
        reject(error);
      });
    });
  }, []);

  const loadAssistantDaemonStatus = useCallback(async (options?: {
    timeoutMs?: number;
    suppressAlert?: boolean;
  }) => {
    try {
      const request = window.ipcRenderer.assistantDaemon.getStatus() as Promise<AssistantDaemonStatus>;
      const status = typeof options?.timeoutMs === 'number' && options.timeoutMs > 0
        ? await withTimeout(request, options.timeoutMs, '远程连接状态加载超时')
        : await request;
      setAssistantDaemonStatus(status);
      replaceAssistantDaemonDraft(assistantDaemonStatusToDraft(status));
    } catch (error) {
      console.error('Failed to load assistant daemon status', error);
      if (!options?.suppressAlert) {
        void appAlert(`加载远程连接状态失败：${String(error)}`);
      }
    }
  }, [replaceAssistantDaemonDraft, withTimeout]);

  const scheduleRemoteTabWarmup = useCallback(() => {
    if (remoteTabWarmTimerRef.current != null) {
      window.clearTimeout(remoteTabWarmTimerRef.current);
    }
    remoteTabWarmTimerRef.current = window.setTimeout(() => {
      remoteTabWarmTimerRef.current = null;
      void loadAssistantDaemonStatus({
        timeoutMs: 1500,
        suppressAlert: true,
      });
    }, 0);
  }, [loadAssistantDaemonStatus]);

  const buildAssistantDaemonPayload = useCallback(() => ({
    enabled: assistantDaemonDraft.enabled,
    autoStart: assistantDaemonDraft.autoStart,
    keepAliveWhenNoWindow: assistantDaemonDraft.keepAliveWhenNoWindow,
    host: String(assistantDaemonDraft.host || '').trim(),
    port: Number(assistantDaemonDraft.port || 0) || undefined,
    feishu: {
      enabled: assistantDaemonDraft.feishu.enabled,
      receiveMode: assistantDaemonDraft.feishu.receiveMode,
      endpointPath: String(assistantDaemonDraft.feishu.endpointPath || '').trim(),
      verificationToken: String(assistantDaemonDraft.feishu.verificationToken || '').trim() || undefined,
      encryptKey: String(assistantDaemonDraft.feishu.encryptKey || '').trim() || undefined,
      appId: String(assistantDaemonDraft.feishu.appId || '').trim() || undefined,
      appSecret: String(assistantDaemonDraft.feishu.appSecret || '').trim() || undefined,
      replyUsingChatId: assistantDaemonDraft.feishu.replyUsingChatId,
    },
    relay: {
      enabled: assistantDaemonDraft.relay.enabled,
      endpointPath: String(assistantDaemonDraft.relay.endpointPath || '').trim(),
      authToken: String(assistantDaemonDraft.relay.authToken || '').trim() || undefined,
    },
    acpGateway: {
      enabled: assistantDaemonDraft.acpGateway.enabled,
      requireToken: assistantDaemonDraft.acpGateway.requireToken,
      localOnly: assistantDaemonDraft.acpGateway.localOnly,
      endpointPath: String(assistantDaemonDraft.acpGateway.endpointPath || '').trim() || '/acp/v1',
      manifestPath: String(assistantDaemonDraft.acpGateway.manifestPath || '').trim() || '/acp/v1/manifest',
      guidePath: String(assistantDaemonDraft.acpGateway.guidePath || '').trim() || '/acp/v1/guide',
      defaultRuntimeMode: String(assistantDaemonDraft.acpGateway.defaultRuntimeMode || '').trim() || 'default',
      defaultClientLabel: String(assistantDaemonDraft.acpGateway.defaultClientLabel || '').trim() || 'External Agent',
    },
    weixin: {
      enabled: assistantDaemonDraft.weixin.enabled,
      endpointPath: String(assistantDaemonDraft.weixin.endpointPath || '').trim(),
      authToken: String(assistantDaemonDraft.weixin.authToken || '').trim() || undefined,
      accountId: String(assistantDaemonDraft.weixin.accountId || '').trim() || undefined,
      autoStartSidecar: assistantDaemonDraft.weixin.autoStartSidecar,
      cursorFile: String(assistantDaemonDraft.weixin.cursorFile || '').trim() || undefined,
      sidecarCommand: String(assistantDaemonDraft.weixin.sidecarCommand || '').trim() || undefined,
      sidecarArgs: String(assistantDaemonDraft.weixin.sidecarArgs || '').trim()
        ? String(assistantDaemonDraft.weixin.sidecarArgs || '').trim().split(/\s+/)
        : undefined,
      sidecarCwd: String(assistantDaemonDraft.weixin.sidecarCwd || '').trim() || undefined,
      sidecarEnv: parseEnvText(assistantDaemonDraft.weixin.sidecarEnvText || ''),
    },
  }), [assistantDaemonDraft]);

  const handleSaveAssistantDaemonConfig = useCallback(async () => {
    setAssistantDaemonBusy(true);
    try {
      const status = await window.ipcRenderer.assistantDaemon.setConfig(buildAssistantDaemonPayload()) as AssistantDaemonStatus;
      setAssistantDaemonStatus(status);
      replaceAssistantDaemonDraft(assistantDaemonStatusToDraft(status));
    } catch (error) {
      console.error('Failed to save assistant daemon config', error);
      void appAlert(`保存后台通信配置失败：${String(error)}`);
    } finally {
      setAssistantDaemonBusy(false);
    }
  }, [buildAssistantDaemonPayload, replaceAssistantDaemonDraft]);

  const handleCreateAssistantDaemonAcpClient = useCallback(async (name: string, kind: string) => {
    setAssistantDaemonBusy(true);
    try {
      const result = await window.ipcRenderer.assistantDaemon.createAcpClient({
        name: String(name || '').trim() || 'External Agent',
        kind: String(kind || '').trim() || 'generic_agent',
      });
      const status = result?.status as AssistantDaemonStatus | undefined;
      if (status) {
        setAssistantDaemonStatus(status);
        replaceAssistantDaemonDraft(assistantDaemonStatusToDraft(status));
      } else {
        await loadAssistantDaemonStatus({ suppressAlert: true });
      }
      setAssistantDaemonAcpToken(String(result?.token || ''));
    } catch (error) {
      console.error('Failed to create ACP client', error);
      void appAlert(`创建 ACP 客户端失败：${String(error)}`);
    } finally {
      setAssistantDaemonBusy(false);
    }
  }, [loadAssistantDaemonStatus, replaceAssistantDaemonDraft]);

  const handleRevokeAssistantDaemonAcpClient = useCallback(async (clientId: string) => {
    const safeClientId = String(clientId || '').trim();
    if (!safeClientId) return;
    setAssistantDaemonBusy(true);
    try {
      const result = await window.ipcRenderer.assistantDaemon.revokeAcpClient({ clientId: safeClientId });
      const status = result?.status as AssistantDaemonStatus | undefined;
      if (status) {
        setAssistantDaemonStatus(status);
        replaceAssistantDaemonDraft(assistantDaemonStatusToDraft(status));
      } else {
        await loadAssistantDaemonStatus({ suppressAlert: true });
      }
    } catch (error) {
      console.error('Failed to revoke ACP client', error);
      void appAlert(`撤销 ACP 客户端失败：${String(error)}`);
    } finally {
      setAssistantDaemonBusy(false);
    }
  }, [loadAssistantDaemonStatus, replaceAssistantDaemonDraft]);

  const handleStartAssistantDaemon = useCallback(async () => {
    setAssistantDaemonBusy(true);
    try {
      const status = await window.ipcRenderer.assistantDaemon.start(buildAssistantDaemonPayload()) as AssistantDaemonStatus;
      setAssistantDaemonStatus(status);
      replaceAssistantDaemonDraft(assistantDaemonStatusToDraft(status));
    } catch (error) {
      console.error('Failed to start assistant daemon', error);
      void appAlert(`启动后台值守失败：${String(error)}`);
    } finally {
      setAssistantDaemonBusy(false);
    }
  }, [buildAssistantDaemonPayload, replaceAssistantDaemonDraft]);

  const handleStopAssistantDaemon = useCallback(async () => {
    setAssistantDaemonBusy(true);
    try {
      const status = await window.ipcRenderer.assistantDaemon.stop() as AssistantDaemonStatus;
      setAssistantDaemonStatus(status);
      replaceAssistantDaemonDraft(assistantDaemonStatusToDraft(status));
    } catch (error) {
      console.error('Failed to stop assistant daemon', error);
      void appAlert(`停止后台值守失败：${String(error)}`);
    } finally {
      setAssistantDaemonBusy(false);
    }
  }, [replaceAssistantDaemonDraft]);

  const handleStartAssistantDaemonWeixinLogin = useCallback(async () => {
    setAssistantDaemonWeixinLoginBusy(true);
    try {
      const result = await window.ipcRenderer.assistantDaemon.startWeixinLogin({
        accountId: String(assistantDaemonDraft.weixin.accountId || '').trim() || undefined,
        force: true,
      });
      const qrcodeImageUrl = await buildWeixinQrImageUrl(result.qrcodeUrl);
      if (!result.success || !result.qrcodeUrl) {
        setAssistantDaemonWeixinLogin({
          sessionKey: result.sessionKey,
          qrcodeUrl: result.qrcodeUrl,
          qrcodeImageUrl,
          message: result.message,
          connected: false,
          stateDir: result.stateDir,
        });
        void appAlert(result.message || '启动微信扫码失败。');
        return;
      }
      setAssistantDaemonWeixinLogin({
        sessionKey: result.sessionKey,
        qrcodeUrl: result.qrcodeUrl,
        qrcodeImageUrl,
        message: result.message,
        connected: false,
        stateDir: result.stateDir,
      });
    } catch (error) {
      console.error('Failed to start Weixin login', error);
      void appAlert(`启动微信扫码失败：${String(error)}`);
    } finally {
      setAssistantDaemonWeixinLoginBusy(false);
    }
  }, [assistantDaemonDraft.weixin.accountId, buildWeixinQrImageUrl]);

  const handleCheckAssistantDaemonWeixinLogin = useCallback(async () => {
    const sessionKey = String(assistantDaemonWeixinLogin?.sessionKey || '').trim();
    if (!sessionKey) {
      void appAlert('请先点击“开始扫码”，生成微信二维码。');
      return;
    }
    setAssistantDaemonWeixinLoginBusy(true);
    try {
      const result = await window.ipcRenderer.assistantDaemon.waitForWeixinLogin({
        sessionKey,
        timeoutMs: 1500,
      });
      setAssistantDaemonWeixinLogin((prev) => ({
        sessionKey,
        qrcodeUrl: prev?.qrcodeUrl,
        qrcodeImageUrl: prev?.qrcodeImageUrl,
        stateDir: prev?.stateDir,
        message: result.message,
        connected: result.connected,
        accountId: result.accountId,
        userId: result.userId,
      }));
      if (result.connected) {
        setAssistantDaemonDraft((prev) => ({
          ...prev,
          weixin: {
            ...prev.weixin,
            enabled: true,
            autoStartSidecar: true,
            accountId: result.accountId || prev.weixin.accountId,
          },
        }));
        await loadAssistantDaemonStatus();
      }
    } catch (error) {
      console.error('Failed to wait for Weixin login', error);
      void appAlert(`检查微信登录状态失败：${String(error)}`);
    } finally {
      setAssistantDaemonWeixinLoginBusy(false);
    }
  }, [assistantDaemonWeixinLogin?.sessionKey, loadAssistantDaemonStatus, setAssistantDaemonDraft]);

  const handleClearAssistantDaemonWeixinLogin = useCallback(() => {
    setAssistantDaemonWeixinLogin(null);
  }, []);

  const ensureBaseSettingsLoaded = useCallback(async (force = false) => {
    if (baseSettingsInFlightRef.current) return;
    if (!force && baseSettingsLoadedRef.current) return;
    baseSettingsInFlightRef.current = true;
    try {
      await Promise.all([
        loadSettings({
          preserveViewState: true,
          preserveRemoteModels: true,
        }),
        loadSpaceContext(),
      ]);
      baseSettingsLoadedRef.current = true;
      setBaseSettingsLoadedRevision((prev) => prev + 1);
      tabWarmRef.current.ai = true;
    } finally {
      baseSettingsInFlightRef.current = false;
    }
  }, [loadSettings, loadSpaceContext]);

  const loadSettingsSkills = useCallback(async () => {
    setIsSettingsSkillsLoading(true);
    try {
      const list = await window.ipcRenderer.listSkills();
      const normalized = (Array.isArray(list) ? list : [])
        .map((skill) => ({
          name: String(skill.name || '').trim(),
          description: String(skill.description || '').trim(),
          location: String(skill.location || '').trim(),
          sourceScope: skill.sourceScope,
          isBuiltin: Boolean(skill.isBuiltin || skill.sourceScope === 'builtin'),
          disabled: Boolean(skill.disabled),
        }))
        .filter((skill) => skill.name);
      normalized.sort((left, right) => {
        const leftBuiltIn = left.isBuiltin ? 0 : 1;
        const rightBuiltIn = right.isBuiltin ? 0 : 1;
        return leftBuiltIn - rightBuiltIn || left.name.localeCompare(right.name);
      });
      setSettingsSkills(normalized);
      return normalized;
    } catch (error) {
      console.error('Failed to load skills:', error);
      setSettingsSkillStatusMessage('技能列表读取失败');
      return [];
    } finally {
      setIsSettingsSkillsLoading(false);
    }
  }, []);

  const handleToggleSettingsSkill = useCallback(async (skill: SettingsSkill) => {
    if (skill.isBuiltin) return;
    const nextDisabled = !skill.disabled;
    setSettingsSkillBusyName(skill.name);
    setSettingsSkillStatusMessage('');
    setSettingsSkills((prev) => prev.map((item) => (
      item.name === skill.name ? { ...item, disabled: nextDisabled } : item
    )));
    try {
      const action = nextDisabled ? window.ipcRenderer.skills.disable : window.ipcRenderer.skills.enable;
      const result = await action({ name: skill.name }) as { success?: boolean; error?: string };
      if (result && result.success === false) {
        throw new Error(result.error || '技能状态保存失败');
      }
      setSettingsSkillStatusMessage(nextDisabled ? `已关闭 ${skill.name}` : `已打开 ${skill.name}`);
      await loadSettingsSkills();
      tabWarmRef.current.skills = true;
    } catch (error) {
      console.error('Failed to update skill state:', error);
      setSettingsSkills((prev) => prev.map((item) => (
        item.name === skill.name ? { ...item, disabled: skill.disabled } : item
      )));
      setSettingsSkillStatusMessage(error instanceof Error ? error.message : '技能状态保存失败');
    } finally {
      setSettingsSkillBusyName('');
    }
  }, [loadSettingsSkills]);

  const loadSkillMarketplace = useCallback(async () => {
    setIsSkillMarketplaceLoading(true);
    try {
      const result = await window.ipcRenderer.skills.marketplace();
      if (result.success === false) {
        throw new Error(result.error || '技能市场加载失败');
      }
      setSkillMarketplaceItems(Array.isArray(result.skills) ? result.skills : []);
      setSettingsSkillStatusMessage('');
    } catch (error) {
      console.error('Failed to load skill marketplace:', error);
      setSkillMarketplaceItems([]);
      setSettingsSkillStatusMessage(error instanceof Error ? error.message : '技能市场加载失败');
    } finally {
      setIsSkillMarketplaceLoading(false);
    }
  }, []);

  const openSkillMarketplace = useCallback(() => {
    setIsSkillMarketplaceOpen(true);
    void loadSkillMarketplace();
  }, [loadSkillMarketplace]);

  const handleUninstallSettingsSkill = useCallback(async (skill: SettingsSkill) => {
    if (skill.isBuiltin) return;
    const confirmed = await appConfirm(`删除技能“${skill.name}”？`, {
      title: '删除技能',
      confirmLabel: '删除',
      tone: 'danger',
    });
    if (!confirmed) return;
    setSettingsSkillBusyName(skill.name);
    setSettingsSkillStatusMessage('');
    try {
      const result = await window.ipcRenderer.skills.uninstall({ name: skill.name }) as { success?: boolean; error?: string };
      if (result && result.success === false) {
        throw new Error(result.error || '技能删除失败');
      }
      setSettingsSkillStatusMessage(`已删除 ${skill.name}`);
      await loadSettingsSkills();
      if (isSkillMarketplaceOpen) {
        await loadSkillMarketplace();
      }
      tabWarmRef.current.skills = true;
    } catch (error) {
      console.error('Failed to uninstall skill:', error);
      setSettingsSkillStatusMessage(error instanceof Error ? error.message : '技能删除失败');
    } finally {
      setSettingsSkillBusyName('');
    }
  }, [isSkillMarketplaceOpen, loadSettingsSkills, loadSkillMarketplace]);

  const handleInstallMarketplaceSkill = useCallback(async (skill: ThriveSkillMarketplaceItem) => {
    setSkillMarketplaceBusyId(skill.id);
    setSettingsSkillStatusMessage(`正在安装 ${skill.name}`);
    try {
      const result = await window.ipcRenderer.skills.marketInstall({
        id: skill.id,
        repo: skill.repo,
      }) as {
        success?: boolean;
        error?: string;
        installed?: Array<{ name?: string }>;
      };
      if (result.success === false) {
        throw new Error(result.error || '技能安装失败');
      }
      const installedName = result.installed?.[0]?.name || skill.name;
      setSettingsSkillStatusMessage(`已安装 ${installedName}`);
      await Promise.all([
        loadSettingsSkills(),
        loadSkillMarketplace(),
      ]);
      tabWarmRef.current.skills = true;
    } catch (error) {
      console.error('Failed to install marketplace skill:', error);
      setSettingsSkillStatusMessage(error instanceof Error ? error.message : '技能安装失败');
    } finally {
      setSkillMarketplaceBusyId('');
    }
  }, [loadSettingsSkills, loadSkillMarketplace]);

  const handleRollbackTeamMemberSkillVersion = useCallback(async (advisor: Advisor, version: string) => {
    if (!version) return;
    if (!(await appConfirm(`确定要把 ${advisor.name} 回滚到成员技能版本 "${version}" 吗？`, {
      title: '回滚成员技能',
      confirmLabel: '回滚',
      tone: 'danger',
    }))) return;
    try {
      await window.ipcRenderer.advisors.rollbackMemberSkillVersion({ advisorId: advisor.id, version });
      await refreshTeamAdvisor(advisor.id);
    } catch (error) {
      console.error('Failed to rollback member skill version:', error);
      setTestMsg('成员技能回滚失败');
      setStatus('error');
    }
  }, [refreshTeamAdvisor]);

  const ensureTabResourcesLoaded = useCallback(async (tab: SettingsTab, force = false) => {
    if (!isActive) return;
    if (tabInFlightRef.current[tab]) return;
    if (!force && tabWarmRef.current[tab]) return;
    tabInFlightRef.current[tab] = true;
    try {
      if (tab === 'general') {
        await Promise.all([
          loadAppVersion(),
          loadRecentDebugLogs(),
          loadFileIndexDashboard({ force }),
          loadLoggingStatus(),
          loadPendingDiagnosticReports(),
        ]);
      } else if (tab === 'profile') {
        await loadRedclawProfileBundle({
          preserveDraft: true,
        });
      } else if (tab === 'team') {
        await loadTeamAdvisors();
      } else if (tab === 'skills') {
        await loadSettingsSkills();
      } else if (tab === 'mcp') {
        await loadMcpRuntimeData();
      } else if (tab === 'tools') {
        await Promise.all([
          loadCliRuntimeDashboard({ silent: true }),
          loadThrivePlugins(),
        ]);
        if (formData.developer_mode_enabled) {
          await Promise.all([
            loadToolDiagnostics(),
            loadRuntimeRoles(),
          ]);
        }
      } else if (tab === 'ai' && officialAiPanelEnabled && !OfficialAiPanelComponent) {
        const module = await loadOfficialAiPanelModule();
        const nextComponent = module?.default || null;
        setOfficialAiPanelComponent(() => nextComponent);
      }
      tabWarmRef.current[tab] = true;
    } finally {
      tabInFlightRef.current[tab] = false;
    }
  }, [
    OfficialAiPanelComponent,
    formData.developer_mode_enabled,
    isActive,
    loadRedclawProfileBundle,
    loadCliRuntimeDashboard,
    loadAppVersion,
    loadBackgroundTasks,
    loadBackgroundWorkerPool,
    loadThrivePlugins,
    loadFileIndexDashboard,
    loadLoggingStatus,
    loadMcpRuntimeData,
    loadPendingDiagnosticReports,
    loadRecentDebugLogs,
    loadSettingsSkills,
    loadTeamAdvisors,
    loadRuntimeHooks,
    loadRuntimeRoles,
    loadRuntimeSessions,
    loadRuntimeTasks,
    loadToolDiagnostics,
    officialAiPanelEnabled,
  ]);

  useEffect(() => {
    if (settingsActivationTimerRef.current != null) {
      window.clearTimeout(settingsActivationTimerRef.current);
    }

    settingsActivationTimerRef.current = window.setTimeout(() => {
      void ensureBaseSettingsLoaded();
      settingsActivationTimerRef.current = null;
    }, SETTINGS_ACTIVATION_DEBOUNCE_MS);

    return () => {
      if (settingsActivationTimerRef.current != null) {
        window.clearTimeout(settingsActivationTimerRef.current);
        settingsActivationTimerRef.current = null;
      }
    };
  }, [ensureBaseSettingsLoaded]);

  useEffect(() => {
    if (!isActive) return;
    const handleSettingsUpdated = () => {
      // Preserve local edits on form-driven tabs; otherwise external auth sync can
      // reload persisted settings and wipe unsaved AI source/model changes.
      const preserveLocalFormState = activeTab === 'general' || activeTab === 'ai' || activeTab === 'platforms' || activeTab === 'experimental';
      if (!preserveLocalFormState) {
        void ensureBaseSettingsLoaded(true);
      }
      tabWarmRef.current.profile = false;
      if (activeTab === 'remote') {
        scheduleRemoteTabWarmup();
      }
      if (activeTab === 'profile' && !redclawProfileDirtyRef.current) {
        void ensureTabResourcesLoaded('profile', true);
      }
      if (activeTab === 'general' || activeTab === 'tools' || activeTab === 'skills' || activeTab === 'mcp') {
        tabWarmRef.current[activeTab] = false;
        void ensureTabResourcesLoaded(activeTab, true);
      }
    };
    return subscribeSettingsUpdated(handleSettingsUpdated);
  }, [activeTab, ensureBaseSettingsLoaded, ensureTabResourcesLoaded, isActive, scheduleRemoteTabWarmup]);

  useEffect(() => {
    if (!isActive) return;
    const handleSpaceChanged = (payload?: { spaceId?: string; activeSpaceId?: string; changeType?: string }) => {
      const previousSpaceId = currentSpaceIdRef.current;
      const nextSpaceId = setCurrentSpaceState(payload?.activeSpaceId || payload?.spaceId);
      void loadSpaceContext();
      if (payload?.changeType === 'rename' && nextSpaceId === previousSpaceId) {
        return;
      }
      tabWarmRef.current.profile = false;
      resetRedclawProfileState();
      clearCachedFileIndexDashboard();
      fileIndexDashboardCurrentRef.current = null;
      fileIndexDashboardLoadedAtRef.current = 0;
      setFileIndexDashboard(null);
      tabWarmRef.current.general = false;
      if (activeTab === 'profile') {
        void loadRedclawProfileBundle({ expectedSpaceId: nextSpaceId });
      }
      if (activeTab === 'general') {
        void loadFileIndexDashboard({ force: true });
      }
    };
    window.ipcRenderer.spaces.onChanged(handleSpaceChanged);
    return () => {
      window.ipcRenderer.spaces.offChanged(handleSpaceChanged);
    };
  }, [activeTab, isActive, loadFileIndexDashboard, loadRedclawProfileBundle, loadSpaceContext, resetRedclawProfileState, setCurrentSpaceState]);

  useEffect(() => {
    if (!redclawOnboardingVersion) return;
    void loadRedclawProfileBundle({ expectedSpaceId: currentSpaceIdRef.current });
  }, [loadRedclawProfileBundle, redclawOnboardingVersion]);

  useEffect(() => {
    if (!isActive || activeTab !== 'general') return;
    const scheduleFileIndexRefresh = () => {
      if (fileIndexDashboardRefreshTimerRef.current != null) {
        window.clearTimeout(fileIndexDashboardRefreshTimerRef.current);
      }
      fileIndexDashboardRefreshTimerRef.current = window.setTimeout(() => {
        fileIndexDashboardRefreshTimerRef.current = null;
        void loadFileIndexDashboard({ force: true, background: true });
      }, 750);
    };
    window.ipcRenderer.knowledge.onFileIndexUpdated(scheduleFileIndexRefresh);
    window.ipcRenderer.knowledge.onCatalogUpdated(scheduleFileIndexRefresh);
    return () => {
      window.ipcRenderer.knowledge.offFileIndexUpdated(scheduleFileIndexRefresh);
      window.ipcRenderer.knowledge.offCatalogUpdated(scheduleFileIndexRefresh);
      if (fileIndexDashboardRefreshTimerRef.current != null) {
        window.clearTimeout(fileIndexDashboardRefreshTimerRef.current);
        fileIndexDashboardRefreshTimerRef.current = null;
      }
    };
  }, [activeTab, isActive, loadFileIndexDashboard]);

  useEffect(() => {
    const handleDiagnosticsReportPending = () => {
      void Promise.all([
        loadLoggingStatus(),
        loadPendingDiagnosticReports(),
      ]);
    };
    const handleFeedbackReportSubmitted = () => {
      void Promise.all([
        loadLoggingStatus(),
        loadPendingDiagnosticReports(),
      ]);
    };
    window.ipcRenderer.logs.onReportPending(handleDiagnosticsReportPending);
    window.addEventListener('redbox:feedback-report-submitted', handleFeedbackReportSubmitted);
    return () => {
      window.ipcRenderer.logs.offReportPending(handleDiagnosticsReportPending);
      window.removeEventListener('redbox:feedback-report-submitted', handleFeedbackReportSubmitted);
    };
  }, [loadLoggingStatus, loadPendingDiagnosticReports]);

  useEffect(() => {
    if (!isActive) {
      return;
    }
    if (!baseSettingsLoadedRef.current) {
      return;
    }
    let runtimePollTimer: number | null = null;
    let backgroundTaskPollTimer: number | null = null;
    let fileIndexPollTimer: number | null = null;
    if (activeTab === 'remote') {
      scheduleRemoteTabWarmup();
    }
    if (activeTab === 'general') {
      void ensureTabResourcesLoaded('general');
      fileIndexPollTimer = window.setInterval(() => {
        void loadFileIndexDashboard({ force: true, background: true });
      }, FILE_INDEX_DASHBOARD_POLL_MS);
    }
    if (activeTab === 'profile') {
      void ensureTabResourcesLoaded('profile');
    }
    if (activeTab === 'team') {
      void ensureTabResourcesLoaded('team');
    }
    if (activeTab === 'skills') {
      void ensureTabResourcesLoaded('skills');
    }
    if (activeTab === 'mcp') {
      void ensureTabResourcesLoaded('mcp');
    }
    if (activeTab === 'tools') {
      void ensureTabResourcesLoaded('tools');
      if (
        formData.developer_mode_enabled
        && (runtimeTasks.length > 0 || runtimeSessions.length > 0)
      ) {
        runtimePollTimer = window.setInterval(() => {
          void Promise.all([
            loadRuntimeTasks(),
            loadRuntimeSessions(),
          ]);
        }, Math.max(8000, SETTINGS_TAB_POLL_DELAY_MS));
      }
      if (
        formData.developer_mode_enabled
        && backgroundTasks.length > 0
      ) {
        backgroundTaskPollTimer = window.setInterval(() => {
          void Promise.all([
            loadBackgroundTasks(),
            loadBackgroundWorkerPool(),
          ]);
        }, Math.max(5000, SETTINGS_TAB_POLL_DELAY_MS));
      }
    }

    return () => {
      if (runtimePollTimer) {
        window.clearInterval(runtimePollTimer);
      }
      if (backgroundTaskPollTimer) {
        window.clearInterval(backgroundTaskPollTimer);
      }
      if (fileIndexPollTimer) {
        window.clearInterval(fileIndexPollTimer);
      }
      if (remoteTabWarmTimerRef.current != null) {
        window.clearTimeout(remoteTabWarmTimerRef.current);
        remoteTabWarmTimerRef.current = null;
      }
    };
  }, [
    activeTab,
    backgroundTasks.length,
    baseSettingsLoadedRevision,
    ensureTabResourcesLoaded,
    formData.developer_mode_enabled,
    isActive,
    loadBackgroundTasks,
    loadBackgroundWorkerPool,
    loadFileIndexDashboard,
    loadRuntimeSessions,
    loadRuntimeTasks,
    runtimeSessions.length,
    runtimeTasks.length,
    scheduleRemoteTabWarmup,
  ]);

  const handlePickWorkspaceDir = useCallback(async () => {
    try {
      const result = await window.ipcRenderer.pickWorkspaceDir();
      if (!result?.success || !String(result.path || '').trim()) {
        if (!result?.canceled && result?.error) {
          void appAlert(`选择工作区目录失败：${String(result.error)}`);
        }
        return;
      }
      setFormData((prev) => ({
        ...prev,
        workspace_dir: String(result.path || '').trim(),
      }));
    } catch (error) {
      console.error('Failed to pick workspace dir', error);
      void appAlert(`选择工作区目录失败：${String(error)}`);
    }
  }, []);

  const handleResetWorkspaceDir = useCallback(() => {
    setFormData((prev) => ({
      ...prev,
      workspace_dir: '',
    }));
  }, []);

  const handleSave = async (e: React.FormEvent) => {
    e.preventDefault();
    if (aiSourceAutosaveTimerRef.current != null) {
      window.clearTimeout(aiSourceAutosaveTimerRef.current);
      aiSourceAutosaveTimerRef.current = null;
    }
    setStatus('saving');
    try {
      if (activeTab === 'profile') {
        const creatorProfileMarkdown = String(redclawProfileDraft.creatorProfile || '').trim();
        if (!creatorProfileMarkdown) {
          throw new Error('创作档案不能为空');
        }
        let savedDocCount = 0;
        await window.ipcRenderer.redclawProfile.updateDoc({
          docType: 'creator_profile',
          markdown: creatorProfileMarkdown,
          reason: 'settings-user-profile-save',
        });
        savedDocCount += 1;
        const nextDraft: RedclawProfileDraft = {
          user: redclawProfileDraft.user,
          creatorProfile: creatorProfileMarkdown,
        };
        setSavedRedclawProfileDraft(nextDraft);
        setRedclawProfileDraft(nextDraft);
        setRedclawProfileDirtyState(false);
        setRedclawProfileMessage({
          tone: 'success',
          text: savedDocCount === 2
            ? `用户档案已保存，${APP_BRAND.aiDisplayName} 后续会直接读取这两份长期档案。`
            : '用户档案已保存。',
        });
        tabWarmRef.current.profile = true;
        setStatus('saved');
        setTimeout(() => setStatus('idle'), 2000);
        return;
      }

      const {
        sanitizedSources,
        resolvedLegacyChatSourceId,
        legacyChatSource,
        resolvedApiEndpoint,
        resolvedApiKey,
        resolvedModelName,
      } = buildAiSourcePersistenceSnapshot();
      const aiSourceSaveGeneration = aiSourceEditGenerationRef.current;
      const aiModelSettingsSaveGeneration = aiModelSettingsEditGenerationRef.current;
      const fallbackRouteSource = officialAiSource || null;
      // 转写**不回退**到官方源：转写是唯一会上传音频的服务，
      // 官方源只做「用户没配转写」时的静默兜底 → 官方 token 一过期就 401，
      // 而提示只说「检查 transcription key」，用户完全看不出来。
      // 转写配置缺失时保留用户手填的值，由后端明确报「未配置音频转写服务」。
      const resolvedTranscriptionSource = getAiSourceById(transcriptionSourceId) || null;
      const resolvedEmbeddingSource = getAiSourceById(embeddingSourceId) || fallbackRouteSource;
      const resolvedVisualIndexSource = getAiSourceById(visualIndexSourceId) || fallbackRouteSource;
      const resolvedVideoAnalysisSource = getAiSourceById(videoAnalysisSourceId) || fallbackRouteSource;
      const resolvedImageSource = getAiSourceById(imageSourceId) || fallbackRouteSource;
      const resolvedVoiceSource = getAiSourceById(voiceSourceId) || fallbackRouteSource;
      const resolvedImageRouting = resolvedImageSource
        ? inferImageRoutingFromSource(resolvedImageSource)
        : {
          provider: formData.image_provider || 'openai-compatible',
          template: formData.image_provider_template || '',
        };
      const resolvedImageProvider = resolvedImageRouting.provider || formData.image_provider || 'openai-compatible';
      const resolvedImageProviderTemplate = inferImageTemplateByProvider(
        resolvedImageProvider,
        resolvedImageRouting.template || formData.image_provider_template || '',
      );
      const resolvedTranscriptionModel = String(formData.transcription_model || pickBestModelForSource(resolvedTranscriptionSource) || '').trim();
      const resolvedEmbeddingModel = String(formData.embedding_model || pickBestModelForSource(resolvedEmbeddingSource) || '').trim();
      const resolvedVisualIndexModel = String(formData.visual_index_model || pickBestVisualIndexModelForSource(resolvedVisualIndexSource) || '').trim();
      const resolvedVideoAnalysisModels = resolvedVideoAnalysisSource ? filterVideoAnalysisModels(getSourceModelList(resolvedVideoAnalysisSource)) : [];
      const requestedVideoAnalysisModel = String(formData.video_analysis_model || '').trim();
      // 用户手动填写的模型名优先采用（不再强制要求它落在当前源的"视频输入"模型列表里），
      // 这样即使视频分析源被误选为不含视频模型的源，手动填一个有效模型名也能通过保存校验。
      const resolvedVideoAnalysisModel = String(
        requestedVideoAnalysisModel
          ? requestedVideoAnalysisModel
          : pickBestVideoAnalysisModelForSource(resolvedVideoAnalysisSource),
      ).trim();
      const requestedImageModel = String(formData.image_model || aiModelRoutes.image.model || '').trim();
      const resolvedImageModel = String(
        resolvedImageProviderTemplate === 'dashscope-wan-native'
          ? DASHSCOPE_LOCKED_IMAGE_MODEL
          : pickCapabilityModelForSource(resolvedImageSource, requestedImageModel, 'image'),
      ).trim();
      const resolvedVoiceTtsModel = String(formData.voice_tts_model || aiModelRoutes.voiceTts.model || formData.tts_model || DEFAULT_VOICE_TTS_MODEL).trim();
      const resolvedVoiceCloneModel = cloneModelForVoiceTtsModel(
        resolvedVoiceTtsModel,
        String(formData.voice_clone_model || aiModelRoutes.voiceClone.model || DEFAULT_VOICE_CLONE_MODEL).trim(),
      );
      const resolvedVideoModel = REDBOX_OFFICIAL_VIDEO_MODEL_LIST.includes(String(formData.video_model || '').trim() as typeof REDBOX_OFFICIAL_VIDEO_MODEL_LIST[number])
        ? String(formData.video_model || '').trim()
        : REDBOX_OFFICIAL_VIDEO_MODELS['text-to-video'];
      const selectedImageModel = String(resolvedImageModel || '').trim();
      if (!selectedImageModel) {
        throw new Error('请填写生图模型（可手动输入或从列表选择）');
      }
      const parsedCompactTokens = Number(formData.redclaw_compact_target_tokens);
      const compactTargetTokens = Number.isFinite(parsedCompactTokens) && parsedCompactTokens > 0
        ? Math.max(16000, Math.floor(parsedCompactTokens))
        : 256000;
      const chatMaxTokensDefault = Number(sanitizeChatMaxTokensInput(
        formData.chat_max_tokens_default,
        DEFAULT_CHAT_MAX_TOKENS,
      ));
      const chatMaxTokensDeepseek = Number(sanitizeChatMaxTokensInput(
        formData.chat_max_tokens_deepseek,
        DEFAULT_CHAT_MAX_TOKENS_DEEPSEEK,
      ));
      const releaseLogRetentionDays = Math.max(1, Number(formData.release_log_retention_days || 7) || 7);
      const releaseLogMaxFileMb = Math.max(1, Number(formData.release_log_max_file_mb || 10) || 10);
      const normalizedVisualIndexProvider = 'openai-compatible';
      const parsedVisualIndexTimeoutSeconds = Number(formData.visual_index_timeout_seconds);
      const visualIndexTimeoutSeconds = Number.isFinite(parsedVisualIndexTimeoutSeconds)
        ? Math.min(300, Math.max(10, Math.floor(parsedVisualIndexTimeoutSeconds)))
        : 90;
      const parsedVisualIndexMaxImageEdge = Number(formData.visual_index_max_image_edge);
      const visualIndexMaxImageEdge = Number.isFinite(parsedVisualIndexMaxImageEdge)
        ? Math.min(4096, Math.max(512, Math.floor(parsedVisualIndexMaxImageEdge)))
        : 1536;
      const parsedVisualIndexPdfMaxPages = Number(formData.visual_index_pdf_max_pages);
      const visualIndexPdfMaxPages = Number.isFinite(parsedVisualIndexPdfMaxPages)
        ? Math.min(200, Math.max(1, Math.floor(parsedVisualIndexPdfMaxPages)))
        : 12;
      const parsedVisualIndexPdfRenderDpi = Number(formData.visual_index_pdf_render_dpi);
      const visualIndexPdfRenderDpi = Number.isFinite(parsedVisualIndexPdfRenderDpi)
        ? Math.min(300, Math.max(72, Math.floor(parsedVisualIndexPdfRenderDpi)))
        : 144;
      const parsedVisualIndexConcurrency = Number(formData.visual_index_concurrency);
      const visualIndexConcurrency = Number.isFinite(parsedVisualIndexConcurrency)
        ? Math.min(4, Math.max(1, Math.floor(parsedVisualIndexConcurrency)))
        : 1;
      const normalizedVisualIndexEndpoint = String(resolvedVisualIndexSource?.baseURL || formData.visual_index_endpoint || resolvedApiEndpoint).trim();
      const normalizedVisualIndexApiKey = String(resolvedVisualIndexSource?.apiKey || formData.visual_index_api_key || '').trim();
      const normalizedVisualIndexModel = resolvedVisualIndexModel;
      if (!normalizedVisualIndexEndpoint || !normalizedVisualIndexModel) {
        throw new Error('启用知识库视觉索引时必须填写多模态 Endpoint 和模型名');
      }
      const normalizedVideoAnalysisEndpoint = String(resolvedVideoAnalysisSource?.baseURL || formData.video_analysis_endpoint || resolvedApiEndpoint).trim();
      const normalizedVideoAnalysisApiKey = String(resolvedVideoAnalysisSource?.apiKey || formData.video_analysis_api_key || '').trim();
      const normalizedVideoAnalysisProtocol = String(resolvedVideoAnalysisSource?.protocol || formData.video_analysis_protocol || 'gemini').trim();
      if (!normalizedVideoAnalysisEndpoint || !resolvedVideoAnalysisModel) {
        throw new Error('启用视频分析专用模型时必须填写 Endpoint 和模型名');
      }
      const routeScopedSource = (scope: AiModelRouteScope) => getRouteSource(aiModelRoutes[scope]) || fallbackRouteSource;
      const routeModel = (
        scope: AiModelRouteScope,
        value: string,
        fallbackSource: AiSourceConfig | null,
        fallbackPicker: (source: AiSourceConfig | null, preferred?: string) => string,
      ) => {
        const mode = aiModelRoutes[scope].mode;
        if (mode === 'disabled') return '';
        const explicitModel = String(value || aiModelRoutes[scope].model || '').trim();
        if (explicitModel) {
          if (mode === 'official') {
            return String(fallbackPicker(fallbackSource, explicitModel) || '').trim();
          }
          return explicitModel;
        }
        return String(fallbackPicker(fallbackSource) || '').trim();
      };
      const routeChatModel = (scope: AiModelRouteScope, value: string) => routeModel(
        scope,
        value,
        routeScopedSource(scope),
        (source) => pickBestModelForSource(source, '', 'chat'),
      );
      const normalizeRouteForSource = (
        route: AiModelRouteConfig,
        sourceId: string,
      ): AiModelRouteConfig => {
        const normalizedSourceId = canonicalizeOfficialAutoSourceId(sourceId);
        if (route.mode === 'disabled') {
          return { ...route, sourceId: normalizedSourceId };
        }
        if (isOfficialAutoSourceId(normalizedSourceId) || route.mode === 'official') {
          return { ...route, mode: 'official', sourceId: OFFICIAL_AUTO_SOURCE_ID };
        }
        if (normalizedSourceId) {
          return { ...route, mode: 'custom', sourceId: normalizedSourceId };
        }
        return { ...route, sourceId: normalizedSourceId };
      };
      // 转写路由的 mode：**不沿用旧的 official**。
      //
      // 转写的 route.mode 会被自动回退污染（用户没为该 scope 选源时，
      // `routeSourceMode` 会给出 official），而 `normalizeRouteForSource` 见到
      // mode==='official' 就会强制写回 official —— 于是后端又按官方源解析，
      // 官方 token 一过期就是 401，而用户配的是 ElevenLabs。
      //
      // 所以转写这一路：禁用的保持 disabled，其余按「实际选中的源」推导 ——
      // 没选源就是 custom + 空 sourceId（后端会明确报「未配置音频转写服务」）。
      const transcriptionRouteMode: AiModelRouteConfig['mode'] =
        aiModelRoutes.transcription.mode === 'disabled'
          ? 'disabled'
          : (resolvedTranscriptionSource?.id
              ? (isOfficialAutoSourceId(resolvedTranscriptionSource.id) ? 'official' : 'custom')
              : 'custom');
      // 转写模型名：**用户填的优先**，不走 routeModel 的 official 兜底。
      //
      // routeModel 在 mode==='official' 时会用 `pickBestModelForSource(source, explicit)`
      // 覆盖显式模型名 —— 而转写的路由 mode 会被自动回退成 official（用户没为
      // 该 scope 选源时），于是用户手填的模型名被换成官方源的模型
      // （实测：ElevenLabs 的 scribe_v2 被换成 fun-asr → 400 unsupported_model）。
      //
      // 用户为转写选了源时，formData.transcription_model 已在选源处同步成源的值，
      // 所以 formData 优先的语义与「源优先」一致。
      const routeTranscriptionModel = String(formData.transcription_model || '').trim()
        || routeModel(
          'transcription',
          resolvedTranscriptionModel,
          resolvedTranscriptionSource,
          (source) => pickBestModelForSource(source, '', 'transcription'),
        );
      const routeEmbeddingModel = routeModel(
        'embedding',
        resolvedEmbeddingModel,
        resolvedEmbeddingSource,
        (source) => pickBestModelForSource(source, '', 'embedding'),
      );
      const routeImageModel = resolvedImageModel;
      const routeVisualIndexModel = resolvedVisualIndexModel;
      const routeVideoAnalysisModel = resolvedVideoAnalysisModel;
      const routeVoiceTtsModel = routeModel(
        'voiceTts',
        resolvedVoiceTtsModel,
        resolvedVoiceSource,
        (source) => pickBestModelForSource(source, '', 'tts') || pickBestModelForSource(source, '', 'audio') || DEFAULT_VOICE_TTS_MODEL,
      ) || DEFAULT_VOICE_TTS_MODEL;
      const routeVoiceCloneModel = resolvedVoiceCloneModel;
      const resolvedChatSource = routeScopedSource('chat') || legacyChatSource || null;
      const resolvedChatSourceId = canonicalizeOfficialAutoSourceId(
        resolvedChatSource?.id || (aiModelRoutes.chat.mode === 'official' ? OFFICIAL_AUTO_SOURCE_ID : resolvedLegacyChatSourceId)
      );
      const routeChatModelValue = routeModel(
        'chat',
        formData.model_name || resolvedModelName,
        resolvedChatSource,
        (source) => pickBestModelForSource(source, '', 'chat'),
      );
      const normalizedModelRoutes: AiModelRoutes = {
        ...aiModelRoutes,
        chat: normalizeRouteForSource({
          ...aiModelRoutes.chat,
          sourceId: resolvedChatSourceId,
          model: routeChatModelValue,
        }, resolvedChatSourceId),
        wander: normalizeRouteForSource(
          { ...aiModelRoutes.wander, model: routeChatModel('wander', formData.model_name_wander) },
          aiModelRoutes.wander.sourceId || OFFICIAL_AUTO_SOURCE_ID,
        ),
        team: normalizeRouteForSource(
          { ...aiModelRoutes.team, model: routeChatModel('team', formData.model_name_chatroom) },
          aiModelRoutes.team.sourceId || OFFICIAL_AUTO_SOURCE_ID,
        ),
        knowledge: normalizeRouteForSource(
          { ...aiModelRoutes.knowledge, model: routeChatModel('knowledge', formData.model_name_knowledge) },
          aiModelRoutes.knowledge.sourceId || OFFICIAL_AUTO_SOURCE_ID,
        ),
        redclaw: normalizeRouteForSource(
          { ...aiModelRoutes.redclaw, model: routeChatModel('redclaw', formData.model_name_redclaw) },
          aiModelRoutes.redclaw.sourceId || OFFICIAL_AUTO_SOURCE_ID,
        ),
        transcription: normalizeRouteForSource(
          { ...aiModelRoutes.transcription, mode: transcriptionRouteMode, sourceId: resolvedTranscriptionSource?.id || '', model: routeTranscriptionModel },
          resolvedTranscriptionSource?.id || '',
        ),
        embedding: normalizeRouteForSource(
          { ...aiModelRoutes.embedding, sourceId: resolvedEmbeddingSource?.id || '', model: routeEmbeddingModel },
          resolvedEmbeddingSource?.id || '',
        ),
        image: normalizeRouteForSource(
          { ...aiModelRoutes.image, sourceId: resolvedImageSource?.id || '', model: routeImageModel },
          resolvedImageSource?.id || '',
        ),
        visualIndex: normalizeRouteForSource({
          ...aiModelRoutes.visualIndex,
          mode: aiModelRoutes.visualIndex.mode === 'disabled' ? 'official' : aiModelRoutes.visualIndex.mode,
          sourceId: resolvedVisualIndexSource?.id || '',
          model: routeVisualIndexModel,
        }, resolvedVisualIndexSource?.id || ''),
        videoAnalysis: normalizeRouteForSource({
          ...aiModelRoutes.videoAnalysis,
          mode: aiModelRoutes.videoAnalysis.mode === 'disabled' ? 'official' : aiModelRoutes.videoAnalysis.mode,
          sourceId: resolvedVideoAnalysisSource?.id || '',
          model: routeVideoAnalysisModel,
        }, resolvedVideoAnalysisSource?.id || ''),
        voiceTts: normalizeRouteForSource(
          { ...aiModelRoutes.voiceTts, sourceId: resolvedVoiceSource?.id || '', model: routeVoiceTtsModel },
          resolvedVoiceSource?.id || '',
        ),
        voiceClone: { mode: 'official', sourceId: OFFICIAL_AUTO_SOURCE_ID, model: routeVoiceCloneModel },
      };
      const parsedParserTimeoutSeconds = Number(formData.parser_timeout_seconds);
      const parserTimeoutSeconds = Number.isFinite(parsedParserTimeoutSeconds)
        ? Math.min(300, Math.max(10, Math.floor(parsedParserTimeoutSeconds)))
        : 90;
      const parsedRerankTimeoutSeconds = Number(formData.rerank_timeout_seconds);
      const rerankTimeoutSeconds = Number.isFinite(parsedRerankTimeoutSeconds)
        ? Math.min(120, Math.max(5, Math.floor(parsedRerankTimeoutSeconds)))
        : 30;
      if (formData.proxy_enabled && !String(formData.proxy_url || '').trim()) {
        throw new Error('启用代理时必须填写代理地址，例如 http://127.0.0.1:7890');
      }

      await window.ipcRenderer.saveSettings({
        ...formData,
        api_endpoint: String(resolvedChatSource?.baseURL || resolvedApiEndpoint).trim(),
        api_key: String(resolvedChatSource?.apiKey || resolvedApiKey).trim(),
        model_name: routeChatModelValue || resolvedModelName,
        model_name_wander: routeChatModel('wander', formData.model_name_wander),
        model_name_chatroom: routeChatModel('team', formData.model_name_chatroom),
        model_name_knowledge: routeChatModel('knowledge', formData.model_name_knowledge),
        model_name_redclaw: routeChatModel('redclaw', formData.model_name_redclaw),
        proxy_enabled: Boolean(formData.proxy_enabled),
        proxy_url: String(formData.proxy_url || '').trim(),
        proxy_bypass: String(formData.proxy_bypass || '').trim(),
        transcription_model: routeTranscriptionModel,
        // 转写的 endpoint/key 以 **formData 为准**（它已是最终生效值）。
        //
        // 之前写的是 `resolvedTranscriptionSource?.baseURL || formData.transcription_endpoint || …`，
        // 而 `resolvedTranscriptionSource` 在用户**没为转写选源**时会回退到默认源
        // （实测是官方源）→ 用户在「语音转写」里手填的地址/key 被整个覆盖成官方源的。
        // 结果请求打到官方网关（api.ziz.hk/beav/v1）、用官方 access token ——
        // 官方 token 一过期就 401 "Invalid access token"，而用户填的其实是 ElevenLabs。
        //
        // 用户为该 scope 选了源时，formData 会在选源处同步成源的值（见上面的
        // transcription_endpoint/transcription_key 赋值），所以这里用 formData 语义一致。
        transcription_endpoint: String(
          formData.transcription_endpoint || resolvedTranscriptionSource?.baseURL || resolvedApiEndpoint,
        ).trim(),
        transcription_key: String(
          formData.transcription_key || resolvedTranscriptionSource?.apiKey || '',
        ).trim(),
        embedding_model: routeEmbeddingModel,
        embedding_endpoint: String(resolvedEmbeddingSource?.baseURL || formData.embedding_endpoint || resolvedApiEndpoint).trim(),
        embedding_key: String(resolvedEmbeddingSource?.apiKey || formData.embedding_key || '').trim(),
        visual_index_enabled: Boolean(formData.visual_index_enabled),
        visual_index_provider: normalizedVisualIndexProvider,
        visual_index_endpoint: normalizedVisualIndexEndpoint,
        visual_index_api_key: normalizedVisualIndexApiKey,
        visual_index_model: routeVisualIndexModel,
        visual_index_prompt_version: normalizeVisualIndexPromptVersion(formData.visual_index_prompt_version),
        visual_index_timeout_seconds: visualIndexTimeoutSeconds,
        visual_index_max_image_edge: visualIndexMaxImageEdge,
        visual_index_skip_small_images: Boolean(formData.visual_index_skip_small_images),
        visual_index_pdf_max_pages: visualIndexPdfMaxPages,
        visual_index_pdf_render_dpi: visualIndexPdfRenderDpi,
        visual_index_concurrency: visualIndexConcurrency,
        video_analysis_enabled: true,
        video_analysis_endpoint: normalizedVideoAnalysisEndpoint,
        video_analysis_api_key: normalizedVideoAnalysisApiKey,
        video_analysis_model: routeVideoAnalysisModel,
        video_analysis_protocol: normalizedVideoAnalysisProtocol,
        video_analysis_max_direct_video_bytes: Number(formData.video_analysis_max_direct_video_bytes || 64 * 1024 * 1024),
        docling_endpoint: String(formData.docling_endpoint || '').trim(),
        tika_endpoint: String(formData.tika_endpoint || '').trim(),
        unstructured_endpoint: String(formData.unstructured_endpoint || '').trim(),
        parser_api_key: String(formData.parser_api_key || '').trim(),
        parser_timeout_seconds: parserTimeoutSeconds,
        rerank_endpoint: String(formData.rerank_endpoint || '').trim(),
        rerank_api_key: String(formData.rerank_api_key || '').trim(),
        rerank_model: String(formData.rerank_model || '').trim(),
        rerank_timeout_seconds: rerankTimeoutSeconds,
        image_provider: resolvedImageProvider,
        image_provider_template: resolvedImageProviderTemplate,
        image_endpoint: String(resolvedImageSource?.baseURL || formData.image_endpoint || '').trim(),
        image_api_key: String(resolvedImageSource?.apiKey || formData.image_api_key || '').trim(),
        image_model: routeImageModel,
        voice_provider: 'voice',
        voice_endpoint: String(resolvedVoiceSource?.baseURL || formData.voice_endpoint || formData.api_endpoint || '').trim(),
        voice_api_key: String(resolvedVoiceSource?.apiKey || formData.voice_api_key || formData.api_key || '').trim(),
        voice_tts_model: routeVoiceTtsModel,
        tts_model: routeVoiceTtsModel,
        voice_clone_model: routeVoiceCloneModel,
        video_endpoint: REDBOX_OFFICIAL_VIDEO_BASE_URL,
        video_api_key: String(formData.video_api_key || formData.api_key || '').trim(),
        video_model: resolvedVideoModel,
        ai_model_routes_json: JSON.stringify(normalizedModelRoutes),
        ai_sources_json: JSON.stringify(sanitizedSources),
        default_ai_source_id: normalizedModelRoutes.chat.sourceId || resolvedLegacyChatSourceId || '',
        mcp_servers_json: JSON.stringify(mcpServers),
        redclaw_compact_target_tokens: compactTargetTokens,
        debug_log_enabled: Boolean(formData.debug_log_enabled),
        diagnostics_upload_consent: formData.diagnostics_upload_consent,
        diagnostics_include_advanced_context: Boolean(formData.diagnostics_include_advanced_context),
        diagnostics_auto_send_same_crash: Boolean(formData.diagnostics_auto_send_same_crash),
        diagnostics_last_prompted_at: formData.diagnostics_last_prompted_at || null,
        analytics_consent: formData.analytics_consent,
        analytics_last_prompted_at: new Date().toISOString(),
        release_log_retention_days: releaseLogRetentionDays,
        release_log_max_file_mb: releaseLogMaxFileMb,
        notifications_json: JSON.stringify(notificationSettings),
        cli_runtime_execution_mode: normalizeCliRuntimeExecutionMode(formData.cli_runtime_execution_mode),
        developer_mode_enabled: Boolean(formData.developer_mode_enabled),
        developer_mode_unlocked_at: formData.developer_mode_enabled
          ? (formData.developer_mode_unlocked_at || new Date().toISOString())
          : null,
        ecommerce_platforms_json: serializeEcommercePlatformsSettings(
          normalizeEcommercePlatformsSettings(formData.ecommerce_platforms_json)
        ),
        chat_max_tokens_default: chatMaxTokensDefault,
        chat_max_tokens_deepseek: chatMaxTokensDeepseek,
      });
      void window.ipcRenderer.analytics.track('settings_changed', {
        surface: 'settings',
        origin: 'renderer',
        properties: {
          settingKey: 'analytics_consent',
          valueKind: formData.analytics_consent,
        },
      });
      clearAiSourceDraftDirty(aiSourceSaveGeneration);
      clearAiModelSettingsDraftDirty(aiModelSettingsSaveGeneration);
      if (formData.debug_log_enabled) {
        await loadRecentDebugLogs();
      }
      await Promise.all([
        loadLoggingStatus(),
        loadPendingDiagnosticReports(),
      ]);
      setStatus('saved');
      setTimeout(() => setStatus('idle'), 2000);
    } catch (e) {
      console.error(e);
      if (activeTab === 'profile') {
        setRedclawProfileMessage({
          tone: 'error',
          text: e instanceof Error ? e.message : String(e),
        });
      }
      if (e instanceof Error && e.message) {
        setTestStatus('error');
        setTestMsg(e.message);
      }
      setStatus('error');
    }
  };

  const handleTestNotificationSound = useCallback(async () => {
    try {
      await playTestNotificationSound('attention', notificationSettings.sound.volume);
    } catch (error) {
      console.warn('Failed to play notification test sound:', error);
    }
  }, [notificationSettings.sound.volume]);

  const loadAiPricingCatalog = useCallback(async (options?: { refreshRemote?: boolean }) => {
    setAiPricingLoading(true);
    setAiPricingError('');
    try {
      const result = options?.refreshRemote
        ? await window.ipcRenderer.officialAuth.refreshPricing()
        : await window.ipcRenderer.officialAuth.getPricing();
      const catalog = parseAiPricingCatalog(result?.pricing);
      setAiPricingCatalog(catalog);
      setAiPricingActiveGroup((prev) => {
        if (prev && catalog?.groups.some((group) => group.type === prev)) return prev;
        return catalog?.groups[0]?.type || '';
      });
      if (!catalog) {
        setAiPricingError('价格表尚未同步，请重启应用后再查看。');
      }
    } catch (error) {
      setAiPricingError(error instanceof Error ? error.message : '价格表读取失败');
    } finally {
      setAiPricingLoading(false);
    }
  }, []);

  const handleOpenAiPricing = useCallback(() => {
    setSettingsSubView('ai-pricing');
    setActiveTab('ai');
    void loadAiPricingCatalog();
  }, [loadAiPricingCatalog]);

  const handleCloseAiPricing = useCallback(() => {
    setSettingsSubView('main');
  }, []);

  useEffect(() => {
    if (settingsSubView !== 'ai-pricing') return;
    const handleSettingsUpdated = () => {
      void loadAiPricingCatalog();
    };
    return subscribeSettingsUpdated(handleSettingsUpdated);
  }, [loadAiPricingCatalog, settingsSubView]);

  const activePricingGroup = useMemo(() => (
    aiPricingCatalog?.groups.find((group) => group.type === aiPricingActiveGroup)
    || aiPricingCatalog?.groups[0]
    || null
  ), [aiPricingActiveGroup, aiPricingCatalog]);

  const filteredPricingModels = useMemo(() => {
    const query = aiPricingSearch.trim().toLowerCase();
    const models = activePricingGroup?.models || [];
    if (!query) return models;
    return models.filter((model) => [
      model.model,
      model.display_name,
      model.provider,
      model.capability,
    ].some((value) => String(value || '').toLowerCase().includes(query)));
  }, [activePricingGroup, aiPricingSearch]);

  const renderPricingRateTable = (model: AiPricingModel) => {
    const rows = Array.isArray(model.price_table) && model.price_table.length
      ? model.price_table
      : Array.isArray(model.image_quality_resolution_rates) && model.image_quality_resolution_rates.length
        ? model.image_quality_resolution_rates
        : Array.isArray(model.video_resolution_rates) && model.video_resolution_rates.length
          ? model.video_resolution_rates
          : [];
    if (!rows.length) return null;
    const keys = Array.from(new Set(rows.flatMap((row) => Object.keys(row)))).filter((key) => (
      rows.some((row) => hasMeaningfulPricingValue(row[key]))
    ));
    if (!keys.length) return null;
    return (
      <div className="mt-3 overflow-hidden rounded-lg border border-border/70">
        <table className="w-full min-w-[560px] text-xs">
          <thead className="bg-surface-secondary/50 text-text-tertiary">
            <tr>
              {keys.map((key) => (
                <th key={key} className="px-3 py-2 text-left font-medium">{pricingRateLabel(key)}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row, index) => (
              <tr key={index} className="border-t border-border/50">
                {keys.map((key) => (
                  <td key={key} className="px-3 py-2 text-text-secondary">{pricingRateCellValue(key, row[key])}</td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    );
  };

  const renderAiPricingPage = () => (
    <div className="flex h-full min-w-0 flex-1 flex-col bg-surface-primary text-text-primary">
      <div className="flex items-center justify-between gap-4 border-b border-border px-6 py-4">
        <div className="flex min-w-0 items-center gap-3">
          <button
            type="button"
            onClick={handleCloseAiPricing}
            className="inline-flex h-8 w-8 items-center justify-center rounded-lg text-text-secondary transition-colors hover:bg-surface-secondary hover:text-text-primary"
            title="返回 AI 设置"
          >
            <ArrowLeft className="h-4 w-4" />
          </button>
          <div className="min-w-0">
            <h2 className="text-base font-semibold text-text-primary">AI 价格表</h2>
            <p className="mt-0.5 text-xs text-text-tertiary">
              更新时间：{formatPricingUpdatedAt(aiPricingCatalog?.updated_at)}
            </p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => void loadAiPricingCatalog({ refreshRemote: true })}
            disabled={aiPricingLoading}
            className="inline-flex h-9 w-9 items-center justify-center rounded-lg border border-border bg-surface-secondary/30 text-text-secondary transition-colors hover:bg-surface-secondary hover:text-text-primary disabled:cursor-not-allowed disabled:opacity-60"
            title="刷新价格表"
            aria-label="刷新价格表"
          >
            <RefreshCw className={clsx('h-4 w-4', aiPricingLoading && 'animate-spin')} />
          </button>
          <input
            type="search"
            value={aiPricingSearch}
            onChange={(event) => setAiPricingSearch(event.target.value)}
            placeholder="搜索模型或供应商"
            className="w-64 rounded-lg border border-border bg-surface-secondary/30 px-3 py-2 text-sm outline-none transition-colors focus:border-accent-primary"
          />
        </div>
      </div>

      <div className="flex min-h-0 flex-1">
        <div className="w-56 shrink-0 border-r border-border bg-surface-secondary/20 p-4">
          <div className="space-y-1">
            {(aiPricingCatalog?.groups || []).map((group) => (
              <button
                key={group.type}
                type="button"
                onClick={() => setAiPricingActiveGroup(group.type)}
                className={clsx(
                  'flex w-full items-center justify-between gap-2 rounded-lg px-3 py-2 text-left text-sm transition-colors',
                  activePricingGroup?.type === group.type
                    ? 'bg-surface-secondary text-text-primary'
                    : 'text-text-secondary hover:bg-surface-secondary/60 hover:text-text-primary',
                )}
              >
                <span>{group.label}</span>
                <span className="text-xs text-text-tertiary">{group.models.length}</span>
              </button>
            ))}
          </div>
        </div>

        <div className="min-w-0 flex-1 overflow-auto p-6">
          {aiPricingLoading && !aiPricingCatalog ? (
            <div className="rounded-lg border border-border bg-surface-secondary/20 p-4 text-sm text-text-tertiary">正在读取本地价格表...</div>
          ) : aiPricingError && !aiPricingCatalog ? (
            <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-4 text-sm text-amber-600">{aiPricingError}</div>
          ) : activePricingGroup ? (
            <div className="space-y-4">
              <div className="flex items-center justify-between gap-3">
                <div>
                  <h3 className="text-sm font-medium text-text-primary">{activePricingGroup.label}</h3>
                  <p className="mt-1 text-xs text-text-tertiary">{filteredPricingModels.length} / {activePricingGroup.models.length} 个模型</p>
                </div>
              </div>

              {filteredPricingModels.map((model) => (
                <div key={`${activePricingGroup.type}:${model.model}`} className="rounded-xl border border-border bg-surface-primary p-4">
                  <div className="flex flex-wrap items-start justify-between gap-3">
                    <div className="min-w-0">
                      <div className="flex flex-wrap items-center gap-2">
                        <h4 className="text-sm font-semibold text-text-primary">{model.display_name || model.model}</h4>
                        {model.is_default ? (
                          <span className="rounded bg-accent-primary/10 px-1.5 py-0.5 text-[10px] font-medium text-accent-primary">默认</span>
                        ) : null}
                      </div>
                      <p className="mt-1 text-xs text-text-tertiary">{model.provider || '-'} · {pricingModeLabel(model.pricing_mode)}</p>
                    </div>
                    {pricingModelFields(model, activePricingGroup.type).length ? (
                      <div className="flex max-w-full flex-wrap justify-end gap-2 text-right text-xs">
                        {pricingModelFields(model, activePricingGroup.type).map((field) => (
                          <div key={field.label} className="rounded-lg border border-border/70 bg-surface-secondary/20 px-2.5 py-1.5">
                            <div className="text-text-tertiary">{field.label}</div>
                            <div className="mt-0.5 font-medium text-text-primary">{field.value}</div>
                          </div>
                        ))}
                      </div>
                    ) : null}
                  </div>
                  {renderPricingRateTable(model)}
                </div>
              ))}
            </div>
          ) : (
            <div className="rounded-lg border border-border bg-surface-secondary/20 p-4 text-sm text-text-tertiary">暂无价格表数据。</div>
          )}
        </div>
      </div>
    </div>
  );

  const renderSettingsSkillRow = useCallback((skill: SettingsSkill) => {
    const isBusy = settingsSkillBusyName === skill.name;
    const enabled = skill.isBuiltin || !skill.disabled;
    return (
      <div key={skill.location || skill.name} className="flex items-center justify-between gap-4 px-4 py-3">
        <div className="min-w-0">
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate text-sm font-medium text-text-primary">{skill.name}</span>
            <span className={clsx(
              'shrink-0 rounded-full px-2 py-0.5 text-[10px] font-medium',
              skill.isBuiltin
                ? 'bg-accent-primary/10 text-accent-primary'
                : 'bg-surface-secondary text-text-tertiary'
            )}>
              {formatSettingsSkillSource(skill.sourceScope)}
            </span>
          </div>
          {skill.description && (
            <div className="mt-1 line-clamp-2 text-xs leading-5 text-text-tertiary">
              {skill.description}
            </div>
          )}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <button
            type="button"
            onClick={() => void handleToggleSettingsSkill(skill)}
            disabled={skill.isBuiltin || isBusy}
            role="switch"
            aria-checked={enabled}
            aria-label={`${enabled ? '关闭' : '打开'}技能 ${skill.name}`}
            title={skill.isBuiltin ? '内置技能不可关闭' : (enabled ? '关闭技能' : '打开技能')}
            className={clsx(
              'ui-switch-track disabled:cursor-not-allowed',
              skill.isBuiltin && 'opacity-70',
              isBusy && 'opacity-60'
            )}
            data-size="sm"
            data-state={enabled ? 'on' : 'off'}
          >
            <span className="ui-switch-thumb" />
          </button>
          {!skill.isBuiltin && (
            <button
              type="button"
              onClick={() => void handleUninstallSettingsSkill(skill)}
              disabled={isBusy}
              className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-border text-text-tertiary transition-colors hover:border-brand-red/30 hover:bg-brand-red/10 hover:text-brand-red disabled:opacity-50"
              aria-label={`删除技能 ${skill.name}`}
              title="删除技能"
            >
              <Trash2 className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
      </div>
    );
  }, [handleToggleSettingsSkill, handleUninstallSettingsSkill, settingsSkillBusyName]);

  const allTabs = useMemo<Array<{ id: SettingsTab; labelKey: I18nKey; icon: ComponentType<{ className?: string }> }>>(() => [
    { id: 'ai', labelKey: 'settings.tabs.ai', icon: Cpu },
    { id: 'general', labelKey: 'settings.tabs.general', icon: LayoutGrid },
    { id: 'team', labelKey: 'settings.tabs.team', icon: Users },
    { id: 'platforms', labelKey: 'settings.tabs.platforms', icon: Store },
    { id: 'skills', labelKey: 'settings.tabs.skills', icon: Star },
    { id: 'mcp', labelKey: 'settings.tabs.mcp', icon: Server },
    { id: 'remote', labelKey: 'settings.tabs.remote', icon: MessageSquareText },
    { id: 'profile', labelKey: 'settings.tabs.profile', icon: FileText },
    { id: 'tools', labelKey: 'settings.tabs.tools', icon: Wrench },
    { id: 'experimental', labelKey: 'settings.tabs.experimental', icon: FlaskConical },
  ], []);

  const tabs = useMemo(() => {
    const visibleTabs = new Set(APP_BRAND.visibleSettingsTabs);
    let filteredTabs = visibleTabs.size === 0
      ? allTabs
      : allTabs.filter((tab) => visibleTabs.has(tab.id));
    // 「AI 模型」只对管理员开放：它是服务器级路由配置，普通用户改不了也用不了
    // （官方源未登录时整页只会显示「请先登录官方账号」）。
    if (!isAdminUser) {
      filteredTabs = filteredTabs.filter((tab) => tab.id !== 'ai');
    }
    if (filteredTabs.length === 0) {
      filteredTabs = allTabs.filter((tab) => isAdminUser || tab.id !== 'ai');
    }
    return filteredTabs.length > 0 ? filteredTabs : allTabs;
  }, [allTabs, isAdminUser]);

  useEffect(() => {
    if (tabs.some((tab) => tab.id === activeTab)) return;
    setActiveTab(tabs[0]?.id || 'general');
  }, [activeTab, tabs]);

  return {
    settingsSubView,
    renderAiPricingPage,
    onReturn,
    t,
    tabs,
    setActiveTab,
    activeTab,
    handleSave,
    appVersion,
    formData,
    setFormData,
    notificationSettings,
    setNotificationSettings,
    handleTestNotificationSound,
    handlePickWorkspaceDir,
    handleResetWorkspaceDir,
    fileIndexDashboard,
    isFileIndexDashboardLoading,
    loadFileIndexDashboard,
    recentDebugLogs,
    isDebugLogsLoading,
    loadRecentDebugLogs,
    openDebugLogDirectory,
    logStatus,
    pendingDiagnosticReports,
    diagnosticsActionBusy,
    handleOpenFeedbackReport,
    handleExportDiagnosticBundle,
    handleUploadPendingReport,
    handleDismissPendingReport,
    handleVersionTap,
    handleShowCurrentReleaseNotes,
    handleOpenDownloadPage,
    onOpenAppOnboarding,
    assistantDaemonStatus,
    assistantDaemonDraft,
    setAssistantDaemonDraft,
    assistantDaemonLogs,
    assistantDaemonBusy,
    assistantDaemonAcpToken,
    assistantDaemonWeixinLogin,
    assistantDaemonWeixinLoginBusy,
    loadAssistantDaemonStatus,
    handleSaveAssistantDaemonConfig,
    handleCreateAssistantDaemonAcpClient,
    handleRevokeAssistantDaemonAcpClient,
    handleStartAssistantDaemon,
    handleStopAssistantDaemon,
    handleStartAssistantDaemonWeixinLogin,
    handleCheckAssistantDaemonWeixinLogin,
    handleClearAssistantDaemonWeixinLogin,
    ecommercePlatformsSettings,
    handleToggleEcommercePlatform,
    isAdminUser,
    officialAiPanelEnabled,
    officialAiPanelRef,
    OfficialAiPanelComponent,
    reloadCustomAiSettings,
    handleOpenAiPricing,
    setShowAiModelSettings,
    showAiModelSettings,
    setTestStatus,
    setTestMsg,
    openCreateAiSourceModal,
    displayedAiSources,
    aiSourceExpandState,
    isOfficialManagedSource,
    hasOfficialManagedSource,
    aiSourceModelExpandState,
    getAddedSourceModelList,
    officialAuthPending,
    officialAuthLoggedIn,
    getLocalGuideForSource,
    isLocalAiSource,
    handleToggleAiSourceExpand,
    handleDeleteAiSource,
    officialAuthNeedsLogin,
    updateAiSource,
    groupedAiPresets,
    setActiveAiSourceId,
    setDetectedAiProtocol,
    handleToggleAiSourceModelExpand,
    openAddModelModal,
    handleSetSourceDefaultModel,
    handleRemoveSourceModel,
    activeAiSourceId,
    detectedAiProtocol,
    testStatus,
    testMsg,
    renderModelSettingsTableRow,
    chatSourceSelectValue,
    handleChatSourceChange,
    effectiveRouteModelValue,
    applyRouteModel,
    routeModelSelectOptions,
    chatRouteSourceModels,
    chatRouteSource,
    transcriptionSourceId,
    aiSources,
    handleLinkedSourceChange,
    setAiModelFormField,
    transcriptionSourceModels,
    getAiSourceById,
    routeScopeSourceValue,
    handleRouteScopeSourceChange,
    routeScopeSourceModels,
    routeScopeSource,
    isDashscopeImageTemplate,
    teamAdvisors,
    isTeamAdvisorsLoading,
    teamAdvisorBusyId,
    draggingTeamAdvisorId,
    handleCreateTeamAdvisor,
    handleToggleTeamAdvisorVisible,
    handleOpenTeamAdvisorSettings,
    handleTeamAdvisorDragStart,
    handleTeamAdvisorDragOver,
    handleTeamAdvisorDragEnd,
    openSkillMarketplace,
    loadSettingsSkills,
    isSettingsSkillsLoading,
    settingsSkillStatusMessage,
    settingsSkills,
    builtinSettingsSkills,
    setAreBuiltinSkillsExpanded,
    areBuiltinSkillsExpanded,
    renderSettingsSkillRow,
    editableSettingsSkills,
    isSkillMarketplaceOpen,
    setIsSkillMarketplaceOpen,
    loadSkillMarketplace,
    isSkillMarketplaceLoading,
    skillMarketplaceItems,
    skillMarketplaceBusyId,
    handleInstallMarketplaceSkill,
    mcpDraft,
    handleCancelMcpDraft,
    mcpDraftOriginalId,
    setMcpDraft,
    handleSaveMcpDraft,
    isSyncingMcp,
    loadMcpRuntimeData,
    handleDiscoverAndImportMcp,
    handleAddMcpServer,
    mcpStatusMessage,
    mcpServers,
    settingsMcpRuntimeMap,
    handleEditMcpServer,
    handleTestMcpServer,
    mcpTestingId,
    handleDeleteMcpServer,
    handleToggleMcpServer,
    onOpenRedClawOnboarding,
    redclawOnboardingCompleted,
    redclawProfileRoot,
    currentSpaceName,
    redclawProfileDirty,
    loadRedclawProfileBundle,
    isRedclawProfileLoading,
    setRedclawProfileDraft,
    savedRedclawProfileDraft,
    setRedclawProfileDirtyState,
    setRedclawProfileMessage,
    setStatus,
    redclawProfileMessage,
    redclawProfileDraft,
    handleRedclawProfileDraftChange,
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
    loadCliRuntimeDashboard,
    handleInspectCliRuntimeTool,
    handleDiagnoseCliRuntimeCommand,
    handleDiscoverCliRuntimeTools,
    handleCreateCliRuntimeEnvironment,
    handleInstallCliRuntimeTool,
    handleOpenCliRuntimeEnvironmentRoot,
    handleSaveMcpServers,
    mcpRuntimeItems,
    mcpLiveSessions,
    handleUpdateMcpServer,
    handleDisconnectMcpServer,
    handleDisconnectAllMcpSessions,
    mcpOauthState,
    handleRefreshMcpOAuth,
    mcpInspectingId,
    thrivePlugins,
    thrivePluginMarketplace,
    codexPluginMarketplace,
    thrivePluginMarketplaceLoading,
    codexPluginMarketplaceLoading,
    thrivePluginsLoading,
    thrivePluginBusyId,
    thrivePluginStatusMessage,
    thrivePluginRepoInput,
    setThrivePluginRepoInput,
    loadThrivePlugins,
    loadThrivePluginMarketplace,
    loadCodexPluginMarketplace,
    handleInstallThriveMarketplacePlugin,
    handleInstallCodexMarketplacePlugin,
    handleInstallThrivePluginFromRepo,
    handleToggleThrivePlugin,
    handleUninstallThrivePlugin,
    handleOpenThrivePluginDataDir,
    toolDiagnostics,
    toolDiagnosticResults,
    toolDiagnosticRunning,
    runToolDiagnostic,
    loadToolDiagnostics,
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
    runtimeTasks,
    runtimeRoles,
    runtimeDiagnosticsSummary,
    runtimeSessions,
    backgroundTasks,
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
    loadRuntimeDeveloperData,
    handleCreateRuntimeTask,
    handleResumeRuntimeTask,
    handleCancelRuntimeTask,
    handleCancelBackgroundTask,
    status,
    editingTeamAdvisor,
    isCreatingTeamAdvisor,
    handleSaveTeamAdvisor,
    setEditingTeamAdvisor,
    setIsCreatingTeamAdvisor,
    settingsTeamAdvisor,
    setSettingsTeamAdvisor,
    isActive,
    isTeamSystemPromptExpanded,
    setIsTeamSystemPromptExpanded,
    isTeamOptimizingPrompt,
    handleOptimizeTeamAdvisorPrompt,
    handleUploadTeamAdvisorKnowledge,
    handleDeleteTeamAdvisorKnowledge,
    handlePromoteTeamMemberSkillCandidate,
    handleDiscardTeamMemberSkillCandidate,
    handleRefreshTeamMemberSkill,
    handleRollbackTeamMemberSkillVersion,
    handleDeleteTeamAdvisor,
    isCreateAiSourceModalOpen,
    closeCreateAiSourceModal,
    createAiSourceDraft,
    setCreateAiSourceDraft,
    handleCreateAiSource,
    addModelModalSource,
    closeAddModelModal,
    addModelModalRemoteModels,
    addModelModalDraft,
    setSourceModelDrafts,
    addModelModalCapability,
    setSourceModelCapabilityDrafts,
    handleAddSourceModel,
    addModelModalDraftTrimmed,
  };
}
