import type { Dispatch, MutableRefObject } from 'react';
import { useCallback, useEffect, useMemo, useRef, useState, type ComponentType, type SetStateAction } from 'react';
import { type ModelCapability } from "../../../shared/modelCapabilities";
import {
  AI_SOURCE_PRESETS,
  canonicalizeOfficialAutoSourceId,
  createOfficialAiSource,
  DEFAULT_AI_PRESET_ID,
  findAiPresetById,
  inferPresetIdByEndpoint,
  isOfficialAutoSourceId,
  OFFICIAL_AI_SOURCE_DISPLAY_NAME,
  OFFICIAL_AUTO_SOURCE_ID,
  type AiSourceConfig,
} from "../../config/aiSources";
import { APP_BRAND } from "../../config/brand";
import { hasOfficialAiPanel, loadOfficialAiPanelModule, type OfficialAiPanelProps } from "../../features/official";
import {
  AiModelRouteConfig,
  AiModelRoutes,
  AiModelRouteScope,
  cloneModelForVoiceTtsModel,
  DEFAULT_AI_MODEL_ROUTES,
  DEFAULT_VOICE_CLONE_MODEL,
  DEFAULT_VOICE_TTS_MODEL,
  SettingsTab,
} from "../../features/settings/settingsModel";
import { useOfficialAuthState } from "../../hooks/useOfficialAuthState";
import { CHAT_FAMILY_ROUTE_SCOPES, ELEVENLABS_TRANSCRIPTION_MODELS } from "./settingsSupport";
import {
  AiModelSelect,
  AiSourceSelect,
  createAiSourceDraftFromPreset,
  createAiSourceFromPreset,
  DASHSCOPE_LOCKED_IMAGE_MODEL,
  filterAiModelsByCapability,
  generateAiSourceId,
  inferImageTemplateByProvider,
  normalizeAiModelDescriptors,
  normalizeSourceModels,
  toAiModelDescriptor,
  type AiModelDescriptor,
  type AiModelOption,
  type AiPresetGroup,
  type AiProtocol,
  type CreateAiSourceDraft,
  type LocalAiGuide,
} from "./shared";

