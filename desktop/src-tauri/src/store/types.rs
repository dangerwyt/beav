use crate::{cli_runtime, runtime::*};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpaceRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct SubjectAttribute {
    pub(crate) key: String,
    pub(crate) value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubjectSku {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) attributes: Vec<SubjectAttribute>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubjectCategory {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubjectRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) category_id: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) attributes: Vec<SubjectAttribute>,
    pub(crate) image_paths: Vec<String>,
    pub(crate) voice_path: Option<String>,
    pub(crate) video_path: Option<String>,
    pub(crate) voice_script: Option<String>,
    pub(crate) voice: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) brand_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) skus: Vec<SubjectSku>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) absolute_image_paths: Vec<String>,
    pub(crate) preview_urls: Vec<String>,
    pub(crate) primary_preview_url: Option<String>,
    pub(crate) absolute_voice_path: Option<String>,
    pub(crate) voice_preview_url: Option<String>,
    pub(crate) absolute_video_path: Option<String>,
    pub(crate) video_preview_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatSessionRecord {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) metadata: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) deleted_at: Option<i64>,
    #[serde(default)]
    pub(crate) starred: bool,
    #[serde(default)]
    pub(crate) archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) archived_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatMessageRecord {
    pub(crate) id: String,
    pub(crate) session_id: String,
    pub(crate) role: String,
    pub(crate) content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) display_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) attachment: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) metadata: Option<Value>,
    pub(crate) created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatSessionContextRecord {
    pub(crate) session_id: String,
    pub(crate) summary: String,
    pub(crate) summary_source: String,
    pub(crate) total_message_count: i64,
    pub(crate) compacted_message_count: i64,
    pub(crate) tail_message_count: i64,
    pub(crate) compact_rounds: i64,
    pub(crate) summary_chars: i64,
    pub(crate) estimated_total_tokens: i64,
    pub(crate) first_user_message: Option<String>,
    pub(crate) last_user_message: Option<String>,
    pub(crate) last_assistant_message: Option<String>,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManuscriptWriteProposalRecord {
    pub(crate) id: String,
    pub(crate) file_path: String,
    pub(crate) session_id: Option<String>,
    pub(crate) tool_call_id: Option<String>,
    pub(crate) draft_type: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) metadata: Option<Value>,
    pub(crate) base_content: String,
    pub(crate) proposed_content: String,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdvisorRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) avatar: String,
    pub(crate) personality: String,
    pub(crate) system_prompt: String,
    pub(crate) knowledge_language: Option<String>,
    pub(crate) knowledge_files: Vec<String>,
    pub(crate) youtube_channel: Option<Value>,
    pub(crate) member_skill_ref: Option<String>,
    pub(crate) member_skill_status: Option<String>,
    pub(crate) member_skill_version: Option<String>,
    pub(crate) member_skill_last_distilled_at: Option<String>,
    pub(crate) member_skill_last_error: Option<String>,
    pub(crate) member_skill_candidate_version: Option<String>,
    pub(crate) member_skill_candidate_path: Option<String>,
    pub(crate) member_skill_candidate_created_at: Option<String>,
    pub(crate) member_skill_candidate_source_event: Option<String>,
    pub(crate) detected_knowledge_language: Option<String>,
    pub(crate) language_detection_status: Option<String>,
    pub(crate) language_confidence: Option<f64>,
    pub(crate) redclaw_visible: Option<bool>,
    pub(crate) redclaw_order: Option<i64>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AdvisorVideoRecord {
    pub(crate) id: String,
    pub(crate) advisor_id: String,
    pub(crate) title: String,
    pub(crate) published_at: String,
    pub(crate) status: String,
    pub(crate) retry_count: i64,
    pub(crate) error_message: Option<String>,
    pub(crate) subtitle_file: Option<String>,
    pub(crate) video_url: Option<String>,
    pub(crate) channel_id: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatRoomRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) advisor_ids: Vec<String>,
    pub(crate) created_at: String,
    pub(crate) is_system: Option<bool>,
    pub(crate) system_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatRoomMessageRecord {
    pub(crate) id: String,
    pub(crate) room_id: String,
    pub(crate) role: String,
    pub(crate) advisor_id: Option<String>,
    pub(crate) advisor_name: Option<String>,
    pub(crate) advisor_avatar: Option<String>,
    pub(crate) content: String,
    pub(crate) timestamp: String,
    pub(crate) is_streaming: Option<bool>,
    pub(crate) phase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WechatOfficialBindingRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) app_id: String,
    pub(crate) secret: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) verified_at: Option<String>,
    pub(crate) is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmbeddingCacheRecord {
    pub(crate) file_path: String,
    pub(crate) content_hash: String,
    pub(crate) embedding: Vec<f64>,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SimilarityCacheRecord {
    pub(crate) manuscript_id: String,
    pub(crate) content_hash: String,
    pub(crate) knowledge_version: String,
    pub(crate) sorted_ids: Vec<String>,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WanderHistoryRecord {
    pub(crate) id: String,
    pub(crate) items: String,
    pub(crate) result: String,
    pub(crate) created_at: i64,
    #[serde(default)]
    pub(crate) status: Option<String>,
    #[serde(default)]
    pub(crate) abandoned_at: Option<i64>,
}

fn default_topic_center_status() -> String {
    "active".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TopicCenterRecord {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) content_direction: String,
    #[serde(default)]
    pub(crate) direction_frame: Value,
    pub(crate) method: String,
    pub(crate) source_evidence: String,
    #[serde(default)]
    pub(crate) source_refs: Vec<Value>,
    pub(crate) target_reader: String,
    pub(crate) user_problem: String,
    pub(crate) content_value: String,
    pub(crate) fit_reason: String,
    #[serde(default)]
    pub(crate) score: Option<f64>,
    #[serde(default = "default_topic_center_status")]
    pub(crate) status: String,
    pub(crate) created_by: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    #[serde(default)]
    pub(crate) abandoned_at: Option<i64>,
    #[serde(default)]
    pub(crate) raw_result: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct YoutubeVideoRecord {
    pub(crate) id: String,
    pub(crate) video_id: String,
    pub(crate) video_url: String,
    pub(crate) title: String,
    pub(crate) original_title: Option<String>,
    pub(crate) description: String,
    pub(crate) summary: Option<String>,
    pub(crate) thumbnail_url: String,
    pub(crate) has_subtitle: bool,
    pub(crate) subtitle_content: Option<String>,
    pub(crate) subtitle_error: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) created_at: String,
    pub(crate) folder_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AcpGatewayStateRecord {
    pub(crate) enabled: bool,
    pub(crate) require_token: bool,
    pub(crate) local_only: bool,
    pub(crate) endpoint_path: String,
    pub(crate) manifest_path: String,
    pub(crate) guide_path: String,
    pub(crate) default_runtime_mode: String,
    pub(crate) default_client_label: String,
    pub(crate) last_error: Option<String>,
    pub(crate) active_run_count: i64,
}

impl Default for AcpGatewayStateRecord {
    fn default() -> Self {
        Self {
            enabled: false,
            require_token: false,
            local_only: true,
            endpoint_path: "/acp/v1".to_string(),
            manifest_path: "/acp/v1/manifest".to_string(),
            guide_path: "/acp/v1/guide".to_string(),
            default_runtime_mode: "default".to_string(),
            default_client_label: "External Agent".to_string(),
            last_error: None,
            active_run_count: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AcpClientRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) token_hash: Option<String>,
    pub(crate) token_preview: Option<String>,
    pub(crate) allowed_scopes: Vec<String>,
    pub(crate) disabled: bool,
    pub(crate) metadata: Option<Value>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) last_seen_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AcpSessionRecord {
    pub(crate) id: String,
    pub(crate) external_session_id: Option<String>,
    pub(crate) external_client_id: Option<String>,
    pub(crate) external_client_name: Option<String>,
    pub(crate) external_client_kind: Option<String>,
    pub(crate) source_label: String,
    pub(crate) collab_session_id: String,
    pub(crate) chat_session_id: String,
    pub(crate) project_ref: Option<Value>,
    pub(crate) title: String,
    pub(crate) objective: String,
    pub(crate) status: String,
    pub(crate) metadata: Option<Value>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) last_message_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AcpRunRecord {
    pub(crate) id: String,
    pub(crate) session_id: String,
    pub(crate) collab_session_id: String,
    pub(crate) chat_session_id: String,
    pub(crate) status: String,
    pub(crate) status_reason: Option<String>,
    pub(crate) input_message_id: Option<String>,
    pub(crate) output_message_id: Option<String>,
    pub(crate) prompt: String,
    pub(crate) response: Option<String>,
    pub(crate) artifact_ids: Vec<String>,
    pub(crate) metadata: Option<Value>,
    pub(crate) cancel_requested: bool,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) started_at: Option<i64>,
    pub(crate) completed_at: Option<i64>,
    pub(crate) last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AcpMessageRecord {
    pub(crate) id: String,
    pub(crate) session_id: String,
    pub(crate) run_id: Option<String>,
    pub(crate) direction: String,
    pub(crate) role: String,
    pub(crate) sender_kind: String,
    pub(crate) sender_label: String,
    pub(crate) content: String,
    pub(crate) content_type: String,
    pub(crate) attachment_refs: Vec<String>,
    pub(crate) payload: Option<Value>,
    pub(crate) chat_message_id: Option<String>,
    pub(crate) collab_message_id: Option<String>,
    pub(crate) created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AcpArtifactRecord {
    pub(crate) id: String,
    pub(crate) session_id: String,
    pub(crate) run_id: Option<String>,
    pub(crate) kind: String,
    pub(crate) title: String,
    pub(crate) summary: Option<String>,
    pub(crate) refs: Vec<String>,
    pub(crate) payload: Option<Value>,
    pub(crate) created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AcpAuditEventRecord {
    pub(crate) id: String,
    pub(crate) session_id: Option<String>,
    pub(crate) run_id: Option<String>,
    pub(crate) event_type: String,
    pub(crate) status: String,
    pub(crate) message: Option<String>,
    pub(crate) payload: Option<Value>,
    pub(crate) created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AppStore {
    pub(crate) settings: Value,
    pub(crate) spaces: Vec<SpaceRecord>,
    pub(crate) active_space_id: String,
    pub(crate) subjects: Vec<SubjectRecord>,
    pub(crate) categories: Vec<SubjectCategory>,
    pub(crate) advisors: Vec<AdvisorRecord>,
    pub(crate) advisor_videos: Vec<AdvisorVideoRecord>,
    pub(crate) chat_rooms: Vec<ChatRoomRecord>,
    pub(crate) chatroom_messages: Vec<ChatRoomMessageRecord>,
    pub(crate) wechat_official_bindings: Vec<WechatOfficialBindingRecord>,
    pub(crate) embedding_cache: Vec<EmbeddingCacheRecord>,
    pub(crate) similarity_cache: Vec<SimilarityCacheRecord>,
    pub(crate) wander_history: Vec<WanderHistoryRecord>,
    pub(crate) topic_center: Vec<TopicCenterRecord>,
    pub(crate) chat_sessions: Vec<ChatSessionRecord>,
    pub(crate) chat_messages: Vec<ChatMessageRecord>,
    pub(crate) session_context_records: Vec<ChatSessionContextRecord>,
    pub(crate) manuscript_write_proposals: Vec<ManuscriptWriteProposalRecord>,
    pub(crate) youtube_videos: Vec<YoutubeVideoRecord>,
    pub(crate) knowledge_notes: Vec<KnowledgeNoteRecord>,
    pub(crate) knowledge_authors: Vec<KnowledgeAuthorRecord>,
    #[serde(default)]
    pub(crate) competitor_watchlist: Vec<CompetitorWatchRecord>,
    #[serde(default)]
    pub(crate) monitored_posts: Vec<MonitoredPostRecord>,
    #[serde(default)]
    pub(crate) publish_log: Vec<PublishLogRecord>,
    #[serde(default)]
    pub(crate) post_snapshots: Vec<PostSnapshotRecord>,
    pub(crate) document_sources: Vec<DocumentKnowledgeSourceRecord>,
    pub(crate) session_transcript_records: Vec<SessionTranscriptRecord>,
    pub(crate) session_checkpoints: Vec<SessionCheckpointRecord>,
    pub(crate) session_tool_results: Vec<SessionToolResultRecord>,
    pub(crate) runtime_tasks: Vec<RuntimeTaskRecord>,
    pub(crate) runtime_task_traces: Vec<RuntimeTaskTraceRecord>,
    pub(crate) runtime_events: Vec<RuntimeEventRecord>,
    pub(crate) collab_sessions: Vec<CollabSessionRecord>,
    pub(crate) collab_members: Vec<CollabMemberRecord>,
    pub(crate) collab_tasks: Vec<CollabTaskRecord>,
    pub(crate) collab_mailbox_messages: Vec<CollabMailboxMessageRecord>,
    pub(crate) collab_progress_reports: Vec<CollabProgressReportRecord>,
    pub(crate) acp_gateway: AcpGatewayStateRecord,
    pub(crate) acp_clients: Vec<AcpClientRecord>,
    pub(crate) acp_sessions: Vec<AcpSessionRecord>,
    pub(crate) acp_runs: Vec<AcpRunRecord>,
    pub(crate) acp_messages: Vec<AcpMessageRecord>,
    pub(crate) acp_artifacts: Vec<AcpArtifactRecord>,
    pub(crate) acp_audit_events: Vec<AcpAuditEventRecord>,
    pub(crate) review_dockets: Vec<ReviewDocketRecord>,
    pub(crate) review_decisions: Vec<ReviewDecisionRecord>,
    pub(crate) cli_tools: Vec<cli_runtime::CliToolRecord>,
    pub(crate) cli_environments: Vec<cli_runtime::CliEnvironmentRecord>,
    pub(crate) cli_manifests: Vec<cli_runtime::CliToolManifestRecord>,
    pub(crate) cli_executions: Vec<cli_runtime::CliExecutionRecord>,
    pub(crate) cli_escalations: Vec<cli_runtime::CliEscalationRequestRecord>,
    pub(crate) cli_verifications: Vec<cli_runtime::CliVerificationRecord>,
    pub(crate) debug_logs: Vec<String>,
    pub(crate) archive_profiles: Vec<ArchiveProfileRecord>,
    pub(crate) archive_samples: Vec<ArchiveSampleRecord>,
    pub(crate) memories: Vec<UserMemoryRecord>,
    pub(crate) memory_history: Vec<MemoryHistoryRecord>,
    pub(crate) mcp_servers: Vec<McpServerRecord>,
    pub(crate) runtime_hooks: Vec<RuntimeHookRecord>,
    pub(crate) skills: Vec<SkillRecord>,
    pub(crate) assistant_state: AssistantStateRecord,
    pub(crate) redclaw_state: RedclawStateRecord,
    pub(crate) redclaw_job_definitions: Vec<RedclawJobDefinitionRecord>,
    pub(crate) redclaw_job_executions: Vec<RedclawJobExecutionRecord>,
    pub(crate) media_assets: Vec<MediaAssetRecord>,
    pub(crate) cover_assets: Vec<CoverAssetRecord>,
    pub(crate) work_items: Vec<WorkItemRecord>,
    pub(crate) legacy_imported_at: Option<String>,
    pub(crate) legacy_import_source: Option<String>,
    /// 对标监控单次抓取的最大帖子条数（web / CDP 两条路径统一尊重）。
    /// 0 / 缺失视为默认 12；上限 200 防止误填超大值把请求拖死。
    #[serde(default)]
    pub(crate) monitor_scan_limit: i64,
    /// 对标监控「异常值检测」的倍数阈值：某帖当前点赞 ≥ 该作者近窗口点赞中位数的该倍数即标为爆文。
    /// 0 / 缺失视为默认 3.0（对齐竞品「互动≥平时3倍」）。
    #[serde(default)]
    pub(crate) monitor_spike_threshold: f64,
    /// 对标监控「帖子最终产物」的**全局导出根目录**（与工作区解耦）。
    /// 空 = 跟随工作区根；非空且目录存在 = 导出到 `<该目录>/<平台>/<日期>/<作者>/<标题前10字>`。
    #[serde(default)]
    pub(crate) monitor_export_root: String,
    /// 视频「分析成功」（抽帧已落库且转写 `ok`）后是否删除 monitor-videos 里的原片。
    /// 默认关；仅在「存在独立预览文件（与原片不同路径）」时才删，删后播放/再分析改用预览。
    #[serde(default)]
    pub(crate) monitor_delete_original_after_analysis: bool,
    /// 爆文拆解沉淀库：每篇被拆解的对标帖结构化出的方法论要素（角度/结构/开场白/标题句式/钩子）。
    #[serde(default)]
    pub(crate) post_deconstructions: Vec<PostDeconstructionRecord>,
    /// 竞品视频分析库：每篇被下载并分析的对标视频，沉淀字幕 + 抽帧 + 摘要（与拆解库并列）。
    #[serde(default)]
    pub(crate) post_video_analyses: Vec<PostVideoAnalysisRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct AssistantStateRecord {
    pub(crate) enabled: bool,
    pub(crate) auto_start: bool,
    pub(crate) keep_alive_when_no_window: bool,
    pub(crate) host: String,
    pub(crate) port: i64,
    pub(crate) listening: bool,
    pub(crate) lock_state: String,
    pub(crate) blocked_by: Option<String>,
    pub(crate) last_error: Option<String>,
    pub(crate) active_task_count: i64,
    pub(crate) queued_peer_count: i64,
    pub(crate) in_flight_keys: Vec<String>,
    pub(crate) feishu: Value,
    pub(crate) relay: Value,
    pub(crate) weixin: Value,
    pub(crate) knowledge_api: Value,
}

impl Default for AssistantStateRecord {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_start: true,
            keep_alive_when_no_window: true,
            host: "127.0.0.1".to_string(),
            port: 31937,
            listening: false,
            lock_state: "passive".to_string(),
            blocked_by: None,
            last_error: Some("Assistant daemon is idle.".to_string()),
            active_task_count: 0,
            queued_peer_count: 0,
            in_flight_keys: Vec::new(),
            feishu: json!({
                "enabled": false,
                "receiveMode": "webhook",
                "endpointPath": "/hooks/feishu/events",
                "replyUsingChatId": true,
                "webhookUrl": "",
                "websocketRunning": false
            }),
            relay: json!({
                "enabled": true,
                "endpointPath": "/hooks/channel/relay",
                "authToken": "",
                "webhookUrl": ""
            }),
            weixin: json!({
                "enabled": false,
                "endpointPath": "/hooks/weixin/relay",
                "authToken": "",
                "accountId": "",
                "autoStartSidecar": false,
                "cursorFile": "",
                "sidecarCommand": "",
                "sidecarArgs": [],
                "sidecarCwd": "",
                "sidecarEnv": {},
                "webhookUrl": "",
                "sidecarRunning": false,
                "connected": false,
                "stateDir": "",
                "availableAccountIds": []
            }),
            knowledge_api: json!({
                "endpointPath": "/api/knowledge",
                "webhookUrl": ""
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArchiveProfileRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) platform: Option<String>,
    pub(crate) goal: Option<String>,
    pub(crate) domain: Option<String>,
    pub(crate) audience: Option<String>,
    pub(crate) tone_tags: Vec<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArchiveSampleRecord {
    pub(crate) id: String,
    pub(crate) profile_id: String,
    pub(crate) title: Option<String>,
    pub(crate) content: Option<String>,
    pub(crate) excerpt: Option<String>,
    pub(crate) tags: Vec<String>,
    pub(crate) images: Vec<String>,
    pub(crate) platform: Option<String>,
    pub(crate) source_url: Option<String>,
    pub(crate) sample_date: Option<String>,
    pub(crate) is_featured: i64,
    pub(crate) created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UserMemoryRecord {
    pub(crate) id: String,
    pub(crate) content: String,
    pub(crate) r#type: String,
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) entities: Vec<String>,
    #[serde(default)]
    pub(crate) scope: Option<String>,
    #[serde(default)]
    pub(crate) space_id: Option<String>,
    #[serde(default)]
    pub(crate) project_id: Option<String>,
    #[serde(default)]
    pub(crate) session_id: Option<String>,
    #[serde(default)]
    pub(crate) source: Option<Value>,
    #[serde(default)]
    pub(crate) confidence: Option<f64>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: Option<i64>,
    pub(crate) last_accessed: Option<i64>,
    pub(crate) status: Option<String>,
    pub(crate) archived_at: Option<i64>,
    pub(crate) archive_reason: Option<String>,
    pub(crate) origin_id: Option<String>,
    pub(crate) canonical_key: Option<String>,
    pub(crate) revision: Option<i64>,
    pub(crate) last_conflict_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MemoryHistoryRecord {
    pub(crate) id: String,
    pub(crate) memory_id: String,
    pub(crate) origin_id: String,
    pub(crate) action: String,
    pub(crate) reason: Option<String>,
    pub(crate) timestamp: i64,
    pub(crate) before: Option<Value>,
    pub(crate) after: Option<Value>,
    pub(crate) archived_memory_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KnowledgeNoteStatsRecord {
    pub(crate) likes: i64,
    pub(crate) collects: Option<i64>,
    pub(crate) comments: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KnowledgeNoteRecord {
    pub(crate) id: String,
    pub(crate) r#type: Option<String>,
    pub(crate) source_domain: Option<String>,
    pub(crate) source_link: Option<String>,
    pub(crate) source_url: Option<String>,
    pub(crate) title: String,
    pub(crate) author: String,
    pub(crate) author_id: Option<String>,
    pub(crate) author_url: Option<String>,
    pub(crate) author_avatar_url: Option<String>,
    pub(crate) author_description: Option<String>,
    pub(crate) content: String,
    pub(crate) excerpt: Option<String>,
    pub(crate) site_name: Option<String>,
    pub(crate) capture_kind: Option<String>,
    pub(crate) metadata: Option<Value>,
    pub(crate) html_file: Option<String>,
    pub(crate) html_file_url: Option<String>,
    pub(crate) images: Vec<String>,
    pub(crate) tags: Option<Vec<String>>,
    pub(crate) cover: Option<String>,
    pub(crate) video: Option<String>,
    pub(crate) video_url: Option<String>,
    pub(crate) transcript: Option<String>,
    pub(crate) transcription_status: Option<String>,
    pub(crate) stats: KnowledgeNoteStatsRecord,
    pub(crate) created_at: String,
    pub(crate) folder_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KnowledgeAuthorRecord {
    pub(crate) id: String,
    pub(crate) r#type: String,
    pub(crate) name: String,
    pub(crate) platform: String,
    pub(crate) platform_user_id: Option<String>,
    pub(crate) handle: Option<String>,
    pub(crate) profile_url: Option<String>,
    pub(crate) avatar_url: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) source_domain: Option<String>,
    pub(crate) linked_note_ids: Vec<String>,
    pub(crate) note_count: i64,
    pub(crate) first_seen_at: String,
    pub(crate) latest_note_at: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) folder_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct CompetitorWatchRecord {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) platform: String,
    pub(crate) profile_url: Option<String>,
    pub(crate) handle: Option<String>,
    pub(crate) note: Option<String>,
    pub(crate) enabled: bool,
    pub(crate) interval_hours: i64,
    pub(crate) last_fetch_at: Option<String>,
    /// 该博主的**深层历史是否已补过**（时间戳字符串）。
    ///
    /// 日常抓取只取主页最新的 N 条（增量窗口），更早的历史帖永远进不来 ——
    /// 首次接入时尤其明显（只拿到最近 30 条）。补历史 = 用大 limit 深翻主页，
    /// 把能翻到的历史帖一次性落库；补过一次就打上此标记，不再每晚重复深翻。
    /// 老数据没有这个字段 → None → 视为"还没补过"，会被自动补一次。
    #[serde(default)]
    pub(crate) history_backfilled_at: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct MonitoredPostRecord {
    pub(crate) id: String,
    pub(crate) watch_id: Option<String>,
    pub(crate) platform: String,
    pub(crate) author: String,
    pub(crate) author_url: Option<String>,
    pub(crate) post_url: String,
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) media_type: String, // "image" | "video"
    #[serde(default)]
    pub(crate) cover_url: Option<String>,
    /// 详情页抓到的图片列表（含封面）。落库后二次打开（本地缓存路径）才有缩略图可显示，
    /// 否则只有当次抓取的临时返回、缓存打开会丢图。
    #[serde(default)]
    pub(crate) images: Vec<String>,
    pub(crate) content: String,
    /// 详情页是否成功抓取过（图/标题抓到即算，与正文有无无关）。
    /// 纯图/无正文笔记 content 永远为空，不能拿它当缓存钥匙，否则每次打开都重抓。
    #[serde(default)]
    pub(crate) detail_fetched: bool,
    pub(crate) excerpt: Option<String>,
    pub(crate) published_at: Option<String>,
    pub(crate) likes: i64,
    pub(crate) comments: i64,
    pub(crate) collects: i64,
    pub(crate) prev_likes: i64,
    pub(crate) prev_comments: i64,
    pub(crate) is_new: bool,
    pub(crate) spike: bool,
    pub(crate) spike_ratio: f64,
    pub(crate) captured_via: String,
    pub(crate) topic_injected: bool,
    /// 该动态是否已被「爆文拆解」沉淀过方法论（卡片/详情可据此显示状态）。
    #[serde(default)]
    pub(crate) deconstructed: bool,
    /// 视频抓取闭环字段：本地视频文件路径（下载成功后填）。
    #[serde(default)]
    pub(crate) video_local_path: Option<String>,
    /// 轻量代理视频路径（720p/H.264/faststart，WebView 内直接播放用）。
    /// 原片常是 4K/HEVC/数百 MB —— HEVC 无解码器、asset 协议无 Range，原片在 `<video>` 里播不了。
    #[serde(default)]
    pub(crate) video_preview_path: Option<String>,
    /// 下载后的视频文件大小（字节）。
    #[serde(default)]
    pub(crate) video_size: Option<i64>,
    /// 视频抓取状态：none | pending | downloading | done | failed。
    #[serde(default = "default_video_status")]
    pub(crate) video_status: String,
    /// 视频抓取完成时间戳（ms）。
    #[serde(default)]
    pub(crate) video_downloaded_at: Option<i64>,
    /// 视频抓取失败原因（video_status=failed 时填）。
    #[serde(default)]
    pub(crate) video_error: Option<String>,
    /// 本地视频是否含音轨（`None` = 未探测）。抖音 DASH 分离流常下到无声 mp4，
    /// 前端据此提示「该视频无口播音轨，转写不可用」而不是让用户以为是转写服务坏了。
    #[serde(default)]
    pub(crate) video_has_audio: Option<bool>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublishLogRecord {
    pub(crate) id: String,
    pub(crate) platform: String,
    #[serde(default)]
    pub(crate) post_url: Option<String>,
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) published_at: Option<String>,
    #[serde(default = "default_publish_status")]
    pub(crate) status: String, // "planned" | "published" | "failed"
    #[serde(default)]
    pub(crate) views: i64,
    #[serde(default)]
    pub(crate) likes: i64,
    #[serde(default)]
    pub(crate) comments: i64,
    #[serde(default)]
    pub(crate) collects: i64,
    #[serde(default)]
    pub(crate) linked_topic_id: Option<String>,
    #[serde(default)]
    pub(crate) notes: Option<String>,
    pub(crate) created_at: i64,
    #[serde(default)]
    pub(crate) updated_at: i64,
}

fn default_publish_status() -> String {
    "planned".to_string()
}

fn default_video_status() -> String {
    "none".to_string()
}

/// 竞品视频分析沉淀：一篇对标视频被下载到本地后，抽帧 + 语音转写 + LLM 摘要的结构化结果。
///
/// 与 `PostDeconstructionRecord`（纯文本拆解）互补——这里处理「视频本体内容」：
/// 字幕（ASR）/ 关键帧（视觉）/ 摘要（LLM）。`local_path` 指向 `monitor_videos/` 下的视频文件。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct PostVideoAnalysisRecord {
    pub(crate) id: String,
    pub(crate) post_id: String,
    pub(crate) platform: String,
    pub(crate) author: String,
    pub(crate) post_title: String,
    pub(crate) post_url: String,
    /// 本地视频文件路径（下载后填）。
    pub(crate) local_path: String,
    /// 视频时长（秒，ffprobe 探测）。
    pub(crate) duration_secs: Option<f64>,
    /// 语音转写全文（ASR）。
    pub(crate) transcript: String,
    /// 转写状态：`ok` / `no-audio`（源视频无音轨）/ `not-configured`（未配 ASR 服务）/
    /// `failed`（调用失败）/ 空串（尚未尝试）。前端据此显示**真实原因**而不是笼统提示。
    pub(crate) transcript_status: String,
    /// 转写失败的具体错误（`transcript_status != ok` 时填），截断到 800 字供 UI 展示。
    pub(crate) transcript_error: Option<String>,
    /// 带时间轴的字幕原文（SRT 文本），供前端做「时间轴」展示。
    /// 与 `transcript`（干净全文）分离存储，避免时间戳污染 LLM / 详情页纯文本。
    pub(crate) transcript_srt: String,
    /// 抽取的关键帧图片路径（相对/绝对，存于 `monitor_videos/<post_id>/frames/`）。
    pub(crate) frames: Vec<String>,
    /// LLM 生成的视频内容摘要（选题角度 / 结构 / 钩子），可空（未跑 LLM 时）。
    pub(crate) summary: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

/// 单篇帖子的**互动快照**：每次抓到真实互动数（CDP 列表 / 详情页）就落一条。
///
/// 为什么需要它：`monitored_posts` 按 post_url 去重、原地更新，只有「当前值 + 单次 prev 差」，
/// 没有历史时间序列——直接拿它做「30 天互动走势折线」是不可能的。本表按抓取时刻存点，
/// 复盘模块再按天分桶聚合出趋势。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct PostSnapshotRecord {
    pub(crate) id: String,
    pub(crate) post_id: String,
    pub(crate) platform: String,
    pub(crate) author: String,
    pub(crate) likes: i64,
    pub(crate) comments: i64,
    pub(crate) collects: i64,
    pub(crate) captured_at: i64, // 毫秒时间戳（与 now_i64 一致）
}

/// 爆文拆解沉淀：把一篇对标爆文交给 LLM 拆成可复用的方法论要素，结构化落库。
///
/// 与 `topic_center`（生成新选题）语义不同——这里是「解剖别人已验证的爆款」，
/// 拆出选题角度 / 文案结构 / 开场白 / 标题句式 / 钩子，供自己写稿时直接套用。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct PostDeconstructionRecord {
    pub(crate) id: String,
    pub(crate) post_id: String,
    pub(crate) platform: String,
    pub(crate) author: String,
    pub(crate) post_title: String,
    pub(crate) post_url: String,
    /// 选题角度：这篇为什么值得写（情绪/痛点/反常识/争议）。
    pub(crate) topic_angle: String,
    /// 文案结构：开头如何钩人、中间如何递进、结尾如何转化。
    pub(crate) copy_structure: String,
    /// 开场白 / 前 3 秒钩子。
    pub(crate) opening: String,
    /// 标题句式（可多条）。
    pub(crate) title_patterns: Vec<String>,
    /// 金句 / 钩子（可多条）。
    pub(crate) hooks: Vec<String>,
    /// 可复用的方法论点。
    pub(crate) reusable_takeaway: String,
    /// 拆解所用的原文（落库备查）。
    pub(crate) raw_content: String,
    pub(crate) source_evidence: String,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DocumentKnowledgeSourceRecord {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) root_path: String,
    pub(crate) locked: bool,
    pub(crate) indexing: bool,
    pub(crate) index_error: Option<String>,
    pub(crate) file_count: i64,
    pub(crate) sample_files: Vec<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MediaAssetRecord {
    pub(crate) id: String,
    pub(crate) source: String,
    pub(crate) source_domain: Option<String>,
    pub(crate) source_link: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) prompt: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) provider_template: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) aspect_ratio: Option<String>,
    pub(crate) size: Option<String>,
    pub(crate) quality: Option<String>,
    pub(crate) mime_type: Option<String>,
    pub(crate) content_hash: Option<String>,
    pub(crate) relative_path: Option<String>,
    pub(crate) bound_manuscript_path: Option<String>,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) absolute_path: Option<String>,
    pub(crate) preview_url: Option<String>,
    pub(crate) thumbnail_url: Option<String>,
    pub(crate) exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CoverAssetRecord {
    pub(crate) id: String,
    pub(crate) title: Option<String>,
    pub(crate) template_name: Option<String>,
    pub(crate) prompt: Option<String>,
    pub(crate) provider: Option<String>,
    pub(crate) provider_template: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) aspect_ratio: Option<String>,
    pub(crate) size: Option<String>,
    pub(crate) quality: Option<String>,
    pub(crate) relative_path: Option<String>,
    pub(crate) preview_url: Option<String>,
    pub(crate) exists: bool,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkRefsRecord {
    pub(crate) project_ids: Vec<String>,
    pub(crate) session_ids: Vec<String>,
    pub(crate) task_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkScheduleRecord {
    pub(crate) mode: String,
    pub(crate) interval_minutes: Option<i64>,
    pub(crate) time: Option<String>,
    pub(crate) weekdays: Option<Vec<i64>>,
    pub(crate) run_at: Option<String>,
    pub(crate) next_run_at: Option<String>,
    pub(crate) completed_rounds: Option<i64>,
    pub(crate) total_rounds: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WorkItemRecord {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) description: Option<String>,
    pub(crate) summary: Option<String>,
    pub(crate) status: String,
    pub(crate) effective_status: String,
    pub(crate) priority: i64,
    pub(crate) r#type: String,
    pub(crate) blocked_by: Vec<String>,
    pub(crate) refs: WorkRefsRecord,
    pub(crate) metadata: Option<Value>,
    pub(crate) schedule: WorkScheduleRecord,
    pub(crate) created_at: String,
    pub(crate) updated_at: String,
    pub(crate) completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatRuntimeStateRecord {
    pub(crate) session_id: String,
    pub(crate) is_processing: bool,
    pub(crate) partial_response: String,
    pub(crate) updated_at: u128,
    pub(crate) error: Option<String>,
    pub(crate) cancel_requested: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EditorRuntimeStateRecord {
    pub(crate) file_path: String,
    pub(crate) session_id: Option<String>,
    pub(crate) playhead_seconds: f64,
    pub(crate) selected_clip_id: Option<String>,
    pub(crate) selected_clip_ids: Option<Value>,
    pub(crate) active_track_id: Option<String>,
    pub(crate) selected_track_ids: Option<Value>,
    pub(crate) selected_scene_id: Option<String>,
    pub(crate) preview_tab: Option<String>,
    pub(crate) canvas_ratio_preset: Option<String>,
    pub(crate) active_panel: Option<String>,
    pub(crate) drawer_panel: Option<String>,
    pub(crate) scene_item_transforms: Option<Value>,
    pub(crate) scene_item_visibility: Option<Value>,
    pub(crate) scene_item_order: Option<Value>,
    pub(crate) scene_item_locks: Option<Value>,
    pub(crate) scene_item_groups: Option<Value>,
    pub(crate) focused_group_id: Option<String>,
    pub(crate) track_ui: Option<Value>,
    pub(crate) viewport_scroll_left: f64,
    pub(crate) viewport_max_scroll_left: f64,
    pub(crate) viewport_scroll_top: f64,
    pub(crate) viewport_max_scroll_top: f64,
    pub(crate) timeline_zoom_percent: f64,
    pub(crate) undo_stack: Vec<Value>,
    pub(crate) redo_stack: Vec<Value>,
    pub(crate) updated_at: u128,
}
