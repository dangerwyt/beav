import clsx from 'clsx';
import { ArrowLeft, ChevronDown, Download, Plus, RefreshCw, Store, Trash2, X } from 'lucide-react';
import { type ModelCapability } from "../../../shared/modelCapabilities";
import {
  REDBOX_OFFICIAL_VIDEO_MODELS
} from "../../../shared/redboxVideo";
import {
  findAiPresetById
} from "../../config/aiSources";
import { APP_BRAND } from "../../config/brand";
import {
  formatMcpTime,
  RUNTIME_PERF_PRESETS
} from "../../features/settings/settingsModel";
import { AdvisorModal, AdvisorSettingsPanel } from "../Advisors";
import { EcommercePlatformsSettingsSection } from "./EcommercePlatformsSettingsSection";
import {
  ExperimentalSettingsSection,
  GeneralSettingsSection,
  RemoteConnectionSettingsSection,
  SettingsSaveBar,
  ToolsSettingsSection,
} from "./SettingsSections";
import {
  DEFAULT_CHAT_MAX_TOKENS,
  DEFAULT_CHAT_MAX_TOKENS_DEEPSEEK,
  MIN_CHAT_MAX_TOKENS,
  sanitizeChatMaxTokensInput,
} from "./settingsSupport";
import {
  AiModelSelect,
  AiPresetSelect,
  AiSourceLogo,
  buildModelCapabilityBadges,
  buildModelInputIcons,
  DASHSCOPE_LOCKED_IMAGE_MODEL,
  parseEnvText,
  PasswordInput,
  stringifyEnvRecord,
  type AiProtocol,
} from "./shared";
import { TeamSettingsSection } from "./TeamSettingsSection";
import type { useSettingsController } from './useSettingsController';