export function useSettingsAi({ setFormData, formData, activeTab, setTestStatus, setTestMsg, baseSettingsLoadedRef, setStatus, baseSettingsLoadedRevision }: {
  setFormData: Dispatch<any>;
  formData: any;
  activeTab: SettingsTab;
  setTestStatus: Dispatch<SetStateAction<"error" | "success" | "idle">>;
  setTestMsg: Dispatch<SetStateAction<string>>;
  baseSettingsLoadedRef: MutableRefObject<boolean>;
  setStatus: Dispatch<SetStateAction<"error" | "idle" | "saving" | "saved">>;
  baseSettingsLoadedRevision: number;
}) {

  const [aiModelRoutes, setAiModelRoutes] = useState<AiModelRoutes>(DEFAULT_AI_MODEL_ROUTES);
  const [aiSources, setAiSources] = useState<AiSourceConfig[]>([]);
  const [defaultAiSourceId, setDefaultAiSourceId] = useState('');
  const [activeAiSourceId, setActiveAiSourceId] = useState('');
  const [detectedAiProtocol, setDetectedAiProtocol] = useState<AiProtocol>('openai');
  const [aiSourceExpandState, setAiSourceExpandState] = useState<Record<string, boolean>>({});
  const [aiSourceModelExpandState, setAiSourceModelExpandState] = useState<Record<string, boolean>>({});
  const [sourceModelDrafts, setSourceModelDrafts] = useState<Record<string, string>>({});
  const [sourceModelCapabilityDrafts, setSourceModelCapabilityDrafts] = useState<Record<string, ModelCapability>>({});
  const [addModelModalSourceId, setAddModelModalSourceId] = useState('');
  const [isCreateAiSourceModalOpen, setIsCreateAiSourceModalOpen] = useState(false);
  const [createAiSourceDraft, setCreateAiSourceDraft] = useState<CreateAiSourceDraft>(() => createAiSourceDraftFromPreset(DEFAULT_AI_PRESET_ID));
  const [transcriptionSourceId, setTranscriptionSourceId] = useState('');
  const [embeddingSourceId, setEmbeddingSourceId] = useState('');
  const [visualIndexSourceId, setVisualIndexSourceId] = useState('');
  const [videoAnalysisSourceId, setVideoAnalysisSourceId] = useState('');
  const [imageSourceId, setImageSourceId] = useState('');
  const [voiceSourceId, setVoiceSourceId] = useState('');

  const aiSourceAutosaveTimerRef = useRef<number | null>(null);

  const aiSourceDraftDirtyRef = useRef(false);

  const aiSourceEditGenerationRef = useRef(0);

  const aiModelSettingsDraftDirtyRef = useRef(false);

  const aiModelSettingsEditGenerationRef = useRef(0);

  const markAiSourceDraftDirty = useCallback(() => {
    aiSourceDraftDirtyRef.current = true;
    aiSourceEditGenerationRef.current += 1;
  }, []);

  const markAiModelSettingsDraftDirty = useCallback(() => {
    aiModelSettingsDraftDirtyRef.current = true;
    aiModelSettingsEditGenerationRef.current += 1;
  }, []);

  const clearAiSourceDraftDirty = useCallback((expectedGeneration?: number) => {
    if (
      typeof expectedGeneration === 'number'
      && aiSourceEditGenerationRef.current !== expectedGeneration
    ) {
      return;
    }
    aiSourceDraftDirtyRef.current = false;
  }, []);

  const clearAiModelSettingsDraftDirty = useCallback((expectedGeneration?: number) => {
    if (
      typeof expectedGeneration === 'number'
      && aiModelSettingsEditGenerationRef.current !== expectedGeneration
    ) {
      return;
    }
    aiModelSettingsDraftDirtyRef.current = false;
  }, []);

  const officialAiSourcePlaceholder = useMemo(() => createOfficialAiSource(), []);

  const activeAiSource = useMemo(() => {
    const normalizedActiveId = canonicalizeOfficialAutoSourceId(activeAiSourceId);
    if (!aiSources.length) {
      return isOfficialAutoSourceId(normalizedActiveId) ? officialAiSourcePlaceholder : null;
    }
    const matchedSource = aiSources.find((source) => source.id === normalizedActiveId);
    if (matchedSource) return matchedSource;
    if (isOfficialAutoSourceId(normalizedActiveId)) return officialAiSourcePlaceholder;
    return aiSources[0] || null;
  }, [aiSources, activeAiSourceId, officialAiSourcePlaceholder]);

  const addModelModalSource = useMemo(() => {
    if (!addModelModalSourceId) return null;
    return aiSources.find((source) => source.id === addModelModalSourceId) || null;
  }, [aiSources, addModelModalSourceId]);

  const getSourceModelList = useCallback((source: AiSourceConfig) => {
    const merged = new Map<string, AiModelDescriptor>();
    for (const raw of (source.modelsMeta || [])) {
      const descriptor = toAiModelDescriptor(raw);
      if (!descriptor) continue;
      merged.set(descriptor.id, descriptor);
    }
    for (const raw of [...(source.models || []), source.model]) {
      const descriptor = toAiModelDescriptor(raw);
      if (!descriptor) continue;
      const previous = merged.get(descriptor.id);
      merged.set(descriptor.id, {
        id: descriptor.id,
        capabilities: Array.from(new Set([...(previous?.capabilities || []), ...descriptor.capabilities])),
        inputCapabilities: Array.from(new Set([...(previous?.inputCapabilities || []), ...descriptor.inputCapabilities])),
      });
    }
    return Array.from(merged.values());
  }, []);

  const getAddedSourceModelList = useCallback((source: AiSourceConfig) => {
    return normalizeAiModelDescriptors([
      ...(source.modelsMeta || []),
      ...(source.models || []).map((id) => ({ id })),
      source.model ? { id: source.model } : null,
    ]);
  }, []);

  const getAiSourceById = useCallback((sourceId: string): AiSourceConfig | null => {
    const normalizedSourceId = canonicalizeOfficialAutoSourceId(sourceId);
    if (!normalizedSourceId) return null;
    return aiSources.find((source) => source.id === normalizedSourceId)
      || (isOfficialAutoSourceId(normalizedSourceId) ? officialAiSourcePlaceholder : null);
  }, [aiSources, officialAiSourcePlaceholder]);

  const pickBestModelForSource = useCallback((
    source: AiSourceConfig | null,
    preferredModel?: string,
    capability: ModelCapability = 'chat',
  ): string => {
    if (!source) return '';
    const normalizedPreferredModel = String(preferredModel || '').trim();
    const sourceModels = getSourceModelList(source);
    const matchingModels = filterAiModelsByCapability(sourceModels, capability);
    if (normalizedPreferredModel && matchingModels.some((item) => item.id === normalizedPreferredModel)) {
      return normalizedPreferredModel;
    }
    const currentDefault = String(source.model || '').trim();
    if (currentDefault && matchingModels.some((item) => item.id === currentDefault)) {
      return currentDefault;
    }
    return String(matchingModels[0]?.id || currentDefault || sourceModels[0]?.id || '').trim();
  }, [getSourceModelList]);

  const pickCapabilityModelForSource = useCallback((
    source: AiSourceConfig | null,
    preferredModel: string,
    capability: ModelCapability,
  ): string => {
    if (!source) return '';
    const normalizedPreferredModel = String(preferredModel || '').trim();
    const matchingModels = filterAiModelsByCapability(getSourceModelList(source), capability);
    if (normalizedPreferredModel && matchingModels.some((item) => item.id === normalizedPreferredModel)) {
      return normalizedPreferredModel;
    }
    const currentDefault = String(source.model || '').trim();
    if (currentDefault && matchingModels.some((item) => item.id === currentDefault)) {
      return currentDefault;
    }
    return String(matchingModels[0]?.id || '').trim();
  }, [getSourceModelList]);

  const filterVisualIndexModels = useCallback((models: AiModelDescriptor[]): AiModelDescriptor[] => {
    const multimodalChatModels = models.filter((model) => (
      model.capabilities.includes('chat') && model.inputCapabilities.includes('image')
    ));
    if (multimodalChatModels.length > 0) return multimodalChatModels;
    const chatModels = filterAiModelsByCapability(models, 'chat');
    return chatModels.length > 0 ? chatModels : models;
  }, []);

  const filterVideoAnalysisModels = useCallback((models: AiModelDescriptor[]): AiModelDescriptor[] => {
    // 视频分析需要一个能"接收视频输入"的模型。官方源的 gemini-omni-flash / grokvideo 只在
    // `capabilities` 上标了 'video'（视频生成/理解能力），而 `inputCapabilities` 为 null，
    // 旧逻辑只看 inputCapabilities 会导致官方源永远解析不出视频模型、保存 AI 设置被卡。
    // 这里同时接受 capabilities 含 'video' 的模型，让下拉可选项与自动选取都能正常工作。
    return models.filter((model) => (
      model.inputCapabilities.includes('video') || model.capabilities.includes('video')
    ));
  }, []);

  const pickBestVisualIndexModelForSource = useCallback((
    source: AiSourceConfig | null,
    preferredModel?: string,
  ): string => {
    if (!source) return '';
    const normalizedPreferredModel = String(preferredModel || '').trim();
    const sourceModels = getSourceModelList(source);
    const visualModels = filterVisualIndexModels(sourceModels);
    if (normalizedPreferredModel && visualModels.some((item) => item.id === normalizedPreferredModel)) {
      return normalizedPreferredModel;
    }
    const currentDefault = String(source.model || '').trim();
    if (currentDefault && visualModels.some((item) => item.id === currentDefault)) {
      return currentDefault;
    }
    return String(visualModels[0]?.id || currentDefault || sourceModels[0]?.id || '').trim();
  }, [filterVisualIndexModels, getSourceModelList]);

  const pickBestVideoAnalysisModelForSource = useCallback((
    source: AiSourceConfig | null,
    preferredModel?: string,
  ): string => {
    if (!source) return '';
    const normalizedPreferredModel = String(preferredModel || '').trim();
    const sourceModels = getSourceModelList(source);
    const videoModels = filterVideoAnalysisModels(sourceModels);
    if (normalizedPreferredModel && videoModels.some((item) => item.id === normalizedPreferredModel)) {
      return normalizedPreferredModel;
    }
    const currentDefault = String(source.model || '').trim();
    if (currentDefault && videoModels.some((item) => item.id === currentDefault)) {
      return currentDefault;
    }
    return String(videoModels[0]?.id || '').trim();
  }, [filterVideoAnalysisModels, getSourceModelList]);

  const resolveLinkedSourceId = useCallback((options: {
    endpoint?: string;
    apiKey?: string;
    model?: string;
    fallbackId?: string;
  }): string => {
    if (!aiSources.length) return '';
    const normalizedEndpoint = String(options.endpoint || '').trim();
    const normalizedApiKey = String(options.apiKey || '').trim();
    const normalizedModel = String(options.model || '').trim();
    let bestSourceId = '';
    let bestScore = -1;

    for (const source of aiSources) {
      let score = 0;
      const sourceEndpoint = String(source.baseURL || '').trim();
      const sourceApiKey = String(source.apiKey || '').trim();
      const sourceModels = getSourceModelList(source).map((item) => item.id);
      if (normalizedEndpoint && sourceEndpoint === normalizedEndpoint) score += 4;
      if (normalizedApiKey && sourceApiKey && sourceApiKey === normalizedApiKey) score += 2;
      if (normalizedModel && sourceModels.includes(normalizedModel)) score += 1;
      if (score > bestScore) {
        bestScore = score;
        bestSourceId = source.id;
      }
    }

    if (bestScore > 0 && bestSourceId) return bestSourceId;
    const fallbackId = String(options.fallbackId || '').trim();
    if (fallbackId && aiSources.some((source) => source.id === fallbackId)) return fallbackId;
    return aiSources[0]?.id || '';
  }, [aiSources, getSourceModelList]);

  const inferImageRoutingFromSource = useCallback((source: AiSourceConfig) => {
    const presetId = String(source.presetId || inferPresetIdByEndpoint(source.baseURL || '') || '').trim().toLowerCase();
    if (presetId === 'buts') {
      return { provider: 'buts', template: 'dashscope-wan-native' };
    }
    if (presetId.includes('dashscope') || presetId.includes('qwen')) {
      return { provider: 'dashscope', template: 'dashscope-wan-native' };
    }
    if (presetId.includes('jimeng')) {
      return { provider: 'jimeng', template: 'jimeng-openai-wrapper' };
    }
    if (presetId.includes('ark')) {
      return { provider: 'ark-seedream', template: 'ark-seedream-native' };
    }
    if (presetId.includes('gemini')) {
      return { provider: 'gemini', template: 'gemini-openai-images' };
    }
    return { provider: 'openai-compatible', template: 'openai-images' };
  }, []);

  const handleLinkedSourceChange = useCallback((feature: 'transcription' | 'embedding' | 'visual' | 'videoAnalysis' | 'image' | 'voice' | 'video', nextSourceId: string) => {
    const source = getAiSourceById(nextSourceId);
    if (!source) return;
    markAiModelSettingsDraftDirty();

    if (feature === 'transcription') setTranscriptionSourceId(nextSourceId);
    if (feature === 'embedding') setEmbeddingSourceId(nextSourceId);
    if (feature === 'visual') setVisualIndexSourceId(nextSourceId);
    if (feature === 'videoAnalysis') setVideoAnalysisSourceId(nextSourceId);
    if (feature === 'image') setImageSourceId(nextSourceId);
    if (feature === 'voice') setVoiceSourceId(nextSourceId);
    setFormData((prev) => {
      if (feature === 'transcription') {
        return {
          ...prev,
          transcription_endpoint: String(source.baseURL || '').trim(),
          transcription_key: String(source.apiKey || '').trim(),
          transcription_model: pickBestModelForSource(source, prev.transcription_model, 'transcription'),
        };
      }
      if (feature === 'embedding') {
        return {
          ...prev,
          embedding_endpoint: String(source.baseURL || '').trim(),
          embedding_key: String(source.apiKey || '').trim(),
          embedding_model: pickBestModelForSource(source, prev.embedding_model, 'embedding'),
        };
      }
      if (feature === 'visual') {
        return {
          ...prev,
          visual_index_provider: 'openai-compatible',
          visual_index_endpoint: String(source.baseURL || '').trim(),
          visual_index_api_key: String(source.apiKey || '').trim(),
          visual_index_model: pickBestVisualIndexModelForSource(source, prev.visual_index_model),
        };
      }
      if (feature === 'videoAnalysis') {
        return {
          ...prev,
          video_analysis_endpoint: String(source.baseURL || '').trim(),
          video_analysis_api_key: String(source.apiKey || '').trim(),
          video_analysis_protocol: source.protocol || findAiPresetById(source.presetId)?.protocol || 'openai',
          video_analysis_model: pickBestVideoAnalysisModelForSource(source, prev.video_analysis_model),
        };
      }
      if (feature === 'video') {
        return prev;
      }
      if (feature === 'voice') {
        const ttsModel = pickBestModelForSource(source, prev.voice_tts_model || prev.tts_model, 'tts')
          || pickBestModelForSource(source, prev.voice_tts_model || prev.tts_model, 'audio');
        const nextTtsModel = ttsModel || prev.voice_tts_model || DEFAULT_VOICE_TTS_MODEL;
        const nextCloneModel = cloneModelForVoiceTtsModel(nextTtsModel, prev.voice_clone_model || DEFAULT_VOICE_CLONE_MODEL);
        return {
          ...prev,
          voice_endpoint: String(source.baseURL || '').trim(),
          voice_api_key: String(source.apiKey || '').trim(),
          voice_tts_model: nextTtsModel,
          tts_model: nextTtsModel,
          voice_clone_model: nextCloneModel,
        };
      }

      const nextRouting = inferImageRoutingFromSource(source);
      const nextTemplate = inferImageTemplateByProvider(nextRouting.provider, nextRouting.template);
      const nextModel = nextTemplate === 'dashscope-wan-native'
        ? DASHSCOPE_LOCKED_IMAGE_MODEL
        : pickBestModelForSource(source, prev.image_model, 'image');

      return {
        ...prev,
        image_provider: nextRouting.provider,
        image_provider_template: nextTemplate,
        image_endpoint: String(source.baseURL || '').trim(),
        image_api_key: String(source.apiKey || '').trim(),
        image_model: nextModel,
      };
    });
  }, [getAiSourceById, inferImageRoutingFromSource, markAiModelSettingsDraftDirty, pickBestModelForSource, pickBestVideoAnalysisModelForSource, pickBestVisualIndexModelForSource]);

  const selectedTranscriptionSource = useMemo(() => {
    return getAiSourceById(transcriptionSourceId);
  }, [getAiSourceById, transcriptionSourceId]);

  const selectedEmbeddingSource = useMemo(() => {
    return getAiSourceById(embeddingSourceId);
  }, [embeddingSourceId, getAiSourceById]);

  const selectedVisualIndexSource = useMemo(() => {
    return getAiSourceById(visualIndexSourceId);
  }, [getAiSourceById, visualIndexSourceId]);

  const selectedVideoAnalysisSource = useMemo(() => {
    return getAiSourceById(videoAnalysisSourceId);
  }, [getAiSourceById, videoAnalysisSourceId]);

  const selectedImageSource = useMemo(() => {
    return getAiSourceById(imageSourceId);
  }, [getAiSourceById, imageSourceId]);

  const selectedVoiceSource = useMemo(() => {
    return getAiSourceById(voiceSourceId);
  }, [getAiSourceById, voiceSourceId]);

  const transcriptionSourceModels = useMemo(() => {
    // ElevenLabs **直连**时，模型名不来自任何「供应商」条目（scribe_* 是
    // ElevenLabs 自己的模型，不在 relayrouter/百炼的模型列表里）。
    // 匹配不到源时模型下拉就是空的 → 用户只能去选一个不相干的源，
    // 结果模型名与地址对不上（实测：地址是 ElevenLabs、模型却是
    // relayrouter 的 qwen3-asr-flash-filetrans → 400 unsupported_model）。
    // 这里按实际生效的地址给出内置列表。
    const endpoint = String(formData.transcription_endpoint || '').trim().toLowerCase();
    if (endpoint.includes('elevenlabs.io')) {
      // 内置列表只有 id/label，但模型行渲染会读 capabilities / inputCapabilities：
      // 缺这两个字段会让 buildModelCapabilityBadges 对 undefined 调 .map，
      // 直接炸掉整个设置页（实测：配好 ElevenLabs 后点开「自定义供应商」即白屏，
      // 报 "undefined is not an object (evaluating 'r.map')"）。这里补齐成完整描述符。
      return ELEVENLABS_TRANSCRIPTION_MODELS.map((item): AiModelDescriptor => ({
        id: item.id,
        capabilities: ['transcription'],
        inputCapabilities: ['audio'],
      }));
    }
    return selectedTranscriptionSource ? filterAiModelsByCapability(getSourceModelList(selectedTranscriptionSource), 'transcription') : [];
  }, [getSourceModelList, selectedTranscriptionSource, formData.transcription_endpoint]);

  const embeddingSourceModels = useMemo(() => {
    return selectedEmbeddingSource ? filterAiModelsByCapability(getSourceModelList(selectedEmbeddingSource), 'embedding') : [];
  }, [getSourceModelList, selectedEmbeddingSource]);

  const visualIndexSourceModels = useMemo(() => {
    return selectedVisualIndexSource ? filterVisualIndexModels(getSourceModelList(selectedVisualIndexSource)) : [];
  }, [filterVisualIndexModels, getSourceModelList, selectedVisualIndexSource]);

  const videoAnalysisSourceModels = useMemo(() => {
    return selectedVideoAnalysisSource ? filterVideoAnalysisModels(getSourceModelList(selectedVideoAnalysisSource)) : [];
  }, [filterVideoAnalysisModels, getSourceModelList, selectedVideoAnalysisSource]);

  const imageSourceModels = useMemo(() => {
    return selectedImageSource ? filterAiModelsByCapability(getSourceModelList(selectedImageSource), 'image') : [];
  }, [getSourceModelList, selectedImageSource]);

  const voiceTtsSourceModels = useMemo(() => {
    const sourceModels = selectedVoiceSource ? getSourceModelList(selectedVoiceSource) : [];
    const ttsModels = filterAiModelsByCapability(sourceModels, 'tts');
    return ttsModels.length > 0 ? ttsModels : filterAiModelsByCapability(sourceModels, 'audio');
  }, [getSourceModelList, selectedVoiceSource]);

  const addModelModalRemoteModels = useMemo(() => {
    if (!addModelModalSource) return [];
    return getAddedSourceModelList(addModelModalSource);
  }, [addModelModalSource, getAddedSourceModelList]);

  const addModelModalDraft = addModelModalSource
    ? String(sourceModelDrafts[addModelModalSource.id] || '')
    : '';

  const addModelModalDraftTrimmed = addModelModalDraft.trim();

  const addModelModalCapability = addModelModalSource
    ? (sourceModelCapabilityDrafts[addModelModalSource.id] || 'chat')
    : 'chat';

  const groupedAiPresets = useMemo<AiPresetGroup[]>(() => {
    const codingPlan = AI_SOURCE_PRESETS.filter((preset) => preset.group === 'coding-plan');
    const general = AI_SOURCE_PRESETS.filter((preset) => preset.group !== 'coding-plan');
    return [
      { id: 'general', label: '通用供应商', items: general },
      { id: 'coding-plan', label: 'Coding Plan', items: codingPlan },
    ].filter((group) => group.items.length > 0);
  }, []);

  const [showAiModelSettings, setShowAiModelSettings] = useState(false);
  const [officialAiPanelEnabled, setOfficialAiPanelEnabled] = useState(false);
  const [OfficialAiPanelComponent, setOfficialAiPanelComponent] = useState<ComponentType<OfficialAiPanelProps> | null>(null);

  const officialAiPanelRef = useRef<HTMLDivElement | null>(null);

  const pendingOfficialAiPanelScrollRef = useRef(false);

  const { snapshot: officialAuthState, bootstrapped: officialAuthBootstrapped } = useOfficialAuthState();

  useEffect(() => {
    if (activeTab !== 'ai' || !officialAiPanelEnabled || !pendingOfficialAiPanelScrollRef.current) return;
    const handle = window.setTimeout(() => {
      officialAiPanelRef.current?.scrollIntoView({ block: 'start', behavior: 'smooth' });
      pendingOfficialAiPanelScrollRef.current = false;
    }, OfficialAiPanelComponent ? 80 : 180);
    return () => window.clearTimeout(handle);
  }, [OfficialAiPanelComponent, activeTab, officialAiPanelEnabled]);

  const isDeprecatedEmptyOpenAiSource = useCallback((source?: AiSourceConfig | null): boolean => {
    if (!source) return false;
    const presetId = String(source.presetId || '').trim().toLowerCase();
    const name = String(source.name || '').trim();
    const baseURL = String(source.baseURL || '').trim().replace(/\/+$/, '');
    const model = String(source.model || '').trim();
    const models = Array.isArray(source.models) ? source.models.map((item) => String(item || '').trim()).filter(Boolean) : [];
    const apiKey = String(source.apiKey || '').trim();
    return (
      presetId === 'openai'
      && name === 'OpenAI'
      && baseURL === 'https://api.openai.com/v1'
      && !apiKey
      && !model
      && models.length === 0
    );
  }, []);

  useEffect(() => {
    if (!hasOfficialAiPanel) {
      setOfficialAiPanelEnabled(false);
      setOfficialAiPanelComponent(null);
      return;
    }
    setOfficialAiPanelEnabled(true);
  }, []);

  useEffect(() => {
    if (!hasOfficialAiPanel || !officialAiPanelEnabled) return;
    if (activeTab !== 'ai' || OfficialAiPanelComponent) return;
    let canceled = false;
    void loadOfficialAiPanelModule().then((module) => {
      if (canceled) return;
      const nextComponent = module?.default || null;
      setOfficialAiPanelComponent(() => nextComponent);
    });
    return () => {
      canceled = true;
    };
  }, [OfficialAiPanelComponent, activeTab, officialAiPanelEnabled]);

  const isDashscopeImageTemplate = useMemo(() => {
    const template = inferImageTemplateByProvider(formData.image_provider, formData.image_provider_template);
    return template === 'dashscope-wan-native';
  }, [formData.image_provider, formData.image_provider_template]);

  const isLocalAiSource = useCallback((source?: { presetId?: string; baseURL?: string; protocol?: AiProtocol } | null): boolean => {
    if (!source) return false;
    if (source.protocol && source.protocol !== 'openai') return false;
    const preset = String(source.presetId || '').toLowerCase();
    const base = String(source.baseURL || '').toLowerCase();
    return (
      preset.endsWith('-local') ||
      preset.includes('local') ||
      base.includes('127.0.0.1') ||
      base.includes('localhost') ||
      base.includes('0.0.0.0') ||
      base.includes('::1')
    );
  }, []);

  const isOfficialManagedSource = useCallback((source?: {
    id?: string;
    name?: string;
    presetId?: string;
  } | null): boolean => {
    if (!source) return false;
    const sourceId = String(source.id || '').trim().toLowerCase();
    const sourceName = String(source.name || '').trim().toLowerCase();
    const presetId = String(source.presetId || '').trim().toLowerCase();
    return isOfficialAutoSourceId(sourceId)
      || sourceName === 'redbox official'
      || sourceName === `${APP_BRAND.displayName} official`.toLowerCase()
      || sourceName === OFFICIAL_AI_SOURCE_DISPLAY_NAME.toLowerCase()
      || presetId === 'redbox-official';
  }, []);

  const hasOfficialManagedSource = useMemo(
    () => aiSources.some((source) => isOfficialManagedSource(source)),
    [aiSources, isOfficialManagedSource]
  );

  const displayedAiSources = useMemo<AiSourceConfig[]>(() => {
    if (!officialAiPanelEnabled || hasOfficialManagedSource) {
      return aiSources;
    }
    return [
      officialAiSourcePlaceholder,
      ...aiSources,
    ];
  }, [aiSources, hasOfficialManagedSource, officialAiPanelEnabled, officialAiSourcePlaceholder]);

  const officialAuthStatus = String((officialAuthState as { status?: string } | null)?.status || '').trim();

  const officialAuthKnown = officialAuthBootstrapped;

  const officialAuthPending = !officialAuthBootstrapped
    || officialAuthStatus === 'restoring'
    || officialAuthStatus === 'refreshing';

  const officialAuthLoggedIn = officialAuthKnown
    && officialAuthStatus !== 'anonymous'
    && officialAuthStatus !== 'reauthRequired'
    && officialAuthStatus !== 'restoring'
    && Boolean((officialAuthState as { loggedIn?: boolean } | null)?.loggedIn);

  const officialAuthNeedsLogin = officialAuthKnown && !officialAuthPending && !officialAuthLoggedIn;

  const officialAiSource = useMemo(() => (
    displayedAiSources.find((source) => isOfficialManagedSource(source)) || null
  ), [displayedAiSources, isOfficialManagedSource]);

  const customAiSources = useMemo(() => (
    aiSources.filter((source) => !isOfficialManagedSource(source))
  ), [aiSources, isOfficialManagedSource]);

  const firstCustomAiSource = customAiSources[0] || null;

  const chatRouteSource = useMemo(() => (
    aiModelRoutes.chat.mode === 'official'
      ? officialAiSource
      : aiModelRoutes.chat.mode === 'custom'
        ? getAiSourceById(aiModelRoutes.chat.sourceId || '') || firstCustomAiSource
        : null
  ), [aiModelRoutes.chat.mode, aiModelRoutes.chat.sourceId, firstCustomAiSource, getAiSourceById, officialAiSource]);

  const chatRouteSourceModels = useMemo(() => {
    if (!chatRouteSource) return [];
    if (isOfficialManagedSource(chatRouteSource) && !officialAuthLoggedIn) return [];
    return filterAiModelsByCapability(getSourceModelList(chatRouteSource), 'chat');
  }, [chatRouteSource, filterAiModelsByCapability, getSourceModelList, isOfficialManagedSource, officialAuthLoggedIn]);

  const chatSourceSelectValue = useMemo(() => {
    if (aiModelRoutes.chat.mode === 'official') return OFFICIAL_AUTO_SOURCE_ID;
    return chatRouteSource?.id || aiModelRoutes.chat.sourceId || firstCustomAiSource?.id || '';
  }, [aiModelRoutes.chat.mode, aiModelRoutes.chat.sourceId, chatRouteSource?.id, firstCustomAiSource?.id]);

  // —— 非聊天能力行也统一读路由 ——
  //
  // 这些行原先读的是旧的 per-能力字段（embedding_source_id / image_endpoint / voice_tts_model…），
  // 于是出现「路由已经是 RelayRouter，界面还显示官方 + 请先登录官方账号」的错位；
  // 更糟的是保存时会把旧字段（多为官方地址 + 空 key）写回 settings，覆盖掉路由配置。
  // 这里改成和「聊天」行同一套来源：显示值/可选模型都从 aiModelRoutes 推出来。
  const routeScopeSource = useCallback((scope: AiModelRouteScope) => {
    const route = aiModelRoutes[scope];
    if (!route) return null;
    if (route.mode === 'official') return officialAiSource;
    if (route.mode === 'custom') return getAiSourceById(route.sourceId || '') || null;
    return null;
  }, [aiModelRoutes, getAiSourceById, officialAiSource]);

  const routeScopeSourceModels = useCallback((scope: AiModelRouteScope): AiModelDescriptor[] => {
    const source = routeScopeSource(scope);
    if (!source) return [];
    if (isOfficialManagedSource(source) && !officialAuthLoggedIn) return [];
    const models = getSourceModelList(source);
    switch (scope) {
      case 'embedding':
        return filterAiModelsByCapability(models, 'embedding');
      case 'image':
        return filterAiModelsByCapability(models, 'image');
      case 'voiceTts':
        return filterAiModelsByCapability(models, 'tts');
      case 'visualIndex':
        return filterVisualIndexModels(models);
      case 'videoAnalysis':
        return filterVideoAnalysisModels(models);
      default:
        return models;
    }
  }, [
    filterAiModelsByCapability,
    filterVideoAnalysisModels,
    filterVisualIndexModels,
    getSourceModelList,
    isOfficialManagedSource,
    officialAuthLoggedIn,
    routeScopeSource,
  ]);

  const routeScopeSourceValue = useCallback((scope: AiModelRouteScope) => {
    const route = aiModelRoutes[scope];
    if (route?.mode === 'official') return OFFICIAL_AUTO_SOURCE_ID;
    return routeScopeSource(scope)?.id || route?.sourceId || '';
  }, [aiModelRoutes, routeScopeSource]);

  const getLocalGuideForSource = useCallback((source?: AiSourceConfig | null): LocalAiGuide | null => {
    if (!source) return null;
    switch (source.presetId) {
      case 'ollama-local':
        return {
          title: 'Ollama 本地服务',
          command: 'ollama serve',
          tip: '建议先执行 `ollama pull <模型名>`，Endpoint 使用 http://127.0.0.1:11434/v1',
        };
      case 'lmstudio-local':
        return {
          title: 'LM Studio 本地服务',
          command: '在 LM Studio 中启动 Developer > Local Server',
          tip: '默认 Endpoint 为 http://127.0.0.1:1234/v1',
        };
      case 'vllm-local':
        return {
          title: 'vLLM 本地服务',
          command: 'vllm serve <model> --port 8000',
          tip: 'Endpoint 使用 http://127.0.0.1:8000/v1；如你配置了 --api-key，请在此填写对应 Key',
        };
      case 'localai-local':
        return {
          title: 'LocalAI 本地服务',
          command: 'docker run -p 8080:8080 localai/localai:latest',
          tip: 'Endpoint 使用 http://127.0.0.1:8080/v1；若设置了 LOCALAI_API_KEY，请同步填写 Key',
        };
      case 'llama-cpp-local':
        return {
          title: 'llama.cpp Server',
          command: 'llama-server -m model.gguf --port 8080',
          tip: 'Endpoint 使用 http://127.0.0.1:8080/v1；如启动时启用了 --api-key，请同步填写 Key',
        };
      default:
        return null;
    }
  }, []);

  useEffect(() => {
    setTestStatus('idle');
    setTestMsg('');
    setDetectedAiProtocol((activeAiSource?.protocol || 'openai') as AiProtocol);
  }, [activeAiSourceId, activeAiSource?.protocol]);

  useEffect(() => {
    if (!activeAiSource) return;
    setDetectedAiProtocol((activeAiSource?.protocol || 'openai') as AiProtocol);
  }, [activeAiSource?.id, activeAiSource?.baseURL, activeAiSource?.presetId, activeTab]);

  useEffect(() => {
    return undefined;
  }, []);

  const buildAiSourcePersistenceSnapshot = useCallback((
    sources: AiSourceConfig[] = aiSources,
    legacyChatSourceId: string = aiModelRoutes.chat.sourceId || defaultAiSourceId,
  ) => {
    const normalizedChatSourceId = canonicalizeOfficialAutoSourceId(legacyChatSourceId)
      || OFFICIAL_AUTO_SOURCE_ID;
    let sanitizedSources: AiSourceConfig[] = sources
      .map((source) => ({
        ...source,
        id: canonicalizeOfficialAutoSourceId(source.id),
        name: source.name.trim(),
        presetId: source.presetId.trim() || 'custom',
        baseURL: source.baseURL.trim(),
        apiKey: source.apiKey.trim(),
        models: normalizeSourceModels([...(source.models || []), source.model]),
        modelsMeta: normalizeAiModelDescriptors([
          ...(source.modelsMeta || []),
          ...(source.models || []).map((id) => ({ id })),
          source.model ? { id: source.model } : null,
        ]),
        model: String(source.model || '').trim(),
        protocol: source.protocol || findAiPresetById(source.presetId)?.protocol || 'openai',
      }))
      .map((source) => ({
        ...source,
        model: source.model || source.models?.[0] || '',
        models: normalizeSourceModels([...(source.models || []), source.model]),
        modelsMeta: normalizeAiModelDescriptors([
          ...(source.modelsMeta || []),
          ...(source.models || []).map((id) => ({ id })),
          source.model ? { id: source.model } : null,
        ]),
      }))
      .filter((source) => !isDeprecatedEmptyOpenAiSource(source));

    if (
      isOfficialAutoSourceId(normalizedChatSourceId)
      && !sanitizedSources.some((source) => isOfficialManagedSource(source))
    ) {
      sanitizedSources = [officialAiSourcePlaceholder, ...sanitizedSources];
    }

    const legacyChatSource = sanitizedSources.find((source) => source.id === normalizedChatSourceId)
      || (isOfficialAutoSourceId(normalizedChatSourceId) ? officialAiSourcePlaceholder : sanitizedSources[0]);
    return {
      sanitizedSources,
      resolvedLegacyChatSourceId: normalizedChatSourceId,
      legacyChatSource,
      resolvedApiEndpoint: String(legacyChatSource?.baseURL || '').trim(),
      resolvedApiKey: String(legacyChatSource?.apiKey || '').trim(),
      resolvedModelName: String(aiModelRoutes.chat.model || legacyChatSource?.model || '').trim(),
    };
  }, [aiModelRoutes.chat.model, aiModelRoutes.chat.sourceId, aiSources, defaultAiSourceId, isDeprecatedEmptyOpenAiSource, isOfficialManagedSource, officialAiSourcePlaceholder]);

  const persistAiSourcesSnapshot = useCallback(async (
    sources: AiSourceConfig[] = aiSources,
    legacyChatSourceId: string = aiModelRoutes.chat.sourceId || defaultAiSourceId,
  ) => {
    const saveGeneration = aiSourceEditGenerationRef.current;
    const snapshot = buildAiSourcePersistenceSnapshot(sources, legacyChatSourceId);
    await window.ipcRenderer.saveSettings({
      ai_sources_json: JSON.stringify(snapshot.sanitizedSources),
      default_ai_source_id: snapshot.resolvedLegacyChatSourceId || snapshot.legacyChatSource?.id || '',
      api_endpoint: snapshot.resolvedApiEndpoint,
      api_key: snapshot.resolvedApiKey,
      model_name: snapshot.resolvedModelName,
    });
    clearAiSourceDraftDirty(saveGeneration);
  }, [aiModelRoutes.chat.sourceId, aiSources, buildAiSourcePersistenceSnapshot, clearAiSourceDraftDirty, defaultAiSourceId]);

  const updateAiSource = useCallback((sourceId: string, updater: (source: AiSourceConfig) => AiSourceConfig) => {
    markAiSourceDraftDirty();
    setAiSources((prev) => prev.map((source) => (source.id === sourceId ? updater(source) : source)));
  }, [markAiSourceDraftDirty]);

  useEffect(() => {
    if (!baseSettingsLoadedRef.current) return;
    if (!aiSourceDraftDirtyRef.current) return;
    if (aiSourceAutosaveTimerRef.current != null) {
      window.clearTimeout(aiSourceAutosaveTimerRef.current);
    }
    aiSourceAutosaveTimerRef.current = window.setTimeout(() => {
      aiSourceAutosaveTimerRef.current = null;
      void persistAiSourcesSnapshot().catch((error) => {
        console.error('Failed to persist AI source snapshot:', error);
        setStatus('error');
        setTestStatus('error');
        setTestMsg(error instanceof Error ? error.message : '供应商配置自动保存失败');
      });
    }, 350);

    return () => {
      if (aiSourceAutosaveTimerRef.current != null) {
        window.clearTimeout(aiSourceAutosaveTimerRef.current);
        aiSourceAutosaveTimerRef.current = null;
      }
    };
  }, [aiSources, baseSettingsLoadedRevision, defaultAiSourceId, persistAiSourcesSnapshot]);

  const openCreateAiSourceModal = () => {
    setCreateAiSourceDraft(createAiSourceDraftFromPreset(DEFAULT_AI_PRESET_ID));
    setIsCreateAiSourceModalOpen(true);
  };

  const closeCreateAiSourceModal = () => {
    setIsCreateAiSourceModalOpen(false);
  };

  const handleCreateAiSource = () => {
    const preset = findAiPresetById(createAiSourceDraft.presetId) || findAiPresetById(DEFAULT_AI_PRESET_ID);
    const nextSource: AiSourceConfig = {
      id: generateAiSourceId(),
      name: String(createAiSourceDraft.name || '').trim() || preset?.label || '未命名供应商',
      presetId: createAiSourceDraft.presetId || preset?.id || 'custom',
      baseURL: String(createAiSourceDraft.baseURL || '').trim(),
      apiKey: String(createAiSourceDraft.apiKey || '').trim(),
      models: [],
      modelsMeta: [],
      model: '',
      protocol: createAiSourceDraft.protocol || preset?.protocol || 'openai',
    };

    markAiSourceDraftDirty();
    setAiSources((prev) => [...prev, nextSource]);
    setActiveAiSourceId(nextSource.id);
    setAiSourceExpandState((prev) => ({ ...prev, [nextSource.id]: true }));
    setIsCreateAiSourceModalOpen(false);
  };

  const handleDeleteAiSource = (sourceId: string) => {
    markAiSourceDraftDirty();
    setAiSources((prev) => {
      const next = prev.filter((source) => source.id !== sourceId);
      if (!next.length) {
        const fallback = createAiSourceFromPreset(DEFAULT_AI_PRESET_ID);
        setActiveAiSourceId(fallback.id);
        setDefaultAiSourceId(fallback.id);
        return [fallback];
      }
      setDefaultAiSourceId((prevDefaultId) => (prevDefaultId === sourceId ? next[0].id : prevDefaultId));
      setActiveAiSourceId((prevActiveId) => (prevActiveId === sourceId ? next[0].id : prevActiveId));
      return next;
    });
    setAiSourceExpandState((prev) => {
      const next = { ...prev };
      delete next[sourceId];
      return next;
    });
    setAiSourceModelExpandState((prev) => {
      const next = { ...prev };
      delete next[sourceId];
      return next;
    });
    setSourceModelDrafts((prev) => {
      const next = { ...prev };
      delete next[sourceId];
      return next;
    });
    setAddModelModalSourceId((prev) => (prev === sourceId ? '' : prev));
  };

  const handleToggleAiSourceExpand = (sourceId: string) => {
    const normalizedSourceId = canonicalizeOfficialAutoSourceId(sourceId);
    setAiSourceExpandState((prev) => {
      const currentExpanded = prev[normalizedSourceId] ?? false;
      if (currentExpanded) {
        return { ...prev, [normalizedSourceId]: false };
      }
      return displayedAiSources.reduce<Record<string, boolean>>((acc, source) => {
        acc[source.id] = source.id === normalizedSourceId;
        return acc;
      }, {});
    });
    setActiveAiSourceId(normalizedSourceId);
  };

  const handleToggleAiSourceModelExpand = (sourceId: string) => {
    setAiSourceModelExpandState((prev) => ({
      ...prev,
      [sourceId]: !(prev[sourceId] ?? false),
    }));
  };

  const ensureDisplayedAiSourcePersisted = (sourceId: string) => {
    const normalizedSourceId = canonicalizeOfficialAutoSourceId(sourceId);
    if (!normalizedSourceId) return;
    const displayedSource = displayedAiSources.find((source) => source.id === normalizedSourceId);
    if (!displayedSource) return;
    setAiSources((prev) => {
      if (prev.some((source) => source.id === normalizedSourceId)) return prev;
      return [displayedSource, ...prev];
    });
  };

  const applyAiModelRoutePatch = useCallback((scope: AiModelRouteScope, patch: Partial<AiModelRouteConfig>) => {
    markAiModelSettingsDraftDirty();
    setAiModelRoutes((prev) => {
      const targetScopes = scope === 'chat' ? CHAT_FAMILY_ROUTE_SCOPES : [scope];
      const next = { ...prev } as AiModelRoutes;
      for (const targetScope of targetScopes) {
        next[targetScope] = {
          ...prev[targetScope],
          ...patch,
        };
      }
      setFormData((data) => {
        const nextData = {
          ...data,
          ai_model_routes_json: JSON.stringify(next),
        };
        if (scope === 'chat' && typeof patch.model === 'string') {
          nextData.model_name = patch.model;
          nextData.model_name_wander = patch.model;
          nextData.model_name_chatroom = patch.model;
          nextData.model_name_knowledge = patch.model;
          nextData.model_name_redclaw = patch.model;
        }
        return nextData;
      });
      return next;
    });
  }, [markAiModelSettingsDraftDirty]);

  const handleSetSourceDefaultModel = (sourceId: string, modelId: string) => {
    const normalizedSourceId = canonicalizeOfficialAutoSourceId(sourceId);
    const normalizedModel = String(modelId || '').trim();
    if (!normalizedModel) return;
    const applyModel = (source: AiSourceConfig): AiSourceConfig => ({
      ...source,
      model: normalizedModel,
      models: normalizeSourceModels([...(source.models || []), normalizedModel]),
      modelsMeta: normalizeAiModelDescriptors([
        ...(source.modelsMeta || []),
        ...((source.models || []).map((id) => ({ id }))),
        { id: normalizedModel, capabilities: (getSourceModelList(source).find((item) => item.id === normalizedModel)?.capabilities || ['chat']) },
      ]),
    });
    markAiSourceDraftDirty();
    markAiModelSettingsDraftDirty();
    setAiSources((prev) => {
      let found = false;
      const next = prev.map((source) => {
        if (canonicalizeOfficialAutoSourceId(source.id) !== normalizedSourceId) return source;
        found = true;
        return applyModel(source);
      });
      if (found) return next;
      const displayedSource = displayedAiSources.find((source) => canonicalizeOfficialAutoSourceId(source.id) === normalizedSourceId);
      return displayedSource ? [applyModel(displayedSource), ...next] : next;
    });
    const chatSourceId = canonicalizeOfficialAutoSourceId(aiModelRoutes.chat.sourceId || defaultAiSourceId);
    if (
      chatSourceId === normalizedSourceId
      || (isOfficialAutoSourceId(normalizedSourceId) && aiModelRoutes.chat.mode === 'official')
    ) {
      applyAiModelRoutePatch('chat', { model: normalizedModel });
    }
  };

  const getRouteSource = useCallback((route: AiModelRouteConfig): AiSourceConfig | null => {
    if (route.mode === 'official') return officialAiSource;
    if (route.mode === 'custom') {
      return getAiSourceById(route.sourceId || '') || firstCustomAiSource;
    }
    return null;
  }, [firstCustomAiSource, getAiSourceById, officialAiSource]);

  const updateAiModelRoute = useCallback((scope: AiModelRouteScope, patch: Partial<AiModelRouteConfig>) => {
    applyAiModelRoutePatch(scope, patch);
  }, [applyAiModelRoutePatch]);

  /**
   * 切换某能力行的供应商：同时写路由与旧字段。
   *
   * 旧字段（*_endpoint / *_key / *_model）仍要同步一份 —— media_transcribe、image_endpoint
   * 这些链路读的是它，不同步会出现「路由对了、实际请求打到旧地址」。
   */
  const handleRouteScopeSourceChange = useCallback((
    scope: AiModelRouteScope,
    feature: 'transcription' | 'embedding' | 'visual' | 'videoAnalysis' | 'image' | 'voice' | 'video',
    nextSourceId: string,
    capability: ModelCapability,
  ) => {
    handleLinkedSourceChange(feature, nextSourceId);
    const source = getAiSourceById(nextSourceId)
      || (isOfficialAutoSourceId(nextSourceId) ? officialAiSource : null);
    if (!source) return;
    const isOfficialSource = isOfficialManagedSource(source);
    updateAiModelRoute(scope, {
      mode: isOfficialSource ? 'official' : 'custom',
      sourceId: isOfficialSource ? OFFICIAL_AUTO_SOURCE_ID : source.id,
      model: pickBestModelForSource(source, aiModelRoutes[scope]?.model || '', capability),
    });
  }, [
    aiModelRoutes,
    getAiSourceById,
    handleLinkedSourceChange,
    isOfficialManagedSource,
    officialAiSource,
    pickBestModelForSource,
    updateAiModelRoute,
  ]);

  const handleChatSourceChange = useCallback((nextSourceId: string) => {
    const normalizedSourceId = canonicalizeOfficialAutoSourceId(nextSourceId);
    const source = displayedAiSources.find((item) => item.id === normalizedSourceId)
      || getAiSourceById(normalizedSourceId)
      || (isOfficialAutoSourceId(normalizedSourceId) ? officialAiSource : null)
      || null;
    if (!source) return;

    const isOfficialSource = isOfficialManagedSource(source);
    const routeSourceId = isOfficialSource ? OFFICIAL_AUTO_SOURCE_ID : source.id;
    const nextModel = pickBestModelForSource(source, aiModelRoutes.chat.model || formData.model_name, 'chat');
    updateAiModelRoute('chat', {
      mode: isOfficialSource ? 'official' : 'custom',
      sourceId: routeSourceId,
      model: nextModel,
    });
    ensureDisplayedAiSourcePersisted(source.id);
    setActiveAiSourceId(source.id);
    setAiSourceExpandState((prev) => ({ ...prev, [source.id]: true }));
    setFormData((prev) => ({
      ...prev,
      model_name: nextModel,
      model_name_wander: nextModel,
      model_name_chatroom: nextModel,
      model_name_knowledge: nextModel,
      model_name_redclaw: nextModel,
    }));
  }, [
    aiModelRoutes.chat.model,
    displayedAiSources,
    formData.model_name,
    getAiSourceById,
    isOfficialManagedSource,
    officialAiSource,
    ensureDisplayedAiSourcePersisted,
    pickBestModelForSource,
    updateAiModelRoute,
  ]);

  const applyRouteModel = useCallback((scope: AiModelRouteScope, modelId: string) => {
    const normalizedModel = String(modelId || '').trim();
    updateAiModelRoute(scope, { model: normalizedModel });
    if (scope === 'chat') {
      setFormData((prev) => ({
        ...prev,
        model_name: normalizedModel,
        model_name_wander: normalizedModel,
        model_name_chatroom: normalizedModel,
        model_name_knowledge: normalizedModel,
        model_name_redclaw: normalizedModel,
      }));
    } else if (scope === 'wander') {
      setFormData((prev) => ({ ...prev, model_name_wander: normalizedModel }));
    } else if (scope === 'team') {
      setFormData((prev) => ({ ...prev, model_name_chatroom: normalizedModel }));
    } else if (scope === 'knowledge') {
      setFormData((prev) => ({ ...prev, model_name_knowledge: normalizedModel }));
    } else if (scope === 'redclaw') {
      setFormData((prev) => ({ ...prev, model_name_redclaw: normalizedModel }));
    } else if (scope === 'transcription') {
      setFormData((prev) => ({ ...prev, transcription_model: normalizedModel }));
    } else if (scope === 'embedding') {
      setFormData((prev) => ({ ...prev, embedding_model: normalizedModel }));
    } else if (scope === 'image') {
      setFormData((prev) => ({ ...prev, image_model: normalizedModel }));
    } else if (scope === 'visualIndex') {
      setFormData((prev) => ({ ...prev, visual_index_model: normalizedModel }));
    } else if (scope === 'videoAnalysis') {
      setFormData((prev) => ({ ...prev, video_analysis_model: normalizedModel }));
    } else if (scope === 'voiceTts') {
      const nextCloneModel = cloneModelForVoiceTtsModel(normalizedModel, formData.voice_clone_model || DEFAULT_VOICE_CLONE_MODEL);
      updateAiModelRoute('voiceClone', { model: nextCloneModel });
      setFormData((prev) => ({ ...prev, voice_tts_model: normalizedModel, tts_model: normalizedModel, voice_clone_model: nextCloneModel }));
    } else if (scope === 'voiceClone') {
      setFormData((prev) => ({ ...prev, voice_clone_model: normalizedModel }));
    }
  }, [formData.voice_clone_model, updateAiModelRoute]);

  const setAiModelFormField = useCallback((field: string, value: string) => {
    markAiModelSettingsDraftDirty();
    setFormData((prev) => ({
      ...prev,
      [field]: value,
    }));
  }, [markAiModelSettingsDraftDirty]);

  // 转写地址 ↔ 模型名**绑定**：
  // - 地址是 ElevenLabs → 模型必须是 scribe_*（否则 400 unsupported_model），
  //   默认给 scribe_v2（官方推荐）。这同时修掉「地址 ElevenLabs + 模型
  //   qwen3-asr-flash-filetrans」这种历史错配（模型名是百炼/中转站的命名）。
  // - 地址不是 ElevenLabs → scribe_* 一定是错的（那是 ElevenLabs 专用模型名），
  //   清空让用户填该服务自己的模型名。
  useEffect(() => {
    const endpoint = String(formData.transcription_endpoint || '').trim().toLowerCase();
    const currentModel = String(formData.transcription_model || '').trim();
    const isElevenLabs = endpoint.includes('elevenlabs.io');
    const currentIsScribe = ELEVENLABS_TRANSCRIPTION_MODELS.some(
      (item) => item.id.toLowerCase() === currentModel.toLowerCase(),
    );
    if (isElevenLabs && !currentIsScribe) {
      setAiModelFormField('transcription_model', 'scribe_v2');
    } else if (!isElevenLabs && currentIsScribe) {
      setAiModelFormField('transcription_model', '');
    }
  }, [formData.transcription_endpoint, formData.transcription_model, setAiModelFormField]);

  const handleRemoveSourceModel = (sourceId: string, modelId: string) => {
    const normalizedModel = String(modelId || '').trim();
    if (!normalizedModel) return;
    updateAiSource(sourceId, (source) => {
      const nextModels = normalizeSourceModels((source.models || []).filter((item) => item !== normalizedModel));
      const nextModelsMeta = normalizeAiModelDescriptors((source.modelsMeta || []).filter((item) => String(item?.id || '').trim() !== normalizedModel));
      const fallbackModel = source.model === normalizedModel ? (nextModels[0] || '') : source.model;
      return {
        ...source,
        models: nextModels,
        modelsMeta: nextModelsMeta,
        model: fallbackModel,
      };
    });
  };

  const handleAddSourceModel = (sourceId: string) => {
    const draft = String(sourceModelDrafts[sourceId] || '').trim();
    if (!draft) return;
    const selectedCapability = sourceModelCapabilityDrafts[sourceId] || 'chat';
    updateAiSource(sourceId, (source) => {
      const nextModels = normalizeSourceModels([...(source.models || []), draft]);
      return {
        ...source,
        models: nextModels,
        modelsMeta: normalizeAiModelDescriptors([
          ...(source.modelsMeta || []),
          ...nextModels.map((id) => ({ id })),
          { id: draft, capabilities: [selectedCapability] },
        ]),
        model: source.model || draft,
      };
    });
    setSourceModelDrafts((prev) => ({ ...prev, [sourceId]: '' }));
    setSourceModelCapabilityDrafts((prev) => ({ ...prev, [sourceId]: 'chat' }));
    setAiSourceModelExpandState((prev) => ({ ...prev, [sourceId]: true }));
    setAddModelModalSourceId('');
  };

  const closeAddModelModal = useCallback(() => {
    setAddModelModalSourceId('');
  }, []);

  const openAddModelModal = (source: AiSourceConfig) => {
    setAddModelModalSourceId(source.id);
    setActiveAiSourceId(source.id);
    setSourceModelCapabilityDrafts((prev) => ({
      ...prev,
      [source.id]: prev[source.id] || 'chat',
    }));
  };

  useEffect(() => {
    if (!activeAiSource) return;
    const baseURL = activeAiSource.baseURL.trim();
    const apiKey = activeAiSource.apiKey.trim();
    const allowEmptyKey = isLocalAiSource(activeAiSource);
    if (!baseURL || (!apiKey && !allowEmptyKey)) {
      setTestStatus('idle');
      setTestMsg('');
    }
  }, [
    activeTab,
    activeAiSource?.id,
    activeAiSource?.baseURL,
    activeAiSource?.apiKey,
    activeAiSource?.presetId,
    activeAiSource?.protocol,
    isLocalAiSource,
  ]);

  useEffect(() => {
    if (!isDashscopeImageTemplate) return;
    setFormData((prev) => {
      const current = String(prev.image_model || '').trim();
      if (current === DASHSCOPE_LOCKED_IMAGE_MODEL) return prev;
      return { ...prev, image_model: DASHSCOPE_LOCKED_IMAGE_MODEL };
    });
  }, [activeTab, isDashscopeImageTemplate]);

  const routeModelOptions = useCallback((scope: AiModelRouteScope, source: AiSourceConfig | null): AiModelDescriptor[] => {
    if (!source) return [];
    const models = getSourceModelList(source);
    if (scope === 'transcription') return filterAiModelsByCapability(models, 'transcription');
    if (scope === 'embedding') return filterAiModelsByCapability(models, 'embedding');
    if (scope === 'image') return filterAiModelsByCapability(models, 'image');
    if (scope === 'visualIndex') return filterVisualIndexModels(models);
    if (scope === 'videoAnalysis') return filterVideoAnalysisModels(models);
    if (scope === 'voiceTts') {
      const ttsModels = filterAiModelsByCapability(models, 'tts');
      return ttsModels.length > 0 ? ttsModels : filterAiModelsByCapability(models, 'audio');
    }
    if (scope === 'voiceClone') {
      const cloneModels = filterAiModelsByCapability(models, 'voice_clone');
      return cloneModels.length > 0 ? cloneModels : filterAiModelsByCapability(models, 'audio');
    }
    return filterAiModelsByCapability(models, 'chat');
  }, [filterVideoAnalysisModels, filterVisualIndexModels, getSourceModelList]);

  const currentRouteModelValue = useCallback((scope: AiModelRouteScope): string => {
    if (scope === 'chat') return String(aiModelRoutes.chat.model || formData.model_name || '').trim();
    if (scope === 'wander') return String(aiModelRoutes.wander.model || formData.model_name_wander || '').trim();
    if (scope === 'team') return String(aiModelRoutes.team.model || formData.model_name_chatroom || '').trim();
    if (scope === 'knowledge') return String(aiModelRoutes.knowledge.model || formData.model_name_knowledge || '').trim();
    if (scope === 'redclaw') return String(aiModelRoutes.redclaw.model || formData.model_name_redclaw || '').trim();
    if (scope === 'transcription') return String(aiModelRoutes.transcription.model || formData.transcription_model || '').trim();
    if (scope === 'embedding') return String(aiModelRoutes.embedding.model || formData.embedding_model || '').trim();
    if (scope === 'image') return String(aiModelRoutes.image.model || formData.image_model || '').trim();
    if (scope === 'visualIndex') return String(aiModelRoutes.visualIndex.model || formData.visual_index_model || '').trim();
    if (scope === 'videoAnalysis') return String(aiModelRoutes.videoAnalysis.model || formData.video_analysis_model || '').trim();
    if (scope === 'voiceTts') return String(aiModelRoutes.voiceTts.model || formData.voice_tts_model || formData.tts_model || DEFAULT_VOICE_TTS_MODEL).trim();
    if (scope === 'voiceClone') return String(aiModelRoutes.voiceClone.model || formData.voice_clone_model || DEFAULT_VOICE_CLONE_MODEL).trim();
    return '';
  }, [
    aiModelRoutes,
    formData.embedding_model,
    formData.image_model,
    formData.model_name,
    formData.model_name_chatroom,
    formData.model_name_knowledge,
    formData.model_name_redclaw,
    formData.model_name_wander,
    formData.transcription_model,
    formData.tts_model,
    formData.video_analysis_model,
    formData.visual_index_model,
    formData.voice_clone_model,
    formData.voice_tts_model,
  ]);

  const fallbackOfficialRouteModel = useCallback((scope: AiModelRouteScope, preferredModel = ''): string => {
    const pickFirstOfficialModel = (capability: ModelCapability = 'chat') => {
      const sourceModels = getSourceModelList(officialAiSource);
      const matchingModels = filterAiModelsByCapability(sourceModels, capability);
      return String(matchingModels[0]?.id || sourceModels[0]?.id || '').trim();
    };
    const pickFirstOfficialVisualModel = () => {
      const sourceModels = getSourceModelList(officialAiSource);
      const visualModels = filterVisualIndexModels(sourceModels);
      return String(visualModels[0]?.id || sourceModels[0]?.id || '').trim();
    };
    const pickFirstOfficialVideoModel = () => {
      const sourceModels = getSourceModelList(officialAiSource);
      const videoModels = filterVideoAnalysisModels(sourceModels);
      if (preferredModel && videoModels.some((item) => item.id === preferredModel)) {
        return preferredModel;
      }
      return String(videoModels[0]?.id || '').trim();
    };
    if (scope === 'voiceTts') return pickBestModelForSource(officialAiSource, preferredModel, 'tts') || pickBestModelForSource(officialAiSource, preferredModel, 'audio') || pickFirstOfficialModel('tts') || DEFAULT_VOICE_TTS_MODEL;
    if (scope === 'voiceClone') return pickBestModelForSource(officialAiSource, preferredModel, 'voice_clone') || pickBestModelForSource(officialAiSource, preferredModel, 'audio') || pickFirstOfficialModel('voice_clone') || DEFAULT_VOICE_CLONE_MODEL;
    if (scope === 'transcription') return pickBestModelForSource(officialAiSource, preferredModel, 'transcription') || pickFirstOfficialModel('transcription');
    if (scope === 'embedding') return pickBestModelForSource(officialAiSource, preferredModel, 'embedding') || pickFirstOfficialModel('embedding') || 'text-embedding-3-small';
    if (scope === 'image') return pickBestModelForSource(officialAiSource, preferredModel, 'image') || pickFirstOfficialModel('image');
    if (scope === 'visualIndex') return pickBestVisualIndexModelForSource(officialAiSource, preferredModel) || pickFirstOfficialVisualModel();
    if (scope === 'videoAnalysis') return pickFirstOfficialVideoModel();
    if (scope === 'chat') return pickBestModelForSource(officialAiSource, preferredModel, 'chat') || pickFirstOfficialModel('chat');
    return pickBestModelForSource(officialAiSource, preferredModel, 'chat') || pickFirstOfficialModel('chat');
  }, [
    filterVideoAnalysisModels,
    filterVisualIndexModels,
    getSourceModelList,
    officialAiSource,
    pickBestModelForSource,
    pickBestVisualIndexModelForSource,
  ]);

  const effectiveRouteModelValue = useCallback((scope: AiModelRouteScope): string => {
    if (aiModelRoutes[scope].mode === 'official') {
      const officialModel = String(aiModelRoutes[scope].model || '').trim();
      const legacyModel = currentRouteModelValue(scope);
      const preferredModel = officialModel || legacyModel;
      if (scope === 'image') {
        return pickCapabilityModelForSource(officialAiSource, preferredModel, 'image') || fallbackOfficialRouteModel(scope);
      }
      return fallbackOfficialRouteModel(scope, preferredModel);
    }
    return currentRouteModelValue(scope);
  }, [aiModelRoutes, currentRouteModelValue, fallbackOfficialRouteModel, officialAiSource, pickCapabilityModelForSource]);

  const routeModelSelectOptions = useCallback((models: AiModelDescriptor[], value: string) => {
    const normalizedValue = String(value || '').trim();
    const options = models.map((model) => ({
      id: model.id,
      label: model.id,
    }));
    if (normalizedValue && !options.some((item) => item.id === normalizedValue)) {
      options.unshift({ id: normalizedValue, label: normalizedValue });
    }
    return options;
  }, []);

  const renderModelSettingsTableRow = ({
    label,
    sourceValue,
    sources,
    onSourceChange,
    modelValue,
    onModelChange,
    modelOptions,
    modelDisabled,
    modelPlaceholder,
  }: {
    label: string;
    sourceValue: string;
    sources: AiSourceConfig[];
    onSourceChange: (sourceId: string) => void;
    modelValue: string;
    onModelChange: (modelId: string) => void;
    modelOptions: AiModelOption[];
    modelDisabled?: boolean;
    modelPlaceholder: string;
  }) => (
    <div className="grid grid-cols-1 gap-2 border-t border-border/70 p-3 md:grid-cols-[132px_minmax(0,1fr)_minmax(0,1fr)] md:items-center">
      <div className="text-sm font-medium text-text-primary">{label}</div>
      <AiSourceSelect
        value={sourceValue}
        sources={sources}
        onChange={onSourceChange}
        className="w-full"
      />
      <AiModelSelect
        value={modelValue}
        onChange={onModelChange}
        options={modelOptions}
        disabled={modelDisabled}
        placeholder={modelPlaceholder}
        className="w-full"
      />
    </div>
  );
  return {
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
  };
}