export function SettingsPageView({
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
}: ReturnType<typeof useSettingsController>) {
  return (
    <div className="flex h-full min-w-0 text-text-primary">
      {settingsSubView === 'ai-pricing' ? (
        renderAiPricingPage()
      ) : (
        <>
      {/* Sidebar */}
      <div className="w-48 border-r border-border pt-6 pb-4 flex flex-col gap-1 px-3 bg-surface-secondary/20">
        {onReturn && (
          <button
            type="button"
            onClick={onReturn}
            className="mb-4 flex items-center gap-2 rounded-lg px-3 py-2 text-sm font-medium text-text-secondary transition-colors hover:bg-surface-secondary/50 hover:text-text-primary"
          >
            <ArrowLeft className="h-4 w-4" />
            返回应用
          </button>
        )}
        <h1 className="px-3 mb-4 text-xs font-bold text-text-tertiary uppercase tracking-wider">{t('settings.title')}</h1>
        <div className="flex flex-1 flex-col gap-1 min-h-0">
          {tabs.map(tab => (
            <button
              key={tab.id}
              onClick={() => {
                setActiveTab(tab.id);
              }}
              className={clsx(
                "flex items-center gap-3 px-3 py-2 rounded-lg text-sm font-medium transition-colors",
                activeTab === tab.id ? "bg-surface-secondary text-text-primary" : "text-text-secondary hover:bg-surface-secondary/50 hover:text-text-primary"
              )}
            >
              <tab.icon className="w-4 h-4" />
              {t(tab.labelKey)}
            </button>
          ))}
        </div>
      </div>

      {/* Content */}
      <div className="min-w-0 flex-1 overflow-auto">
        <div
          className={clsx(
            'mx-auto px-8 py-8 pb-32',
            activeTab === 'ai' ? 'max-w-5xl' : activeTab === 'platforms' ? 'max-w-3xl' : 'max-w-2xl'
          )}
        >
          <form onSubmit={handleSave} className="space-y-10">

            {/* General Tab */}
            {activeTab === 'general' && (
              <GeneralSettingsSection
                appVersion={appVersion}
                formData={formData}
                setFormData={setFormData}
                notificationSettings={notificationSettings}
                setNotificationSettings={setNotificationSettings}
                handleTestNotificationSound={handleTestNotificationSound}
                handlePickWorkspaceDir={handlePickWorkspaceDir}
                handleResetWorkspaceDir={handleResetWorkspaceDir}
                fileIndexDashboard={fileIndexDashboard}
                fileIndexLoading={isFileIndexDashboardLoading}
                handleRefreshFileIndexDashboard={async () => {
                  await loadFileIndexDashboard({ force: true });
                }}
                recentDebugLogs={recentDebugLogs}
                isDebugLogsLoading={isDebugLogsLoading}
                handleRefreshDebugLogs={loadRecentDebugLogs}
                handleOpenDebugLogDir={openDebugLogDirectory}
                logStatus={logStatus}
                pendingReports={pendingDiagnosticReports}
                diagnosticsActionBusy={diagnosticsActionBusy}
                handleOpenFeedbackReport={handleOpenFeedbackReport}
                handleExportDiagnosticBundle={handleExportDiagnosticBundle}
                handleUploadPendingReport={handleUploadPendingReport}
                handleDismissPendingReport={handleDismissPendingReport}
                handleVersionTap={handleVersionTap}
                handleShowCurrentReleaseNotes={handleShowCurrentReleaseNotes}
                handleOpenDownloadPage={handleOpenDownloadPage}
                handleOpenAppOnboarding={onOpenAppOnboarding}
              />
            )}

            {/* Experimental Tab */}
            {activeTab === 'experimental' && (
              <ExperimentalSettingsSection
                formData={formData}
                setFormData={setFormData}
              />
            )}

            {activeTab === 'remote' && (
              <RemoteConnectionSettingsSection
                assistantDaemonStatus={assistantDaemonStatus}
                assistantDaemonDraft={assistantDaemonDraft}
                setAssistantDaemonDraft={setAssistantDaemonDraft}
                assistantDaemonLogs={assistantDaemonLogs}
                assistantDaemonBusy={assistantDaemonBusy}
                assistantDaemonAcpToken={assistantDaemonAcpToken}
                assistantDaemonWeixinLogin={assistantDaemonWeixinLogin}
                assistantDaemonWeixinLoginBusy={assistantDaemonWeixinLoginBusy}
                handleReloadAssistantDaemonStatus={loadAssistantDaemonStatus}
                handleSaveAssistantDaemonConfig={handleSaveAssistantDaemonConfig}
                handleCreateAssistantDaemonAcpClient={handleCreateAssistantDaemonAcpClient}
                handleRevokeAssistantDaemonAcpClient={handleRevokeAssistantDaemonAcpClient}
                handleStartAssistantDaemon={handleStartAssistantDaemon}
                handleStopAssistantDaemon={handleStopAssistantDaemon}
                handleStartAssistantDaemonWeixinLogin={handleStartAssistantDaemonWeixinLogin}
                handleCheckAssistantDaemonWeixinLogin={handleCheckAssistantDaemonWeixinLogin}
                handleClearAssistantDaemonWeixinLogin={handleClearAssistantDaemonWeixinLogin}
              />
            )}

            {activeTab === 'platforms' && (
              <EcommercePlatformsSettingsSection
                settings={ecommercePlatformsSettings}
                onTogglePlatform={handleToggleEcommercePlatform}
              />
            )}

            {/* AI Tab（仅管理员：服务器级模型路由配置） */}
            {activeTab === 'ai' && isAdminUser && (
              <div className="space-y-10">
                {/* LLM Connection Config */}
                <section className="space-y-6">
                  {officialAiPanelEnabled && (
                    <div ref={officialAiPanelRef} className="space-y-4 scroll-mt-6">
                      {OfficialAiPanelComponent ? (
                        <OfficialAiPanelComponent
                          onReloadSettings={reloadCustomAiSettings}
                          onOpenPricing={handleOpenAiPricing}
                        />
                      ) : (
                        <div className="rounded-xl border border-border bg-surface-secondary/20 p-4 text-sm text-text-tertiary">
                          正在加载账号信息...
                        </div>
                      )}
                    </div>
                  )}

                  <div className="overflow-visible rounded-xl border border-border bg-surface-secondary/20">
                    <button
                      type="button"
                      onClick={() => setShowAiModelSettings((prev) => !prev)}
                      className="flex w-full items-center justify-between gap-3 px-4 py-3 text-left transition-colors hover:bg-surface-secondary/40"
                      aria-expanded={showAiModelSettings}
                      aria-controls="ai-model-settings-panel"
                    >
                      <span className="text-sm font-medium text-text-primary">高级：自定义供应商</span>
                      <ChevronDown className={clsx('h-4 w-4 text-text-tertiary transition-transform', showAiModelSettings && 'rotate-180')} />
                    </button>

                    {showAiModelSettings && (
                      <div id="ai-model-settings-panel" className="space-y-4 border-t border-border/70 p-4">
                  <div className="space-y-4">
                    <div className="flex items-center justify-between gap-3">
	                      <div>
	                        <h3 className="text-sm font-medium text-text-primary">聊天供应商</h3>
	                        <p className="text-[11px] text-text-tertiary mt-1">
	                          支持多供应商、多模型；下方各能力使用同一套供应商和模型选择方式。
	                        </p>
	                      </div>
                      <div className="flex items-center gap-2">
                        <button
                          type="button"
                          onClick={() => {
                            setTestStatus('idle');
                            setTestMsg('');
                          }}
                          className="px-3 py-1.5 border border-border rounded text-xs hover:bg-surface-secondary transition-colors"
                        >
                          清除状态
                        </button>
                        <button
                          type="button"
                          onClick={openCreateAiSourceModal}
                          className="flex items-center gap-1.5 px-3 py-1.5 border border-border rounded text-xs hover:bg-surface-secondary transition-colors"
                        >
                          <Plus className="w-3 h-3" />
                          添加供应商
                        </button>
                      </div>
                    </div>

	                    <div className="rounded-xl border border-border bg-surface-secondary/20 p-2 space-y-2">
	                      {displayedAiSources.length ? displayedAiSources.map((source) => {
	                        const preset = findAiPresetById(source.presetId);
	                        const isExpanded = aiSourceExpandState[source.id] ?? false;
                        const isOfficialSource = isOfficialManagedSource(source);
                        const isOfficialPlaceholder = isOfficialSource && !hasOfficialManagedSource;
                        const isModelListExpanded = aiSourceModelExpandState[source.id] ?? false;
                        const sourceModels = getAddedSourceModelList(source);
                        const isOfficialSourcePending = isOfficialSource && officialAuthPending;
                        const isOfficialSourceLoggedIn = isOfficialSource && officialAuthLoggedIn;
                        const isOfficialSourceUnavailable = isOfficialSource && !officialAuthLoggedIn;
                        const sourceModelsForDisplay = isOfficialSource
                          ? (isOfficialSourceLoggedIn ? sourceModels : [])
                          : sourceModels;
                        const localGuide = getLocalGuideForSource(source);
                        const allowEmptyKey = isLocalAiSource(source);

                        return (
                          <div key={source.id} className="overflow-visible rounded-lg border border-border bg-surface-primary">
                            <div className="px-3 py-2 border-b border-border/70 flex items-center gap-2.5">
                              <button
                                type="button"
                                onClick={() => handleToggleAiSourceExpand(source.id)}
                                className="text-text-tertiary hover:text-text-primary transition-colors"
                                title={isExpanded ? '收起' : '展开'}
                              >
                                <ChevronDown className={clsx('w-4 h-4 transition-transform', !isExpanded && '-rotate-90')} />
                              </button>
                              <div className="min-w-0 flex-1">
                                <div className="flex items-center gap-2 min-w-0">
	                                  <AiSourceLogo source={source} />
	                                  <span className="text-sm font-medium text-text-primary truncate">{source.name || '未命名供应商'}</span>
	                                </div>
                                <p className="text-[11px] text-text-tertiary mt-0.5 truncate">
                                  {isOfficialSource
                                    ? isOfficialSourcePending
                                      ? '官方托管供应商 · 正在检查登录状态'
                                      : isOfficialSourceUnavailable
                                      ? '官方托管供应商 · 当前未登录，登录后自动同步官方模型与凭据'
                                      : `已托管登录态 · 默认模型：${source.model || '(未设置)'} · 已添加 ${sourceModelsForDisplay.length} 个模型`
                                    : `${preset?.label || 'Custom'} · 默认模型：${source.model || '(未设置)'} · 已添加 ${sourceModels.length} 个模型`}
                                </p>
                              </div>
                              {isOfficialSourceUnavailable ? (
                                <button
                                  type="button"
                                  onClick={() => setShowAiModelSettings(false)}
                                  className="px-2 py-1 text-[11px] border rounded transition-colors border-border text-text-secondary hover:text-text-primary hover:bg-surface-secondary"
                                  disabled={isOfficialSourcePending}
                                >
                                  {isOfficialSourcePending ? '检查中' : '查看账号'}
                                </button>
	                              ) : (
	                                <>
	                                  {!isOfficialSource && (
	                                    <button
                                      type="button"
                                      onClick={() => handleDeleteAiSource(source.id)}
                                      className="p-1.5 text-text-tertiary hover:text-red-500 hover:bg-red-500/10 rounded transition-colors"
                                      title="删除供应商"
                                    >
                                      <Trash2 className="w-3.5 h-3.5" />
                                    </button>
                                  )}
                                </>
                              )}
                            </div>

                            {isExpanded && (
                              <div className="p-3 space-y-3">
                                {isOfficialSourceUnavailable ? (
                                  <div className={clsx(
                                    'rounded border px-3 py-3 text-[11px] space-y-2',
                                    isOfficialSourcePending
                                      ? 'border-border bg-surface-secondary/30 text-text-secondary'
                                      : 'border-amber-500/25 bg-amber-500/5 text-text-secondary'
                                  )}>
                                    <div className={clsx(
                                      'font-medium',
                                      isOfficialSourcePending ? 'text-text-primary' : 'text-amber-600'
                                    )}>
                                      {isOfficialSourcePending
                                        ? '正在检查登录状态'
                                        : officialAuthNeedsLogin
                                        ? '当前账号登录已失效'
                                        : '当前账号未登录'}
                                    </div>
                                    <p>
                                      {isOfficialSourcePending
                                        ? '正在和宿主同步官方账号状态，完成后会自动刷新这里的模型与凭据。'
                                        : '官方供应商仍会固定显示在这里，但当前不会再使用旧模型和旧凭据。重新登录后会自动恢复同步。'}
                                    </p>
                                    {!isOfficialSourcePending && (
                                      <button
                                        type="button"
                                        onClick={() => setShowAiModelSettings(false)}
                                        className="px-3 py-1.5 border border-border rounded text-xs hover:bg-surface-secondary transition-colors"
                                      >
                                        查看账号
                                      </button>
                                    )}
                                  </div>
                                ) : isOfficialSource ? (
                                  <div className="rounded border border-emerald-500/20 bg-emerald-500/5 px-3 py-2 text-[11px] text-text-secondary">
                                    <div className="font-medium text-emerald-600">已登录</div>
                                  </div>
                                ) : (
                                  <>
                                    <div className="grid grid-cols-1 md:grid-cols-3 gap-2">
                                      <input
                                        type="text"
                                        value={source.name}
                                        onChange={(e) => updateAiSource(source.id, (prev) => ({ ...prev, name: e.target.value }))}
                                        placeholder="来源名称"
                                        className="w-full bg-surface-secondary/30 rounded border border-border px-3 py-2 text-sm focus:outline-none focus:border-accent-primary transition-colors"
                                      />
                                      <AiPresetSelect
                                        value={source.presetId}
                                        groups={groupedAiPresets}
                                        onChange={(nextPresetId) => {
                                          updateAiSource(source.id, (prev) => {
                                            const previousPreset = findAiPresetById(prev.presetId);
                                            const nextPreset = findAiPresetById(nextPresetId);
                                            const shouldSyncBaseURL = !prev.baseURL || (previousPreset?.baseURL && prev.baseURL === previousPreset.baseURL);
                                            const shouldSyncName = !prev.name || prev.name === previousPreset?.label;
                                            return {
                                              ...prev,
                                              presetId: nextPresetId,
                                              baseURL: shouldSyncBaseURL ? (nextPreset?.baseURL || '') : prev.baseURL,
                                              name: shouldSyncName ? (nextPreset?.label || prev.name) : prev.name,
                                              protocol: nextPreset?.protocol || prev.protocol || 'openai',
                                            };
                                          });
                                          setActiveAiSourceId(source.id);
                                        }}
                                      />
                                      <AiModelSelect
                                        value={source.protocol || 'openai'}
                                        onChange={(value) => {
                                          const protocol = value as AiProtocol;
                                          updateAiSource(source.id, (prev) => ({ ...prev, protocol }));
                                          setDetectedAiProtocol(protocol);
                                          setActiveAiSourceId(source.id);
                                        }}
                                        className="w-full"
                                        options={[
                                          { id: 'openai', label: 'OpenAI Compatible' },
                                          { id: 'anthropic', label: 'Anthropic Native' },
                                          { id: 'gemini', label: 'Gemini Native' },
                                        ]}
                                      />
                                    </div>

                                    <input
                                      type="text"
                                      value={source.baseURL}
                                      onChange={(e) => {
                                        updateAiSource(source.id, (prev) => ({ ...prev, baseURL: e.target.value }));
                                        setActiveAiSourceId(source.id);
                                      }}
                                      placeholder="API Endpoint (Base URL)"
                                      className="w-full bg-surface-secondary/30 rounded border border-border px-3 py-2 text-sm focus:outline-none focus:border-accent-primary transition-colors"
                                    />

                                    {localGuide && (
                                      <div className="rounded border border-border bg-surface-secondary/30 px-3 py-2 text-[11px] text-text-secondary space-y-1">
                                        <div className="font-medium text-text-primary">{localGuide.title}</div>
                                        <div className="font-mono">{localGuide.command}</div>
                                        <div className="text-text-tertiary">{localGuide.tip}</div>
                                      </div>
                                    )}

                                    <PasswordInput
                                      value={source.apiKey}
                                      onChange={(e) => {
                                        updateAiSource(source.id, (prev) => ({ ...prev, apiKey: e.target.value }));
                                        setActiveAiSourceId(source.id);
                                      }}
                                      placeholder={allowEmptyKey ? '本地源可留空' : 'API Key'}
                                      className="w-full bg-surface-secondary/30 rounded border border-border px-3 py-2 text-sm focus:outline-none focus:border-accent-primary transition-colors"
                                    />
                                  </>
                                )}

                                {(isOfficialSource && !isOfficialSourceLoggedIn) ? (
                                  <div className="rounded border border-dashed border-border px-2.5 py-2 text-[11px] text-text-tertiary">
                                    {isOfficialSourcePending
                                      ? '正在等待官方账号状态检查完成。'
                                      : '请先重新登录，登录后会同步官方模型配置。'}
                                  </div>
                                ) : (
                                  <div className="rounded border border-border bg-surface-secondary/20 p-2.5 space-y-2">
                                    <div className="flex items-center justify-between">
                                      <button
                                        type="button"
                                        onClick={() => handleToggleAiSourceModelExpand(source.id)}
                                        className="flex items-center gap-2 text-xs font-medium text-text-primary"
                                      >
                                        <ChevronDown className={clsx('w-3.5 h-3.5 transition-transform', !isModelListExpanded && '-rotate-90')} />
                                        已添加模型
                                      </button>
                                      <div className="flex items-center gap-2">
                                        <button
                                          type="button"
                                          onClick={() => openAddModelModal(source)}
                                          className="px-2 py-1 text-[11px] border border-border rounded hover:bg-surface-secondary transition-colors"
                                        >
                                          添加模型
                                        </button>
                                      </div>
                                    </div>

                                    {isModelListExpanded && (
                                      sourceModelsForDisplay.length ? (
                                      <div className="space-y-1">
                                        {sourceModelsForDisplay.map((model) => {
                                          const isDefaultModel = source.model === model.id;
                                          return (
                                            <div key={model.id} className="flex items-center justify-between gap-2 rounded border border-border bg-surface-primary px-2.5 py-1.5">
                                              <div className="min-w-0 flex items-center gap-2 flex-wrap">
                                                <button
                                                  type="button"
                                                  onClick={() => handleSetSourceDefaultModel(source.id, model.id)}
                                                  className={clsx(
                                                    'text-[10px] px-1.5 py-0.5 rounded border',
                                                    isDefaultModel
                                                      ? 'border-amber-500/40 text-amber-600 bg-amber-500/10'
                                                      : 'border-border text-text-tertiary hover:text-text-primary'
                                                  )}
                                                >
                                                  默认
                                                </button>
                                                <span className="text-xs text-text-primary truncate">{model.id}</span>
                                                {buildModelCapabilityBadges(model.capabilities).map((badge) => (
                                                  <span
                                                    key={`${model.id}-${badge.text}`}
                                                    className={clsx(
                                                      'px-1.5 py-0.5 rounded text-[10px] leading-none whitespace-nowrap font-medium',
                                                      badge.className || 'text-text-tertiary'
                                                    )}
                                                  >
                                                    {badge.text}
                                                  </span>
                                                ))}
                                                <span className="ml-0.5 flex items-center gap-1">
                                                  {buildModelInputIcons(model.inputCapabilities).map((icon) => {
                                                    const Icon = icon.icon;
                                                    return (
                                                      <span
                                                        key={`${model.id}-${icon.key}`}
                                                        title={icon.label}
                                                        className={clsx('inline-flex h-5 w-5 items-center justify-center rounded-full', icon.className)}
                                                      >
                                                        <Icon className="h-3.5 w-3.5" strokeWidth={2.1} />
                                                      </span>
                                                    );
                                                  })}
                                                </span>
                                              </div>
                                              <button
                                                type="button"
                                                onClick={() => handleRemoveSourceModel(source.id, model.id)}
                                                className="p-1 text-text-tertiary hover:text-red-500 hover:bg-red-500/10 rounded transition-colors"
                                                title="删除模型"
                                              >
                                                <Trash2 className="w-3 h-3" />
                                              </button>
                                            </div>
                                          );
                                        })}
                                      </div>
                                    ) : (
                                      <div className="text-[11px] text-text-tertiary rounded border border-dashed border-border px-2.5 py-2">
                                        暂无已添加模型，请先点击“添加模型”。
                                      </div>
                                      )
                                    )}
                                  </div>
                                )}

                                {activeAiSourceId === source.id && (
                                  <div className="flex items-center justify-between gap-2">
                                    <span className="text-[11px] text-text-tertiary">
                                      当前协议: <span className="font-mono">{detectedAiProtocol}</span>
                                    </span>
                                    <span
                                      className={clsx(
                                        'text-[11px]',
                                        testStatus === 'success' && 'text-status-success',
                                        testStatus === 'error' && 'text-status-error',
                                        testStatus === 'idle' && 'text-text-tertiary'
                                      )}
                                    >
                                      {testMsg || '等待操作'}
                                    </span>
                                  </div>
                                )}
                              </div>
                            )}
                          </div>
                        );
                      }) : (
                        <div className="rounded-lg border border-dashed border-border px-3 py-3 text-xs text-text-tertiary">
                          暂无供应商，请先点击“添加供应商”。
                        </div>
                      )}
                    </div>
                  </div>

	                    <div className="pt-4 border-t border-border space-y-3">
	                      <h3 className="text-sm font-medium text-text-primary">模型能力设置</h3>
	                      <div className="relative z-10 overflow-visible rounded-xl border border-border bg-surface-secondary/20">
	                        <div className="grid grid-cols-[132px_minmax(0,1fr)_minmax(0,1fr)] gap-2 bg-surface-secondary/35 px-3 py-2 text-xs font-medium text-text-secondary">
	                          <div>能力</div>
	                          <div>供应商</div>
	                          <div>模型</div>
	                        </div>
	                        {renderModelSettingsTableRow({
	                          label: '聊天',
	                          sourceValue: chatSourceSelectValue,
	                          sources: displayedAiSources,
	                          onSourceChange: handleChatSourceChange,
	                          modelValue: effectiveRouteModelValue('chat'),
	                          onModelChange: (modelId) => applyRouteModel('chat', modelId),
	                          modelOptions: routeModelSelectOptions(chatRouteSourceModels, effectiveRouteModelValue('chat')).map((model) => {
	                            const descriptor = chatRouteSourceModels.find((item) => item.id === model.id);
	                            return {
	                              ...model,
	                              badges: descriptor ? buildModelCapabilityBadges(descriptor.capabilities) : undefined,
	                              inputIcons: descriptor ? buildModelInputIcons(descriptor.inputCapabilities) : undefined,
	                            };
	                          }),
	                          modelDisabled: !chatRouteSource || chatRouteSourceModels.length === 0,
	                          modelPlaceholder: chatRouteSource && isOfficialManagedSource(chatRouteSource) && !officialAuthLoggedIn
	                            ? '请先登录官方账号'
	                            : '请先在该供应商中添加聊天模型',
	                        })}
	                        {renderModelSettingsTableRow({
	                          label: '转录',
	                          sourceValue: transcriptionSourceId,
	                          sources: aiSources,
	                          onSourceChange: (nextSourceId) => handleLinkedSourceChange('transcription', nextSourceId),
	                          modelValue: formData.transcription_model,
	                          onModelChange: (modelId) => setAiModelFormField('transcription_model', modelId),
	                          modelOptions: transcriptionSourceModels.map((model) => ({
	                            id: model.id,
	                            label: model.id,
	                            badges: buildModelCapabilityBadges(model.capabilities),
	                            inputIcons: buildModelInputIcons(model.inputCapabilities),
	                          })),
	                          modelDisabled: !transcriptionSourceModels.length,
	                          modelPlaceholder: '请先在该供应商中添加模型',
	                        })}
	                        <div className="border-t border-border/70 px-3 py-2">
	                          <div className="text-xs font-medium text-text-secondary">
	                            自定义转写服务（可选；与上面的供应商下拉二选一，填了这里就以这里为准）
	                          </div>
	                          <div className="mt-2 grid grid-cols-1 gap-2 md:grid-cols-2">
	                            <input
	                              value={formData.transcription_endpoint}
	                              onChange={(event) => setAiModelFormField('transcription_endpoint', event.target.value)}
	                              placeholder="地址：https://api.elevenlabs.io/v1/speech-to-text"
	                              className="w-full rounded-lg border border-border bg-surface-secondary px-3 py-2 text-sm focus:border-accent-primary focus:outline-none"
	                            />
	                            <input
	                              value={formData.transcription_key}
	                              onChange={(event) => setAiModelFormField('transcription_key', event.target.value)}
	                              placeholder="Key：你的 STT 服务密钥"
	                              type="password"
	                              className="w-full rounded-lg border border-border bg-surface-secondary px-3 py-2 text-sm focus:border-accent-primary focus:outline-none"
	                            />
	                          </div>
	                          <div className="mt-2 text-xs text-text-tertiary">
	                            模型名（不在上面的列表里时填这里）
	                          </div>
	                          <input
	                            value={formData.transcription_model}
	                            onChange={(event) => setAiModelFormField('transcription_model', event.target.value)}
	                            placeholder="ElevenLabs 直连：scribe_v2；relayrouter 中转：qwen3-asr-flash-filetrans"
	                            className="mt-1 w-full rounded-lg border border-border bg-surface-secondary px-3 py-2 text-sm focus:border-accent-primary focus:outline-none"
	                          />
	                        </div>
	                        {(() => {
	                          // 只读提示：显示**实际生效**的转写地址。
	                          //
	                          // 为什么需要：转写地址可以手填（ElevenLabs 之类），不一定对应
	                          // 任何「供应商」条目；而下拉框在匹配不上时会回退显示默认源，
	                          // 于是「下拉框选中的源」与「真正发请求的地址」可能对不上 ——
	                          // 用户以为在用自己的 STT，实际请求打到了别处（401 Invalid access token）。
	                          const effectiveTranscriptionEndpoint = String(
	                            formData.transcription_endpoint || '',
	                          ).trim();
	                          const linkedTranscriptionSource = getAiSourceById(transcriptionSourceId);
	                          const mismatch =
	                            effectiveTranscriptionEndpoint &&
	                            linkedTranscriptionSource &&
	                            String(linkedTranscriptionSource.baseURL || '').trim() !== effectiveTranscriptionEndpoint;
	                          if (!effectiveTranscriptionEndpoint || !mismatch) return null;
	                          return (
	                            <div className="border-t border-border/70 px-3 py-2 text-xs text-amber-600 dark:text-amber-400">
	                              实际生效的转写地址为「{effectiveTranscriptionEndpoint}」，与所选供应商不一致；
	                              保存时将以该地址为准。
	                            </div>
	                          );
	                        })()}
	                        {renderModelSettingsTableRow({
	                          label: 'Embedding',
	                          sourceValue: routeScopeSourceValue('embedding'),
	                          sources: displayedAiSources,
	                          onSourceChange: (nextSourceId) => handleRouteScopeSourceChange('embedding', 'embedding', nextSourceId, 'embedding'),
	                          modelValue: effectiveRouteModelValue('embedding'),
	                          onModelChange: (modelId) => applyRouteModel('embedding', modelId),
	                          modelOptions: routeModelSelectOptions(routeScopeSourceModels('embedding'), effectiveRouteModelValue('embedding')).map((model) => {
	                            const descriptor = routeScopeSourceModels('embedding').find((item) => item.id === model.id);
	                            return {
	                              ...model,
	                              badges: descriptor ? buildModelCapabilityBadges(descriptor.capabilities) : undefined,
	                              inputIcons: descriptor ? buildModelInputIcons(descriptor.inputCapabilities) : undefined,
	                            };
	                          }),
	                          modelDisabled: !routeScopeSource('embedding') || routeScopeSourceModels('embedding').length === 0,
	                          modelPlaceholder: '请先在该供应商中添加模型',
	                        })}
	                        {renderModelSettingsTableRow({
	                          label: '生图',
	                          sourceValue: routeScopeSourceValue('image'),
	                          sources: displayedAiSources,
	                          onSourceChange: (nextSourceId) => handleRouteScopeSourceChange('image', 'image', nextSourceId, 'image'),
	                          modelValue: effectiveRouteModelValue('image'),
	                          onModelChange: (modelId) => applyRouteModel('image', modelId),
	                          modelOptions: isDashscopeImageTemplate
	                            ? [{ id: DASHSCOPE_LOCKED_IMAGE_MODEL, label: DASHSCOPE_LOCKED_IMAGE_MODEL }]
	                            : routeModelSelectOptions(routeScopeSourceModels('image'), effectiveRouteModelValue('image')).map((model) => {
	                                const descriptor = routeScopeSourceModels('image').find((item) => item.id === model.id);
	                                return {
	                                  ...model,
	                                  badges: descriptor ? buildModelCapabilityBadges(descriptor.capabilities) : undefined,
	                                  inputIcons: descriptor ? buildModelInputIcons(descriptor.inputCapabilities) : undefined,
	                                };
	                              }),
	                          modelDisabled: isDashscopeImageTemplate || !routeScopeSource('image') || routeScopeSourceModels('image').length === 0,
	                          modelPlaceholder: isDashscopeImageTemplate ? DASHSCOPE_LOCKED_IMAGE_MODEL : '请先在该源中添加模型',
	                        })}
	                        {renderModelSettingsTableRow({
	                          label: 'TTS',
	                          sourceValue: routeScopeSourceValue('voiceTts'),
	                          sources: displayedAiSources,
	                          onSourceChange: (nextSourceId) => handleRouteScopeSourceChange('voiceTts', 'voice', nextSourceId, 'tts'),
	                          modelValue: effectiveRouteModelValue('voiceTts'),
	                          onModelChange: (modelId) => applyRouteModel('voiceTts', modelId),
	                          modelOptions: routeModelSelectOptions(routeScopeSourceModels('voiceTts'), effectiveRouteModelValue('voiceTts')).map((model) => {
	                            const descriptor = routeScopeSourceModels('voiceTts').find((item) => item.id === model.id);
	                            return {
	                              ...model,
	                              badges: descriptor ? buildModelCapabilityBadges(descriptor.capabilities) : undefined,
	                              inputIcons: descriptor ? buildModelInputIcons(descriptor.inputCapabilities) : undefined,
	                            };
	                          }),
	                          modelDisabled: !routeScopeSource('voiceTts') || routeScopeSourceModels('voiceTts').length === 0,
	                          modelPlaceholder: '请先在该供应商中添加 TTS 模型',
	                        })}
	                        {renderModelSettingsTableRow({
	                          label: '视觉索引',
	                          sourceValue: routeScopeSourceValue('visualIndex'),
	                          sources: displayedAiSources,
	                          onSourceChange: (nextSourceId) => handleRouteScopeSourceChange('visualIndex', 'visual', nextSourceId, 'chat'),
	                          modelValue: effectiveRouteModelValue('visualIndex'),
	                          onModelChange: (modelId) => applyRouteModel('visualIndex', modelId),
	                          modelOptions: routeModelSelectOptions(routeScopeSourceModels('visualIndex'), effectiveRouteModelValue('visualIndex')).map((model) => {
	                            const descriptor = routeScopeSourceModels('visualIndex').find((item) => item.id === model.id);
	                            return {
	                              ...model,
	                              badges: descriptor ? buildModelCapabilityBadges(descriptor.capabilities) : undefined,
	                              inputIcons: descriptor ? buildModelInputIcons(descriptor.inputCapabilities) : undefined,
	                            };
	                          }),
	                          modelDisabled: !routeScopeSource('visualIndex') || routeScopeSourceModels('visualIndex').length === 0,
	                          modelPlaceholder: '请先在该供应商中添加支持图片输入的模型',
	                        })}
	                        {renderModelSettingsTableRow({
	                          label: '视频分析',
	                          sourceValue: routeScopeSourceValue('videoAnalysis'),
	                          sources: displayedAiSources,
	                          onSourceChange: (nextSourceId) => handleRouteScopeSourceChange('videoAnalysis', 'videoAnalysis', nextSourceId, 'chat'),
	                          modelValue: effectiveRouteModelValue('videoAnalysis'),
	                          onModelChange: (modelId) => applyRouteModel('videoAnalysis', modelId),
	                          modelOptions: routeModelSelectOptions(routeScopeSourceModels('videoAnalysis'), effectiveRouteModelValue('videoAnalysis')).map((model) => {
	                            const descriptor = routeScopeSourceModels('videoAnalysis').find((item) => item.id === model.id);
	                            return {
	                              ...model,
	                              badges: descriptor ? buildModelCapabilityBadges(descriptor.capabilities) : undefined,
	                              inputIcons: descriptor ? buildModelInputIcons(descriptor.inputCapabilities) : undefined,
	                            };
	                          }),
	                          modelDisabled: !routeScopeSource('videoAnalysis') || routeScopeSourceModels('videoAnalysis').length === 0,
	                          modelPlaceholder: '请先在该供应商中添加支持视频输入的模型',
	                        })}
	                      </div>
                      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                        <div>
                          <label className="block text-xs font-medium text-text-secondary mb-1.5">直传上限（bytes）</label>
                          <input
                            type="number"
                            min={1048576}
                            max={536870912}
                            value={formData.video_analysis_max_direct_video_bytes}
                            onChange={(e) => setFormData((d) => ({ ...d, video_analysis_max_direct_video_bytes: e.target.value }))}
                            className="w-full rounded border border-border bg-surface-secondary/30 px-3 py-2 text-sm transition-colors focus:border-accent-primary focus:outline-none"
                          />
                        </div>
                        <div>
                          <label className="block text-xs font-medium text-text-secondary mb-1.5">协议</label>
                          <input
                            value={formData.video_analysis_protocol}
                            onChange={(e) => setFormData((d) => ({ ...d, video_analysis_protocol: e.target.value }))}
                            className="w-full rounded border border-border bg-surface-secondary/30 px-3 py-2 text-sm transition-colors focus:border-accent-primary focus:outline-none"
                          />
                        </div>
                      </div>
                  </div>

                  <div className="pt-4 border-t border-border space-y-3">
                    <h3 className="text-sm font-medium text-text-primary">生视频模型设置</h3>
                    <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
                        <div>
                          <div className="text-[11px] text-text-tertiary mb-1">文生视频</div>
                          <div className="text-sm font-medium text-text-primary">{REDBOX_OFFICIAL_VIDEO_MODELS['text-to-video']}</div>
                        </div>
                        <div>
                          <div className="text-[11px] text-text-tertiary mb-1">参考图视频</div>
                          <div className="text-sm font-medium text-text-primary">{REDBOX_OFFICIAL_VIDEO_MODELS['reference-guided']}</div>
                        </div>
                        <div>
                          <div className="text-[11px] text-text-tertiary mb-1">图片/首尾帧视频</div>
                          <div className="text-sm font-medium text-text-primary">{REDBOX_OFFICIAL_VIDEO_MODELS['first-last-frame']}</div>
                        </div>
                    </div>
                  </div>

                  <div className="pt-4 border-t border-border">
                    <h3 className="text-sm font-medium text-text-primary mb-4">聊天输出上限（max_tokens）</h3>
                    <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                      <div className="group">
                        <label className="block text-xs font-medium text-text-secondary mb-1.5">
                          通用模型 max_tokens
                        </label>
                        <input
                          type="number"
                          min={MIN_CHAT_MAX_TOKENS}
                          step={1}
                          value={formData.chat_max_tokens_default}
                          onChange={e => setFormData(d => ({ ...d, chat_max_tokens_default: e.target.value }))}
                          onBlur={e => setFormData(d => ({
                            ...d,
                            chat_max_tokens_default: sanitizeChatMaxTokensInput(e.target.value, DEFAULT_CHAT_MAX_TOKENS),
                          }))}
                          className="w-full bg-surface-secondary/30 rounded border border-border px-3 py-2 text-sm focus:outline-none focus:border-accent-primary transition-colors"
                        />
                        <p className="mt-1 text-[11px] text-text-tertiary">
                          默认 262144，最低 1024。用于除 DeepSeek 外的 OpenAI 兼容模型。
                        </p>
                      </div>

                      <div className="group">
                        <label className="block text-xs font-medium text-text-secondary mb-1.5">
                          DeepSeek max_tokens
                        </label>
                        <input
                          type="number"
                          min={MIN_CHAT_MAX_TOKENS}
                          step={1}
                          value={formData.chat_max_tokens_deepseek}
                          onChange={e => setFormData(d => ({ ...d, chat_max_tokens_deepseek: e.target.value }))}
                          onBlur={e => setFormData(d => ({
                            ...d,
                            chat_max_tokens_deepseek: sanitizeChatMaxTokensInput(e.target.value, DEFAULT_CHAT_MAX_TOKENS_DEEPSEEK),
                          }))}
                          className="w-full bg-surface-secondary/30 rounded border border-border px-3 py-2 text-sm focus:outline-none focus:border-accent-primary transition-colors"
                        />
                        <p className="mt-1 text-[11px] text-text-tertiary">
                          默认 131072，最低 1024。若服务端报 max_tokens 越界，可在此下调。
                        </p>
                      </div>
                    </div>
                  </div>

                  <div className="pt-4 border-t border-border">
                    <h3 className="text-sm font-medium text-text-primary mb-4">{APP_BRAND.aiDisplayName} 上下文压缩策略</h3>
                    <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                      <div className="group">
                        <label className="block text-xs font-medium text-text-secondary mb-1.5">
                          自动压缩阈值（tokens）
                        </label>
                        <input
                          type="number"
                          min={16000}
                          step={1000}
                          value={formData.redclaw_compact_target_tokens}
                          onChange={e => setFormData(d => ({ ...d, redclaw_compact_target_tokens: e.target.value }))}
                          className="w-full bg-surface-secondary/30 rounded border border-border px-3 py-2 text-sm focus:outline-none focus:border-accent-primary transition-colors"
                        />
                        <p className="mt-1 text-[11px] text-text-tertiary">
                          默认 256000。{APP_BRAND.aiDisplayName} 对话预计上下文超过该值时会自动 compact。
                        </p>
                      </div>
                    </div>
                  </div>

                  <div className="pt-4 border-t border-border">
                    <h3 className="text-sm font-medium text-text-primary mb-4">选题中心模式</h3>
                    <div className="bg-surface-secondary/30 rounded-lg border border-border p-4">
                      <div className="flex items-start justify-between gap-4">
                        <div>
                          <h4 className="text-sm font-medium text-text-primary">多选题模式</h4>
                          <p className="text-xs text-text-tertiary mt-1.5 leading-relaxed">
                            选题中心默认使用 Agent Runtime。关闭时每次生成 1 个方向；开启后每次基于同样素材一次性生成 3 个方向供选择。
                          </p>
                        </div>
                        <button
                          type="button"
                          onClick={() => setFormData((d) => ({ ...d, wander_deep_think_enabled: !d.wander_deep_think_enabled }))}
                          className="ui-switch-track shrink-0 mt-0.5"
                          data-size="md"
                          data-state={formData.wander_deep_think_enabled ? 'on' : 'off'}
                        >
                          <div className="ui-switch-thumb" />
                        </button>
                      </div>
                    </div>
                  </div>
                      </div>
                    )}
                  </div>

                </section>
              </div>
            )}

            {activeTab === 'team' && (
              <TeamSettingsSection
                advisors={teamAdvisors}
                loading={isTeamAdvisorsLoading}
                busyAdvisorId={teamAdvisorBusyId}
                draggingAdvisorId={draggingTeamAdvisorId}
                onCreateAdvisor={handleCreateTeamAdvisor}
                onToggleVisible={handleToggleTeamAdvisorVisible}
                onOpenSettings={handleOpenTeamAdvisorSettings}
                onDragStart={handleTeamAdvisorDragStart}
                onDragOver={handleTeamAdvisorDragOver}
                onDragEnd={handleTeamAdvisorDragEnd}
              />
            )}

            {activeTab === 'skills' && (
              <div className="space-y-6">
                <section className="space-y-4">
                  <div className="flex items-center justify-between gap-3">
                    <div>
                      <h2 className="text-lg font-medium text-text-primary">技能</h2>
                      <p className="mt-1 text-xs text-text-tertiary">管理当前可用技能。内置技能由系统依赖，始终保持打开。</p>
                    </div>
                    <div className="flex shrink-0 items-center gap-2">
                      <button
                        type="button"
                        onClick={openSkillMarketplace}
                        className="inline-flex items-center gap-2 rounded-md border border-border px-3 py-2 text-xs text-text-secondary transition-colors hover:bg-surface-secondary"
                      >
                        <Store className="h-3.5 w-3.5" />
                        技能市场
                      </button>
                      <button
                        type="button"
                        onClick={() => void loadSettingsSkills()}
                        disabled={isSettingsSkillsLoading}
                        className="inline-flex items-center gap-2 rounded-md border border-border px-3 py-2 text-xs text-text-secondary transition-colors hover:bg-surface-secondary disabled:opacity-50"
                      >
                        <RefreshCw className={clsx('h-3.5 w-3.5', isSettingsSkillsLoading && 'animate-spin')} />
                        刷新
                      </button>
                    </div>
                  </div>

                  {settingsSkillStatusMessage && (
                    <div className="rounded-lg border border-border bg-surface-secondary/30 px-3 py-2 text-xs text-text-secondary">
                      {settingsSkillStatusMessage}
                    </div>
                  )}
                </section>

                <section className="overflow-hidden rounded-xl border border-border bg-surface-primary">
                  {isSettingsSkillsLoading && settingsSkills.length === 0 ? (
                    <div className="flex items-center gap-2 px-4 py-5 text-sm text-text-tertiary">
                      <RefreshCw className="h-4 w-4 animate-spin" />
                      正在读取技能
                    </div>
                  ) : settingsSkills.length === 0 ? (
                    <div className="px-4 py-5 text-sm text-text-tertiary">暂无技能</div>
                  ) : (
                    <div className="divide-y divide-border">
                      {builtinSettingsSkills.length > 0 && (
                        <div>
                          <button
                            type="button"
                            onClick={() => setAreBuiltinSkillsExpanded((value) => !value)}
                            className="flex w-full items-center justify-between gap-3 px-4 py-3 text-left transition-colors hover:bg-surface-secondary/40"
                            aria-expanded={areBuiltinSkillsExpanded}
                          >
                            <div className="min-w-0">
                              <div className="flex min-w-0 items-center gap-2">
                                <ChevronDown className={clsx(
                                  'h-4 w-4 shrink-0 text-text-tertiary transition-transform',
                                  !areBuiltinSkillsExpanded && '-rotate-90'
                                )} />
                                <span className="text-sm font-medium text-text-primary">内置技能</span>
                                <span className="rounded-full bg-accent-primary/10 px-2 py-0.5 text-[10px] font-medium text-accent-primary">
                                  {builtinSettingsSkills.length}
                                </span>
                              </div>
                            </div>
                            <span className="shrink-0 text-xs text-text-tertiary">
                              {areBuiltinSkillsExpanded ? '收起' : '展开'}
                            </span>
                          </button>
                          {areBuiltinSkillsExpanded && (
                            <div className="divide-y divide-border border-t border-border bg-surface-secondary/10">
                              {builtinSettingsSkills.map(renderSettingsSkillRow)}
                            </div>
                          )}
                        </div>
                      )}
                      {editableSettingsSkills.map(renderSettingsSkillRow)}
                    </div>
                  )}
                </section>

                {isSkillMarketplaceOpen && (
                  <div
                    className="fixed inset-0 z-[140] flex items-center justify-center bg-black/45 px-5 py-6"
                    onMouseDown={() => setIsSkillMarketplaceOpen(false)}
                  >
                    <div
                      className="flex max-h-[82vh] w-full max-w-3xl flex-col overflow-hidden rounded-xl border border-border bg-surface-primary shadow-2xl"
                      onMouseDown={(event) => event.stopPropagation()}
                    >
                      <div className="flex items-center justify-between gap-3 border-b border-border px-4 py-3">
                        <h3 className="text-sm font-medium text-text-primary">技能市场</h3>
                        <div className="flex items-center gap-2">
                          <button
                            type="button"
                            onClick={() => void loadSkillMarketplace()}
                            disabled={isSkillMarketplaceLoading}
                            className="flex items-center gap-2 rounded border border-border px-3 py-1.5 text-xs font-medium text-text-primary transition-colors hover:bg-surface-secondary disabled:opacity-50"
                          >
                            <RefreshCw className={clsx('h-3 w-3', isSkillMarketplaceLoading && 'animate-spin')} />
                            刷新
                          </button>
                          <button
                            type="button"
                            onClick={() => setIsSkillMarketplaceOpen(false)}
                            className="flex h-8 w-8 items-center justify-center rounded border border-border text-text-secondary transition-colors hover:bg-surface-secondary hover:text-text-primary"
                            aria-label="关闭"
                          >
                            <X className="h-4 w-4" />
                          </button>
                        </div>
                      </div>
                      <div className="min-h-0 flex-1 overflow-auto">
                        {isSkillMarketplaceLoading && skillMarketplaceItems.length === 0 ? (
                          <div className="flex items-center gap-2 px-4 py-6 text-xs text-text-tertiary">
                            <RefreshCw className="h-3.5 w-3.5 animate-spin" />
                            正在读取市场
                          </div>
                        ) : skillMarketplaceItems.length === 0 ? (
                          <div className="px-4 py-8 text-center text-xs text-text-tertiary">市场暂无技能</div>
                        ) : (
                          <div className="divide-y divide-border">
                            {skillMarketplaceItems.map((skill) => {
                              const busy = skillMarketplaceBusyId === skill.id;
                              return (
                                <div key={`${skill.repo}:${skill.id}`} className="px-4 py-3">
                                  <div className="flex items-start justify-between gap-3">
                                    <div className="min-w-0">
                                      <div className="flex flex-wrap items-center gap-2">
                                        <div className="truncate text-sm font-medium text-text-primary">{skill.name}</div>
                                        {skill.installed ? (
                                          <span className="rounded bg-green-500/10 px-1.5 py-0.5 text-[10px] font-medium text-green-500">已安装</span>
                                        ) : null}
                                      </div>
                                      <div className="mt-1 line-clamp-2 text-xs leading-5 text-text-tertiary">
                                        {skill.description || skill.repo}
                                      </div>
                                      <div className="mt-2 flex flex-wrap gap-1.5">
                                        <span className="rounded border border-border px-1.5 py-0.5 font-mono text-[10px] text-text-tertiary">{skill.id}</span>
                                        <span className="rounded border border-border px-1.5 py-0.5 font-mono text-[10px] text-text-tertiary">{skill.repo}</span>
                                      </div>
                                    </div>
                                    <button
                                      type="button"
                                      onClick={() => void handleInstallMarketplaceSkill(skill)}
                                      disabled={busy || Boolean(skill.installed)}
                                      className="flex shrink-0 items-center gap-1.5 rounded bg-accent-primary px-2.5 py-1.5 text-xs font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50"
                                    >
                                      <Download className="h-3 w-3" />
                                      {skill.installed ? '已安装' : busy ? '安装中' : '安装'}
                                    </button>
                                  </div>
                                </div>
                              );
                            })}
                          </div>
                        )}
                      </div>
                    </div>
                  </div>
                )}
              </div>
            )}

            {activeTab === 'mcp' && (
              <div className="space-y-6">
                {mcpDraft ? (
                  <section className="mx-auto max-w-3xl space-y-4">
                    <button
                      type="button"
                      onClick={handleCancelMcpDraft}
                      className="inline-flex items-center gap-1 text-xs text-text-tertiary transition-colors hover:text-text-primary"
                    >
                      <ArrowLeft className="h-3.5 w-3.5" />
                      返回
                    </button>
                    <div>
                      <h2 className="text-xl font-medium text-text-primary">
                        {mcpDraftOriginalId ? '编辑 MCP Server' : '连接至自定义 MCP'}
                      </h2>
                    </div>
                    <div className="overflow-hidden rounded-xl border border-border bg-surface-primary">
                      <div className="space-y-3 border-b border-border p-3">
                        <label className="block text-xs font-medium text-text-secondary">名称</label>
                        <input
                          value={mcpDraft.name}
                          onChange={(event) => setMcpDraft((draft) => draft ? { ...draft, name: event.target.value } : draft)}
                          placeholder="MCP server name"
                          className="w-full rounded-lg border border-border bg-surface-secondary/20 px-3 py-2 text-sm text-text-primary outline-none transition-colors focus:border-accent-primary"
                        />
                        <div className="grid grid-cols-2 overflow-hidden rounded-lg border border-border bg-surface-secondary/20 p-0.5">
                          {([
                            ['stdio', 'STDIO'],
                            ['streamable-http', '流式 HTTP'],
                          ] as const).map(([transport, label]) => (
                            <button
                              key={transport}
                              type="button"
                              onClick={() => setMcpDraft((draft) => draft ? { ...draft, transport } : draft)}
                              className={clsx(
                                'rounded-md px-3 py-2 text-xs font-medium transition-colors',
                                mcpDraft.transport === transport
                                  ? 'bg-surface-primary text-text-primary shadow-sm'
                                  : 'text-text-tertiary hover:text-text-primary'
                              )}
                            >
                              {label}
                            </button>
                          ))}
                        </div>
                      </div>

                      {mcpDraft.transport === 'stdio' ? (
                        <>
                          <div className="space-y-3 border-b border-border p-3">
                            <label className="block text-xs font-medium text-text-secondary">启动命令</label>
                            <input
                              value={mcpDraft.command || ''}
                              onChange={(event) => setMcpDraft((draft) => draft ? { ...draft, command: event.target.value } : draft)}
                              placeholder="openai-dev-mcp"
                              className="w-full rounded-lg border border-border bg-surface-secondary/20 px-3 py-2 text-sm text-text-primary outline-none transition-colors focus:border-accent-primary"
                            />
                          </div>
                          <div className="space-y-2 border-b border-border p-3">
                            <label className="block text-xs font-medium text-text-secondary">参数</label>
                            {(mcpDraft.args && mcpDraft.args.length ? mcpDraft.args : ['']).map((arg, index) => (
                              <div key={index} className="flex items-center gap-2">
                                <input
                                  value={arg}
                                  onChange={(event) => setMcpDraft((draft) => {
                                    if (!draft) return draft;
                                    const args = [...(draft.args && draft.args.length ? draft.args : [''])];
                                    args[index] = event.target.value;
                                    return { ...draft, args };
                                  })}
                                  className="min-w-0 flex-1 rounded-lg border border-border bg-surface-secondary/20 px-3 py-2 text-sm text-text-primary outline-none transition-colors focus:border-accent-primary"
                                />
                                <button
                                  type="button"
                                  onClick={() => setMcpDraft((draft) => draft ? { ...draft, args: (draft.args || []).filter((_, itemIndex) => itemIndex !== index) } : draft)}
                                  className="rounded-md p-2 text-text-tertiary transition-colors hover:bg-surface-secondary hover:text-red-600"
                                  aria-label="删除参数"
                                >
                                  <Trash2 className="h-4 w-4" />
                                </button>
                              </div>
                            ))}
                            <button
                              type="button"
                              onClick={() => setMcpDraft((draft) => draft ? { ...draft, args: [...(draft.args || []), ''] } : draft)}
                              className="w-full rounded-lg bg-surface-secondary/40 px-3 py-2 text-xs text-text-secondary transition-colors hover:bg-surface-secondary"
                            >
                              + 添加参数
                            </button>
                          </div>
                          <div className="space-y-2 border-b border-border p-3">
                            <label className="block text-xs font-medium text-text-secondary">环境变量</label>
                            {Object.entries(mcpDraft.env && Object.keys(mcpDraft.env).length ? mcpDraft.env : { '': '' }).map(([key, value], index) => (
                              <div key={`${key}:${index}`} className="flex items-center gap-2">
                                <input
                                  value={key}
                                  onChange={(event) => setMcpDraft((draft) => {
                                    if (!draft) return draft;
                                    const entries = Object.entries(draft.env || {});
                                    entries[index] = [event.target.value, entries[index]?.[1] || ''];
                                    return { ...draft, env: Object.fromEntries(entries) };
                                  })}
                                  placeholder="键"
                                  className="min-w-0 flex-1 rounded-lg border border-border bg-surface-secondary/20 px-3 py-2 text-sm text-text-primary outline-none transition-colors focus:border-accent-primary"
                                />
                                <input
                                  value={value}
                                  onChange={(event) => setMcpDraft((draft) => {
                                    if (!draft) return draft;
                                    const entries = Object.entries(draft.env || {});
                                    entries[index] = [entries[index]?.[0] || '', event.target.value];
                                    return { ...draft, env: Object.fromEntries(entries) };
                                  })}
                                  placeholder="值"
                                  className="min-w-0 flex-1 rounded-lg border border-border bg-surface-secondary/20 px-3 py-2 text-sm text-text-primary outline-none transition-colors focus:border-accent-primary"
                                />
                                <button
                                  type="button"
                                  onClick={() => setMcpDraft((draft) => {
                                    if (!draft) return draft;
                                    const entries = Object.entries(draft.env || {}).filter((_, itemIndex) => itemIndex !== index);
                                    return { ...draft, env: Object.fromEntries(entries) };
                                  })}
                                  className="rounded-md p-2 text-text-tertiary transition-colors hover:bg-surface-secondary hover:text-red-600"
                                  aria-label="删除环境变量"
                                >
                                  <Trash2 className="h-4 w-4" />
                                </button>
                              </div>
                            ))}
                            <button
                              type="button"
                              onClick={() => setMcpDraft((draft) => draft ? { ...draft, env: { ...(draft.env || {}), '': '' } } : draft)}
                              className="w-full rounded-lg bg-surface-secondary/40 px-3 py-2 text-xs text-text-secondary transition-colors hover:bg-surface-secondary"
                            >
                              + 添加环境变量
                            </button>
                          </div>
                          <div className="space-y-2 border-b border-border p-3">
                            <label className="block text-xs font-medium text-text-secondary">环境变量传递</label>
                            {(mcpDraft.envPassthrough.length ? mcpDraft.envPassthrough : ['']).map((key, index) => (
                              <div key={index} className="flex items-center gap-2">
                                <input
                                  value={key}
                                  onChange={(event) => setMcpDraft((draft) => {
                                    if (!draft) return draft;
                                    const envPassthrough = [...(draft.envPassthrough.length ? draft.envPassthrough : [''])];
                                    envPassthrough[index] = event.target.value;
                                    return { ...draft, envPassthrough };
                                  })}
                                  className="min-w-0 flex-1 rounded-lg border border-border bg-surface-secondary/20 px-3 py-2 text-sm text-text-primary outline-none transition-colors focus:border-accent-primary"
                                />
                                <button
                                  type="button"
                                  onClick={() => setMcpDraft((draft) => draft ? { ...draft, envPassthrough: draft.envPassthrough.filter((_, itemIndex) => itemIndex !== index) } : draft)}
                                  className="rounded-md p-2 text-text-tertiary transition-colors hover:bg-surface-secondary hover:text-red-600"
                                  aria-label="删除传递变量"
                                >
                                  <Trash2 className="h-4 w-4" />
                                </button>
                              </div>
                            ))}
                            <button
                              type="button"
                              onClick={() => setMcpDraft((draft) => draft ? { ...draft, envPassthrough: [...draft.envPassthrough, ''] } : draft)}
                              className="w-full rounded-lg bg-surface-secondary/40 px-3 py-2 text-xs text-text-secondary transition-colors hover:bg-surface-secondary"
                            >
                              + 添加变量
                            </button>
                          </div>
                          <div className="space-y-3 p-3">
                            <label className="block text-xs font-medium text-text-secondary">工作目录</label>
                            <input
                              value={mcpDraft.cwd || ''}
                              onChange={(event) => setMcpDraft((draft) => draft ? { ...draft, cwd: event.target.value } : draft)}
                              placeholder="~/code"
                              className="w-full rounded-lg border border-border bg-surface-secondary/20 px-3 py-2 text-sm text-text-primary outline-none transition-colors focus:border-accent-primary"
                            />
                          </div>
                        </>
                      ) : (
                        <div className="space-y-3 p-3">
                          <label className="block text-xs font-medium text-text-secondary">URL</label>
                          <input
                            value={mcpDraft.url || ''}
                            onChange={(event) => setMcpDraft((draft) => draft ? { ...draft, url: event.target.value } : draft)}
                            placeholder="https://your-mcp-host/mcp"
                            className="w-full rounded-lg border border-border bg-surface-secondary/20 px-3 py-2 text-sm text-text-primary outline-none transition-colors focus:border-accent-primary"
                          />
                        </div>
                      )}
                    </div>
                    <div className="flex justify-end">
                      <button
                        type="button"
                        onClick={() => void handleSaveMcpDraft()}
                        disabled={isSyncingMcp}
                        className="rounded-lg bg-accent-primary px-4 py-2 text-sm font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50"
                      >
                        保存
                      </button>
                    </div>
                  </section>
                ) : (
                  <>
                    <section className="space-y-4">
                      <div className="flex items-center justify-between gap-3">
                        <div>
                          <h2 className="text-lg font-medium text-text-primary">MCP 服务器</h2>
                        </div>
                        <div className="flex items-center gap-2">
                          <button
                            type="button"
                            onClick={() => void loadMcpRuntimeData()}
                            disabled={isSyncingMcp}
                            className="inline-flex items-center gap-2 rounded-md border border-border px-3 py-2 text-xs text-text-secondary transition-colors hover:bg-surface-secondary disabled:opacity-50"
                          >
                            <RefreshCw className="h-3.5 w-3.5" />
                            刷新
                          </button>
                          <button
                            type="button"
                            onClick={() => void handleDiscoverAndImportMcp()}
                            disabled={isSyncingMcp}
                            className="rounded-md border border-border px-3 py-2 text-xs text-text-secondary transition-colors hover:bg-surface-secondary disabled:opacity-50"
                          >
                            {isSyncingMcp ? '导入中' : '导入'}
                          </button>
                          <button
                            type="button"
                            onClick={handleAddMcpServer}
                            disabled={isSyncingMcp}
                            className="rounded-md bg-accent-primary px-3 py-2 text-xs font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-50"
                          >
                            新增
                          </button>
                        </div>
                      </div>

                      {mcpStatusMessage && (
                        <div className="rounded-lg border border-border bg-surface-secondary/30 px-3 py-2 text-xs text-text-secondary">
                          {mcpStatusMessage}
                        </div>
                      )}
                    </section>

                    <section className="overflow-hidden rounded-xl border border-border bg-surface-primary">
                      {mcpServers.length === 0 ? (
                        <div className="px-4 py-5 text-sm text-text-tertiary">暂无 MCP Server</div>
                      ) : (
                        <div className="divide-y divide-border">
                          {mcpServers.map((server) => {
                            const enabled = server.enabled !== false;
                            const runtime = settingsMcpRuntimeMap[server.id];
                            const endpoint = server.transport === 'stdio'
                              ? [server.command, ...(server.args || [])].filter(Boolean).join(' ')
                              : server.url || '';
                            return (
                              <div key={server.id} className="flex items-center justify-between gap-4 px-4 py-3">
                                <div className="min-w-0">
                                  <div className="flex min-w-0 items-center gap-2">
                                    <span className="truncate text-sm font-medium text-text-primary">{server.name || server.id}</span>
                                    <span className="shrink-0 rounded-full bg-surface-secondary px-2 py-0.5 text-[10px] font-medium text-text-tertiary">
                                      {server.transport}
                                    </span>
                                    {runtime && (
                                      <span className="shrink-0 rounded-full bg-emerald-500/10 px-2 py-0.5 text-[10px] font-medium text-emerald-600">
                                        已连接
                                      </span>
                                    )}
                                  </div>
                                  <div className="mt-1 truncate font-mono text-xs text-text-tertiary">
                                    {endpoint || server.id}
                                  </div>
                                  {runtime && (
                                    <div className="mt-1 text-[11px] text-text-tertiary">
                                      calls {runtime.callCount} · tools {runtime.toolCount} · last {formatMcpTime(runtime.lastUsedAt)}
                                    </div>
                                  )}
                                </div>
                                <div className="flex shrink-0 items-center gap-2">
                                  <button
                                    type="button"
                                    onClick={() => handleEditMcpServer(server)}
                                    disabled={isSyncingMcp}
                                    className="rounded-md border border-border px-2.5 py-1.5 text-xs text-text-secondary transition-colors hover:bg-surface-secondary disabled:opacity-50"
                                  >
                                    编辑
                                  </button>
                                  <button
                                    type="button"
                                    onClick={() => void handleTestMcpServer(server)}
                                    disabled={mcpTestingId === server.id || isSyncingMcp}
                                    className="rounded-md border border-border px-2.5 py-1.5 text-xs text-text-secondary transition-colors hover:bg-surface-secondary disabled:opacity-50"
                                  >
                                    {mcpTestingId === server.id ? '测试中' : '测试'}
                                  </button>
                                  <button
                                    type="button"
                                    onClick={() => void handleDeleteMcpServer(server.id)}
                                    disabled={isSyncingMcp}
                                    className="rounded-md border border-red-500/30 px-2.5 py-1.5 text-xs text-red-600 transition-colors hover:bg-red-500/10 disabled:opacity-50"
                                  >
                                    删除
                                  </button>
                                  <button
                                    type="button"
                                    onClick={() => void handleToggleMcpServer(server)}
                                    disabled={isSyncingMcp}
                                    role="switch"
                                    aria-checked={enabled}
                                    aria-label={`${enabled ? '关闭' : '打开'} MCP Server ${server.name || server.id}`}
                                    title={enabled ? '关闭 MCP Server' : '打开 MCP Server'}
                                    className="ui-switch-track shrink-0 disabled:cursor-not-allowed disabled:opacity-60"
                                    data-size="sm"
                                    data-state={enabled ? 'on' : 'off'}
                                  >
                                    <span className="ui-switch-thumb" />
                                  </button>
                                </div>
                              </div>
                            );
                          })}
                        </div>
                      )}
                    </section>
                  </>
                )}
              </div>
            )}

            {/* Profile Tab */}
            {activeTab === 'profile' && (
              <div className="space-y-6">
                <section className="space-y-4">
                  <div className="flex items-center justify-between gap-3">
                    <div className="min-w-0">
                      <div className="flex items-center gap-3">
                        <h2 className="text-lg font-medium text-text-primary">创作档案</h2>
                        <button
                          type="button"
                          onClick={() => onOpenRedClawOnboarding?.()}
                          className="text-xs font-medium text-text-tertiary underline-offset-4 transition-colors hover:text-text-primary hover:underline"
                        >
                          {redclawOnboardingCompleted ? '重新自定义风格' : '去定义风格'}
                        </button>
                      </div>
                      <div className="mt-1 flex items-center gap-2 text-xs text-text-tertiary">
                        <span
                          className="rounded-full bg-surface-secondary px-2 py-1"
                          title={redclawProfileRoot || undefined}
                        >
                          空间：{currentSpaceName}
                        </span>
                        <span className={clsx(
                          'rounded-full px-2 py-1',
                          redclawProfileDirty
                            ? 'bg-amber-500/10 text-amber-600'
                            : 'bg-emerald-500/10 text-emerald-600'
                        )}>
                          {redclawProfileDirty ? '未保存' : '已同步'}
                        </span>
                      </div>
                    </div>
                    <div className="flex items-center gap-2">
                      <button
                        type="button"
                        onClick={() => void loadRedclawProfileBundle()}
                        disabled={isRedclawProfileLoading}
                        className="inline-flex items-center gap-2 rounded-md border border-border px-3 py-2 text-xs text-text-secondary transition-colors hover:bg-surface-secondary disabled:opacity-50"
                      >
                        <RefreshCw className={clsx('h-3.5 w-3.5', isRedclawProfileLoading && 'animate-spin')} />
                        {isRedclawProfileLoading ? '刷新中' : '刷新'}
                      </button>
                      <button
                        type="button"
                        onClick={() => {
                          setRedclawProfileDraft(savedRedclawProfileDraft);
                          setRedclawProfileDirtyState(false);
                          setRedclawProfileMessage(null);
                          setStatus('idle');
                        }}
                        disabled={!redclawProfileDirty}
                        className="inline-flex items-center gap-2 rounded-md border border-border px-3 py-2 text-xs text-text-secondary transition-colors hover:bg-surface-secondary disabled:opacity-50"
                      >
                        还原
                      </button>
                    </div>
                  </div>

                  {redclawProfileMessage && (
                    <div className={clsx(
                      'rounded-xl border px-4 py-3 text-sm',
                      redclawProfileMessage.tone === 'error'
                        ? 'border-red-500/25 bg-red-500/5 text-red-600'
                        : 'border-emerald-500/25 bg-emerald-500/5 text-emerald-600'
                    )}>
                      {redclawProfileMessage.text}
                    </div>
                  )}
                </section>

                <section className="space-y-4">
                  <div className="rounded-xl border border-border bg-surface-secondary/20 p-4">
                    <div className="mb-3">
                      <h3 className="text-sm font-medium text-text-primary">主档案</h3>
                      <p className="mt-1 text-xs leading-6 text-text-tertiary">
                        记录内容定位、目标受众、表达风格、运营策略、商业目标和长期边界。
                      </p>
                    </div>
                    <textarea
                      value={redclawProfileDraft.creatorProfile}
                      onChange={(event) => handleRedclawProfileDraftChange('creatorProfile', event.target.value)}
                      placeholder="# CreatorProfile.md"
                      spellCheck={false}
                      className="min-h-[360px] w-full rounded-lg border border-border bg-surface-primary px-4 py-3 font-mono text-sm leading-6 text-text-primary focus:border-accent-primary focus:outline-none"
                    />
                  </div>
                </section>
              </div>
            )}

            {/* Tools Tab */}
            {activeTab === 'tools' && (
              <ToolsSettingsSection
                cliRuntimeTools={cliRuntimeTools}
                cliRuntimeEnvironments={cliRuntimeEnvironments}
                cliRuntimeInstallDraft={cliRuntimeInstallDraft}
                setCliRuntimeInstallDraft={setCliRuntimeInstallDraft}
                cliRuntimeInstallQueue={cliRuntimeInstallQueue}
                cliRuntimeStatusMessage={cliRuntimeStatusMessage}
                isCliRuntimeRefreshing={isCliRuntimeRefreshing}
                cliRuntimeInstalling={cliRuntimeInstalling}
                cliRuntimeInspectingToolId={cliRuntimeInspectingToolId}
                cliRuntimeDiagnosticCommand={cliRuntimeDiagnosticCommand}
                setCliRuntimeDiagnosticCommand={setCliRuntimeDiagnosticCommand}
                cliRuntimeExecutionMode={cliRuntimeExecutionMode}
                setCliRuntimeExecutionMode={handleCliRuntimeExecutionModeChange}
                cliRuntimeDiscoverQuery={cliRuntimeDiscoverQuery}
                setCliRuntimeDiscoverQuery={setCliRuntimeDiscoverQuery}
                cliRuntimeDiscoverResults={cliRuntimeDiscoverResults}
                cliRuntimeDiscovering={cliRuntimeDiscovering}
                cliRuntimeCreatingEnvironment={cliRuntimeCreatingEnvironment}
                handleRefreshCliRuntime={loadCliRuntimeDashboard}
                handleInspectCliRuntimeTool={handleInspectCliRuntimeTool}
                handleDiagnoseCliRuntimeCommand={handleDiagnoseCliRuntimeCommand}
                handleDiscoverCliRuntimeTools={handleDiscoverCliRuntimeTools}
                handleCreateCliRuntimeEnvironment={handleCreateCliRuntimeEnvironment}
                handleInstallCliRuntimeTool={handleInstallCliRuntimeTool}
                handleOpenCliRuntimeEnvironmentRoot={handleOpenCliRuntimeEnvironmentRoot}
                isSyncingMcp={isSyncingMcp}
                handleDiscoverAndImportMcp={handleDiscoverAndImportMcp}
                handleAddMcpServer={handleAddMcpServer}
                handleSaveMcpServers={handleSaveMcpServers}
                mcpStatusMessage={mcpStatusMessage}
                mcpServers={mcpServers}
                mcpRuntimeItems={mcpRuntimeItems}
                mcpLiveSessions={mcpLiveSessions}
                handleUpdateMcpServer={handleUpdateMcpServer}
                handleDeleteMcpServer={handleDeleteMcpServer}
                handleDisconnectMcpServer={handleDisconnectMcpServer}
                handleDisconnectAllMcpSessions={handleDisconnectAllMcpSessions}
                stringifyEnvRecord={stringifyEnvRecord}
                parseEnvText={parseEnvText}
                mcpOauthState={mcpOauthState}
                handleRefreshMcpOAuth={handleRefreshMcpOAuth}
                handleTestMcpServer={handleTestMcpServer}
                mcpTestingId={mcpTestingId}
                mcpInspectingId={mcpInspectingId}
                thrivePlugins={thrivePlugins}
                thrivePluginMarketplace={thrivePluginMarketplace}
                codexPluginMarketplace={codexPluginMarketplace}
                thrivePluginMarketplaceLoading={thrivePluginMarketplaceLoading}
                codexPluginMarketplaceLoading={codexPluginMarketplaceLoading}
                thrivePluginsLoading={thrivePluginsLoading}
                thrivePluginBusyId={thrivePluginBusyId}
                thrivePluginStatusMessage={thrivePluginStatusMessage}
                thrivePluginRepoInput={thrivePluginRepoInput}
                setThrivePluginRepoInput={setThrivePluginRepoInput}
                handleRefreshThrivePlugins={loadThrivePlugins}
                handleRefreshThrivePluginMarketplace={loadThrivePluginMarketplace}
                handleRefreshCodexPluginMarketplace={loadCodexPluginMarketplace}
                handleInstallThriveMarketplacePlugin={handleInstallThriveMarketplacePlugin}
                handleInstallCodexMarketplacePlugin={handleInstallCodexMarketplacePlugin}
                handleInstallThrivePluginFromRepo={handleInstallThrivePluginFromRepo}
                handleToggleThrivePlugin={handleToggleThrivePlugin}
                handleUninstallThrivePlugin={handleUninstallThrivePlugin}
                handleOpenThrivePluginDataDir={handleOpenThrivePluginDataDir}
                showDeveloperDiagnostics={Boolean(formData.developer_mode_enabled)}
                toolDiagnostics={toolDiagnostics}
                toolDiagnosticResults={toolDiagnosticResults}
                toolDiagnosticRunning={toolDiagnosticRunning}
                handleRunDirectToolDiagnostic={(toolName) => runToolDiagnostic(toolName, 'direct')}
                handleRunAiToolDiagnostic={(toolName) => runToolDiagnostic(toolName, 'ai')}
                handleRefreshToolDiagnostics={loadToolDiagnostics}
                handleRunAllDirectToolDiagnostics={() => runAllToolDiagnostics('direct')}
                handleRunAllAiToolDiagnostics={() => runAllToolDiagnostics('ai')}
                runtimePerfPresets={RUNTIME_PERF_PRESETS}
                runtimePerfMode={runtimePerfMode}
                setRuntimePerfMode={setRuntimePerfMode}
                runtimePerfPresetId={runtimePerfPresetId}
                setRuntimePerfPresetId={setRuntimePerfPresetId}
                runtimePerfMessage={runtimePerfMessage}
                setRuntimePerfMessage={setRuntimePerfMessage}
                runtimePerfIterations={runtimePerfIterations}
                setRuntimePerfIterations={setRuntimePerfIterations}
                runtimePerfResults={runtimePerfResults}
                activeRuntimePerfRunId={activeRuntimePerfRunId}
                isRuntimePerfRunning={isRuntimePerfRunning}
                runtimePerfStatusMessage={runtimePerfStatusMessage}
                handleApplyRuntimePerfPreset={handleApplyRuntimePerfPreset}
                handleRunRuntimePerfBenchmark={handleRunRuntimePerfBenchmark}
                handleClearRuntimePerfResults={handleClearRuntimePerfResults}
                runtimeTasks={runtimeTasks}
                runtimeRoles={runtimeRoles}
                runtimeDiagnosticsSummary={runtimeDiagnosticsSummary}
                runtimeSessions={runtimeSessions}
                backgroundTasks={backgroundTasks}
                backgroundWorkerPool={backgroundWorkerPool}
                selectedRuntimeTaskId={selectedRuntimeTaskId}
                setSelectedRuntimeTaskId={setSelectedRuntimeTaskId}
                selectedRuntimeSessionId={selectedRuntimeSessionId}
                setSelectedRuntimeSessionId={setSelectedRuntimeSessionId}
                selectedBackgroundTaskId={selectedBackgroundTaskId}
                setSelectedBackgroundTaskId={setSelectedBackgroundTaskId}
                selectedBackgroundTask={selectedBackgroundTaskDetail}
                runtimeTaskTraces={runtimeTaskTraces}
                runtimeSessionTranscript={runtimeSessionTranscript}
                runtimeSessionCheckpoints={runtimeSessionCheckpoints}
                runtimeSessionToolResults={runtimeSessionToolResults}
                runtimeHooks={runtimeHooks}
                runtimeDraftInput={runtimeDraftInput}
                setRuntimeDraftInput={setRuntimeDraftInput}
                runtimeDraftMode={runtimeDraftMode}
                setRuntimeDraftMode={setRuntimeDraftMode}
                isRuntimeLoading={isRuntimeLoading}
                isRuntimeTraceLoading={isRuntimeTraceLoading}
                isRuntimeSessionLoading={isRuntimeSessionLoading}
                isBackgroundTasksLoading={isBackgroundTasksLoading}
                isRuntimeCreating={isRuntimeCreating}
                runtimeTaskActionRunning={runtimeTaskActionRunning}
                backgroundTaskActionRunning={backgroundTaskActionRunning}
                handleRefreshRuntimeData={loadRuntimeDeveloperData}
                handleCreateRuntimeTask={handleCreateRuntimeTask}
                handleResumeRuntimeTask={handleResumeRuntimeTask}
                handleCancelRuntimeTask={handleCancelRuntimeTask}
                handleCancelBackgroundTask={handleCancelBackgroundTask}
              />
            )}

            {/* Global Save Actions (Visible on all tabs usually, but maybe better inside the form only if relevant) */}
            {/* Actually, it's safer to keep the save button available for settings that need saving (General, AI). Tools operations are immediate. */}
            <SettingsSaveBar
              activeTab={activeTab}
              status={status}
            />
          </form>
          {(editingTeamAdvisor || isCreatingTeamAdvisor) && (
            <AdvisorModal
              advisor={editingTeamAdvisor}
              defaultMode="manual"
              onSave={handleSaveTeamAdvisor}
              onClose={() => {
                setEditingTeamAdvisor(null);
                setIsCreatingTeamAdvisor(false);
              }}
            />
          )}
          {settingsTeamAdvisor && (
            <>
              <button
                type="button"
                className="fixed inset-0 z-30 bg-black/20 backdrop-blur-[2px] transition-opacity"
                onClick={() => setSettingsTeamAdvisor(null)}
                aria-label="关闭成员设置"
              />
              <aside className="fixed bottom-4 right-4 top-4 z-40 w-[30rem] max-w-[calc(100vw-2rem)] overflow-hidden rounded-2xl border border-white/60 bg-white/85 shadow-[0_24px_64px_-16px_rgba(0,0,0,0.16)] backdrop-blur-[40px] animate-slide-in-right">
                <AdvisorSettingsPanel
                  advisor={settingsTeamAdvisor}
                  isActive={isActive}
                  downloadStatus={null}
                  isSystemPromptExpanded={isTeamSystemPromptExpanded}
                  setIsSystemPromptExpanded={setIsTeamSystemPromptExpanded}
                  isOptimizingPrompt={isTeamOptimizingPrompt}
                  onOptimizePrompt={() => void handleOptimizeTeamAdvisorPrompt(settingsTeamAdvisor)}
                  onUploadKnowledge={() => void handleUploadTeamAdvisorKnowledge(settingsTeamAdvisor)}
                  onDeleteKnowledge={(fileName) => void handleDeleteTeamAdvisorKnowledge(settingsTeamAdvisor, fileName)}
                  onPromoteMemberSkillCandidate={() => void handlePromoteTeamMemberSkillCandidate(settingsTeamAdvisor)}
                  onDiscardMemberSkillCandidate={() => void handleDiscardTeamMemberSkillCandidate(settingsTeamAdvisor)}
                  onRefreshMemberSkill={() => handleRefreshTeamMemberSkill(settingsTeamAdvisor)}
                  onRollbackMemberSkillVersion={(version) => void handleRollbackTeamMemberSkillVersion(settingsTeamAdvisor, version)}
                  onEdit={() => setEditingTeamAdvisor(settingsTeamAdvisor)}
                  onDelete={() => void handleDeleteTeamAdvisor(settingsTeamAdvisor)}
                  onClose={() => setSettingsTeamAdvisor(null)}
                />
              </aside>
            </>
          )}
          {isCreateAiSourceModalOpen && (
            <div
              className="fixed inset-0 z-[140] bg-black/45 flex items-center justify-center px-6 py-6"
              onMouseDown={closeCreateAiSourceModal}
            >
              <div
                className="w-full max-w-2xl rounded-2xl border border-border bg-surface-primary shadow-2xl"
                onMouseDown={(event) => event.stopPropagation()}
              >
                <div className="px-5 py-4 border-b border-border flex items-center justify-between gap-3">
                  <div className="min-w-0">
                    <h3 className="text-base font-semibold text-text-primary truncate">新建供应商</h3>
                    <p className="text-xs text-text-tertiary mt-1 truncate">
                      先创建供应商，再在该供应商下添加常用模型
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={closeCreateAiSourceModal}
                    className="px-2.5 py-1 text-xs border border-border rounded hover:bg-surface-secondary transition-colors"
                  >
                    关闭
                  </button>
                </div>

                <div className="px-5 py-4 space-y-4">
                  <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
                    <div className="space-y-1.5">
                      <label className="block text-[11px] font-medium text-text-secondary">平台预设</label>
                      <AiPresetSelect
                        value={createAiSourceDraft.presetId}
                        groups={groupedAiPresets}
                        onChange={(nextPresetId) => {
                          const previousPreset = findAiPresetById(createAiSourceDraft.presetId);
                          const nextPreset = findAiPresetById(nextPresetId);
                          const shouldSyncBaseURL = !createAiSourceDraft.baseURL
                            || (previousPreset?.baseURL && createAiSourceDraft.baseURL === previousPreset.baseURL);
                          const shouldSyncName = !createAiSourceDraft.name
                            || createAiSourceDraft.name === previousPreset?.label;
                          setCreateAiSourceDraft((prev) => ({
                            ...prev,
                            presetId: nextPresetId,
                            baseURL: shouldSyncBaseURL ? (nextPreset?.baseURL || '') : prev.baseURL,
                            name: shouldSyncName ? (nextPreset?.label || prev.name) : prev.name,
                            protocol: nextPreset?.protocol || prev.protocol || 'openai',
                          }));
                        }}
                      />
                    </div>
                    <div className="space-y-1.5">
                      <label className="block text-[11px] font-medium text-text-secondary">来源名称</label>
                      <input
                        type="text"
                        value={createAiSourceDraft.name}
                        onChange={(e) => setCreateAiSourceDraft((prev) => ({ ...prev, name: e.target.value }))}
                        placeholder="例如：DashScope (Qwen)"
                        className="w-full bg-surface-secondary/30 rounded border border-border px-3 py-2 text-sm focus:outline-none focus:border-accent-primary transition-colors"
                        autoFocus
                      />
                    </div>
                  </div>

	                  <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
	                    <div className="space-y-1.5">
	                      <label className="block text-[11px] font-medium text-text-secondary">协议类型</label>
	                      <AiModelSelect
                        value={createAiSourceDraft.protocol}
                        onChange={(value) => setCreateAiSourceDraft((prev) => ({ ...prev, protocol: value as AiProtocol }))}
                        className="w-full"
                        options={[
                          { id: 'openai', label: 'OpenAI Compatible' },
                          { id: 'anthropic', label: 'Anthropic Native' },
                          { id: 'gemini', label: 'Gemini Native' },
                        ]}
	                      />
	                    </div>
	                  </div>

                  <div className="space-y-1.5">
                    <label className="block text-[11px] font-medium text-text-secondary">API Endpoint (Base URL)</label>
                    <input
                      type="text"
                      value={createAiSourceDraft.baseURL}
                      onChange={(e) => setCreateAiSourceDraft((prev) => ({ ...prev, baseURL: e.target.value }))}
                      placeholder="https://api.openai.com/v1"
                      className="w-full bg-surface-secondary/30 rounded border border-border px-3 py-2 text-sm focus:outline-none focus:border-accent-primary transition-colors"
                    />
                  </div>

                  <div className="space-y-1.5">
                    <label className="block text-[11px] font-medium text-text-secondary">API Key</label>
                    <PasswordInput
                      value={createAiSourceDraft.apiKey}
                      onChange={(e) => setCreateAiSourceDraft((prev) => ({ ...prev, apiKey: e.target.value }))}
                      placeholder="可先留空，后续再补充"
                      className="w-full bg-surface-secondary/30 rounded border border-border px-3 py-2 text-sm focus:outline-none focus:border-accent-primary transition-colors"
                    />
                  </div>
                </div>

                <div className="px-5 py-4 border-t border-border flex items-center justify-end gap-2">
                  <button
                    type="button"
                    onClick={closeCreateAiSourceModal}
                    className="px-3 py-1.5 text-xs border border-border rounded hover:bg-surface-secondary transition-colors"
                  >
                    取消
                  </button>
                  <button
                    type="button"
                    onClick={handleCreateAiSource}
                    className="px-3 py-1.5 text-xs bg-text-primary text-background rounded hover:opacity-90 transition-opacity"
                  >
                    创建供应商
                  </button>
                </div>
              </div>
            </div>
          )}

          {addModelModalSource && (
            <div
              className="fixed inset-0 z-[140] bg-black/45 flex items-center justify-center px-6 py-6"
              onMouseDown={closeAddModelModal}
            >
              <div
                className="w-full max-w-xl rounded-2xl border border-border bg-surface-primary shadow-2xl"
                onMouseDown={(event) => event.stopPropagation()}
              >
                <div className="px-5 py-4 border-b border-border flex items-center justify-between gap-3">
                  <div className="min-w-0">
                    <h3 className="text-base font-semibold text-text-primary truncate">添加模型</h3>
                    <p className="text-xs text-text-tertiary mt-1 truncate">
                      {addModelModalSource.name || '未命名供应商'} · 已配置模型 {addModelModalRemoteModels.length} 个，可手动输入模型 ID
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={closeAddModelModal}
                    className="px-2.5 py-1 text-xs border border-border rounded hover:bg-surface-secondary transition-colors"
                  >
                    关闭
                  </button>
                </div>

                <div className="px-5 py-4 space-y-3">
                  <div className="text-[12px] text-text-tertiary">
                    可从已配置模型中选择，也可以直接手动输入模型 ID，点击确认后才会加入当前供应商。
                  </div>
                  <div className="grid grid-cols-1 md:grid-cols-[minmax(0,1fr),160px] gap-2">
                    <input
                      type="text"
                      list={`ai-source-model-options-${addModelModalSource.id}`}
                      value={addModelModalDraft}
                      onChange={(e) => setSourceModelDrafts((prev) => ({ ...prev, [addModelModalSource.id]: e.target.value }))}
                      placeholder="输入或选择模型ID"
                      className="flex-1 bg-surface-secondary/30 rounded border border-border px-3 py-2 text-sm focus:outline-none focus:border-accent-primary transition-colors"
                      autoFocus
                    />
                    <datalist id={`ai-source-model-options-${addModelModalSource.id}`}>
                      {addModelModalRemoteModels.map((item) => (
                        <option key={item.id} value={item.id} />
                      ))}
                    </datalist>
                    <select
                      value={addModelModalCapability}
                      onChange={(e) => setSourceModelCapabilityDrafts((prev) => ({
                        ...prev,
                        [addModelModalSource.id]: e.target.value as ModelCapability,
                      }))}
                      className="bg-surface-secondary/30 rounded border border-border px-3 py-2 text-sm focus:outline-none focus:border-accent-primary transition-colors"
                    >
                      <option value="chat">语言模型</option>
                      <option value="transcription">转录模型</option>
                      <option value="audio">音频生成</option>
                      <option value="tts">语音合成</option>
                      <option value="voice_clone">音色克隆</option>
                      <option value="image">图片生成</option>
                      <option value="video">视频生成</option>
                      <option value="embedding">向量模型</option>
                    </select>
                  </div>
                  <div className="max-h-40 overflow-auto rounded border border-border bg-surface-secondary/20 p-2">
                    {addModelModalRemoteModels.length ? (
                      <div className="flex flex-wrap gap-1.5">
                        {addModelModalRemoteModels.slice(0, 80).map((item) => (
                          <button
                            key={item.id}
                            type="button"
                            onClick={() => {
                              setSourceModelDrafts((prev) => ({ ...prev, [addModelModalSource.id]: item.id }));
                              setSourceModelCapabilityDrafts((prev) => ({
                                ...prev,
                                [addModelModalSource.id]: item.capabilities[0] || 'chat',
                              }));
                            }}
                            className="px-2 py-1 text-[11px] rounded border border-border hover:bg-surface-secondary transition-colors flex items-center gap-1.5"
                          >
                            <span>{item.id}</span>
                            {buildModelCapabilityBadges(item.capabilities).map((badge) => (
                              <span
                                key={`${item.id}-${badge.text}`}
                                className={clsx(
                                  'px-1 py-0.5 rounded text-[10px] leading-none whitespace-nowrap font-medium',
                                  badge.className || 'text-text-tertiary'
                                )}
                              >
                                {badge.text}
                              </span>
                            ))}
                            <span className="ml-0.5 flex items-center gap-1">
                              {buildModelInputIcons(item.inputCapabilities).map((icon) => {
                                const Icon = icon.icon;
                                return (
                                  <span
                                    key={`${item.id}-${icon.key}`}
                                    title={icon.label}
                                    className={clsx('inline-flex h-4.5 w-4.5 items-center justify-center rounded-full', icon.className)}
                                  >
                                    <Icon className="h-3 w-3" strokeWidth={2.1} />
                                  </span>
                                );
                              })}
                            </span>
                          </button>
                        ))}
                      </div>
                    ) : (
                      <div className="text-xs text-text-tertiary">
                        暂无已配置模型，可直接手动输入模型 ID。
                      </div>
                    )}
                  </div>
                </div>

                <div className="px-5 py-4 border-t border-border flex items-center justify-end gap-2">
                  <button
                    type="button"
                    onClick={closeAddModelModal}
                    className="px-3 py-1.5 text-xs border border-border rounded hover:bg-surface-secondary transition-colors"
                  >
                    取消
                  </button>
                  <button
                    type="button"
                    onClick={() => handleAddSourceModel(addModelModalSource.id)}
                    disabled={!addModelModalDraftTrimmed}
                    className="px-3 py-1.5 text-xs bg-text-primary text-background rounded hover:opacity-90 transition-opacity disabled:opacity-50"
                  >
                    确认添加
                  </button>
                </div>
              </div>
            </div>
          )}
        </div>
      </div>
        </>
      )}
    </div>
  );
}
