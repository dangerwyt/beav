import { startTransition, useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { isTauriRuntime } from '../bridge/browserHost';
import { Radar, Plus, RefreshCw, Trash2, ExternalLink, Zap, Sparkles, Check, X, Eye, Loader2, Link2, Lock, LogOut, ChevronDown, ChevronLeft, ChevronRight, FlaskConical, BookOpen, Copy, Download, Video, Film, History, AudioLines } from 'lucide-react';
import { createPortal } from 'react-dom';
import { clsx } from 'clsx';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { MonitorChromeStatus, MonitorChromeLaunchResult, MonitorChromeTestResult } from '../bridge/domains/monitorBridge';
import { resolveAssetUrl } from '../utils/pathManager';
import { CDP_PLATFORMS, PLATFORM_HOME, PLATFORMS, type CdpPlatform } from '../constants/platforms';

/** 是否是内容平台链接（小红书/抖音/B站/微博）—— 这些链接含会话凭证。 */
function isPlatformUrl(url: string): boolean {
  return /xiaohongshu\.com|douyin\.com|bilibili\.com|weibo\.com/i.test(String(url || ''));
}

interface CompetitorWatch {
  id: string;
  name: string;
  platform: string;
  profileUrl: string | null;
  handle: string | null;
  note: string | null;
  enabled: boolean;
  intervalHours: number;
  lastFetchAt: string | null;
  /** 深层历史是否已补过（补过一次后不再每晚重复深翻）。 */
  historyBackfilledAt?: string | null;
  /** 该监控已收录的帖子数（后端 watch-list 从 monitored_posts 派生，不依赖前端选中的 posts 数组）。 */
  postCount?: number;
  createdAt: string;
  updatedAt: string;
}

interface MonitoredPost {
  id: string;
  watchId: string | null;
  platform: string;
  author: string;
  authorUrl: string | null;
  postUrl: string;
  title: string;
  mediaType?: string;
  coverUrl?: string | null;
  images?: string[];
  content: string;
  /** 详情页是否成功抓取过（图/标题抓到即算，纯图/无正文笔记也算）。作为缓存钥匙，避免无正文帖每次打开都重抓。 */
  detailFetched?: boolean;
  excerpt: string | null;
  publishedAt: string | null;
  likes: number;
  comments: number;
  collects: number;
  prevLikes: number;
  prevComments: number;
  isNew: boolean;
  spike: boolean;
  spikeRatio: number;
  capturedVia: string;
  topicInjected: boolean;
  /** 是否已对该帖做过爆文拆解（后端 post_deconstructions 落库标记）。 */
  deconstructed?: boolean;
  /** 视频下载态：none / downloading / done / failed。 */
  videoStatus?: string;
  /** 本地视频文件路径（下载后填）。 */
  videoLocalPath?: string | null;
  /** 轻量代理视频路径（720p/H.264，WebView 内直接播放用）；原片常 4K/HEVC 播不了。 */
  videoPreviewPath?: string | null;
  videoSize?: number | null;
  videoDownloadedAt?: number | null;
  videoError?: string | null;
  /** 本地视频是否含音轨（null/undefined = 未探测）。抖音 DASH 常只给视频轨，无音轨则无法转写。 */
  videoHasAudio?: boolean | null;
  createdAt: string;
  updatedAt: string;
}

/** 爆文拆解沉淀：一篇对标帖经 LLM 拆出的方法论要素（对应后端 PostDeconstructionRecord）。 */
interface PostDeconstruction {
  id: string;
  postId: string;
  platform: string;
  author: string;
  postTitle: string;
  postUrl: string;
  topicAngle: string;
  copyStructure: string;
  opening: string;
  titlePatterns: string[];
  hooks: string[];
  reusableTakeaway: string;
  rawContent?: string;
  sourceEvidence?: string;
  createdAt: number;
  updatedAt: number;
}

/** 视频本体分析沉淀：ASR 字幕 + 抽帧 + LLM 摘要（对应后端 PostVideoAnalysisRecord）。 */
interface PostVideoAnalysis {
  id: string;
  postId: string;
  platform: string;
  author: string;
  postTitle: string;
  postUrl: string;
  localPath: string;
  durationSecs?: number | null;
  transcript: string;
  /** 带时间轴的字幕原文（SRT 文本），供前端渲染每句时间戳。可能为空（转写失败 / 未产出 SRT）。 */
  transcriptSrt?: string;
  /** 转写状态：ok / no-audio / not-configured / failed / pending（尚未转写）/ 空串（未知）。 */
  transcriptStatus: string;
  /** 转写失败的具体原因（status 非 ok 时后端会填），供 UI 显示真实原因。 */
  transcriptError?: string | null;
  frames: string[];
  summary: string;
  createdAt: number;
  updatedAt: number;
}

// 平台改为必填：「自动识别」靠 host 猜，遇到 b23.tv 短链 / 分享链接经常猜错，
// 干脆去掉，由用户显式选择。
const PLATFORM_OPTIONS = PLATFORMS;

const platformLabel = (code: string): string => {
  const found = PLATFORM_OPTIONS.find((opt) => opt.value === code);
  return found ? found.label : (code || '网页');
};

const formatTs = (ts: string | null | undefined): string => {
  if (!ts) return '—';
  const n = Number(ts);
  if (!Number.isFinite(n)) return ts;
  let ms = n;
  if (ms < 1e12) ms *= 1000; // 兼容秒级时间戳
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return ts;
  return d.toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' });
};

/** 把一篇拆解沉淀格式化成可复制的纯文本（用于「复制方法论」）。 */
const formatDeconText = (d: PostDeconstruction): string => {
  const lines: string[] = [];
  lines.push(`【爆文拆解】${d.postTitle || '(无标题)'}`);
  lines.push(`博主：${d.author} · ${platformLabel(d.platform)}`);
  lines.push('');
  lines.push(`一、选题角度：${d.topicAngle || '—'}`);
  lines.push(`二、文案结构：${d.copyStructure || '—'}`);
  lines.push(`三、开场白：${d.opening || '—'}`);
  lines.push(`四、标题句式：`);
  (d.titlePatterns.length ? d.titlePatterns : ['—']).forEach((t, i) => lines.push(`  ${i + 1}. ${t}`));
  lines.push(`五、钩子 / 悬念点：`);
  (d.hooks.length ? d.hooks : ['—']).forEach((h, i) => lines.push(`  ${i + 1}. ${h}`));
  lines.push(`六、可复用方法论：${d.reusableTakeaway || '—'}`);
  return lines.join('\n');
};

/**
 * 把后端回传的 deconstruction 记录（camelCase 已对齐，但容错 snake_case）规整成前端类型。
 * 纯函数，置于模块作用域 —— 避免被组件内的 useEffect 依赖数组在「声明前」引用触发 TDZ（ReferenceError）。
 */
function normalizeDecon(raw: Record<string, unknown> | null | undefined): PostDeconstruction | null {
  if (!raw) return null;
  const arr = (key: string): string[] => {
    const v = raw[key];
    return Array.isArray(v) ? v.map((x) => String(x)) : [];
  };
  const num = (key: string): number => {
    const n = Number(raw[key]);
    return Number.isFinite(n) ? n : 0;
  };
  return {
    id: String(raw.id ?? ''),
    postId: String(raw.postId ?? raw.post_id ?? ''),
    platform: String(raw.platform ?? ''),
    author: String(raw.author ?? ''),
    postTitle: String(raw.postTitle ?? raw.post_title ?? ''),
    postUrl: String(raw.postUrl ?? raw.post_url ?? ''),
    topicAngle: String(raw.topicAngle ?? raw.topic_angle ?? ''),
    copyStructure: String(raw.copyStructure ?? raw.copy_structure ?? ''),
    opening: String(raw.opening ?? ''),
    titlePatterns: arr('titlePatterns').length ? arr('titlePatterns') : arr('title_patterns'),
    hooks: arr('hooks'),
    reusableTakeaway: String(raw.reusableTakeaway ?? raw.reusable_takeaway ?? ''),
    rawContent: raw.rawContent != null ? String(raw.rawContent) : undefined,
    sourceEvidence: raw.sourceEvidence != null ? String(raw.sourceEvidence) : undefined,
    createdAt: num('createdAt') || num('created_at'),
    updatedAt: num('updatedAt') || num('updated_at'),
  };
}

/** 爆文拆解六要素展示卡（详情弹窗与方法论库共用）。fields 区 + 复制按钮。 */
function DeconFields({ decon }: { decon: PostDeconstruction }) {
  const copyAll = () => {
    const text = formatDeconText(decon);
    void navigator.clipboard?.writeText(text).catch(() => {});
  };
  const Field = ({ label, children }: { label: string; children: ReactNode }) => (
    <div>
      <div className="text-[11px] font-medium text-accent-primary/80 mb-1">{label}</div>
      <div className="text-sm text-text-secondary leading-relaxed">{children}</div>
    </div>
  );
  return (
    <div className="rounded-lg border border-accent-primary/20 bg-accent-primary/[0.04] p-3 space-y-3">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-1.5 text-[12px] font-semibold text-accent-primary">
          <FlaskConical className="w-3.5 h-3.5" />爆文拆解方法论
        </div>
        <button
          onClick={copyAll}
          className="inline-flex items-center gap-1 text-[11px] px-2 py-1 rounded-md border border-border text-text-tertiary hover:text-accent-primary hover:border-accent-primary/40 transition-colors"
          title="复制六要素为纯文本"
        >
          <Copy className="w-3 h-3" />复制
        </button>
      </div>

      <Field label="选题角度（为什么火）">{decon.topicAngle || '—'}</Field>
      <Field label="文案结构（怎么铺）">{decon.copyStructure || '—'}</Field>
      <Field label="开场白（前 3 秒）">{decon.opening || '—'}</Field>

      <Field label="标题句式">
        {decon.titlePatterns.length ? (
          <ul className="list-disc pl-5 space-y-0.5">
            {decon.titlePatterns.map((t, i) => (
              <li key={i}>{t}</li>
            ))}
          </ul>
        ) : (
          '—'
        )}
      </Field>

      <Field label="钩子 / 悬念点">
        {decon.hooks.length ? (
          <ul className="list-disc pl-5 space-y-0.5">
            {decon.hooks.map((h, i) => (
              <li key={i}>{h}</li>
            ))}
          </ul>
        ) : (
          '—'
        )}
      </Field>

      <Field label="可复用方法论 / 论点">{decon.reusableTakeaway || '—'}</Field>

      {decon.sourceEvidence && (
        <div className="text-[11px] text-text-tertiary border-t border-border pt-2">
          佐证：{decon.sourceEvidence}
        </div>
      )}
    </div>
  );
}

/**
 * 后端回传的视频分析记录（camelCase 已对齐，容错 snake_case）→ 前端类型。
 * 模块作用域纯函数，原因同 normalizeDecon：避免 useEffect 依赖数组在声明前引用触发 TDZ。
 */
function normalizeVideoAnalysis(raw: Record<string, unknown> | null | undefined): PostVideoAnalysis | null {
  if (!raw) return null;
  const postId = String(raw.postId ?? raw.post_id ?? '');
  if (!postId) return null;
  const num = (key: string, fallbackKey?: string): number => {
    // 后端返回 camelCase（如 updatedAt）；旧写法在给了 fallbackKey 时只读 snake_case，
    // 把 camelCase 值丢掉 → updatedAt 恒 0 → 展示成 1970/1/1。这里先读主键再退回 fallback。
    const value = raw[key] ?? (fallbackKey != null ? raw[fallbackKey] : undefined);
    const n = Number(value);
    return Number.isFinite(n) ? n : 0;
  };
  const framesRaw = Array.isArray(raw.frames) ? raw.frames : [];
  const duration = Number(raw.durationSecs ?? raw.duration_secs);
  return {
    id: String(raw.id ?? ''),
    postId,
    platform: String(raw.platform ?? ''),
    author: String(raw.author ?? ''),
    postTitle: String(raw.postTitle ?? raw.post_title ?? ''),
    postUrl: String(raw.postUrl ?? raw.post_url ?? ''),
    localPath: String(raw.localPath ?? raw.local_path ?? ''),
    durationSecs: Number.isFinite(duration) ? duration : null,
    transcript: String(raw.transcript ?? ''),
    transcriptSrt:
      raw.transcriptSrt != null || raw.transcript_srt != null
        ? String(raw.transcriptSrt ?? raw.transcript_srt ?? '')
        : '',
    transcriptStatus: String(raw.transcriptStatus ?? raw.transcript_status ?? ''),
    transcriptError:
      raw.transcriptError != null || raw.transcript_error != null
        ? String(raw.transcriptError ?? raw.transcript_error ?? '')
        : null,
    frames: framesRaw.map((f) => String(f)).filter(Boolean),
    summary: String(raw.summary ?? ''),
    createdAt: num('createdAt', 'created_at'),
    updatedAt: num('updatedAt', 'updated_at'),
  };
}

/** 解析 SRT 时间戳（`00:01:02,300` / `00:01:02.300` / `01:02,300`）为秒数。 */
function parseSrtTimestamp(ts: string): number {
  const cleaned = ts.trim().replace(',', '.');
  const parts = cleaned.split(':').map(Number);
  let secs = 0;
  if (parts.length === 3) secs = (parts[0] || 0) * 3600 + (parts[1] || 0) * 60 + (parts[2] || 0);
  else if (parts.length === 2) secs = (parts[0] || 0) * 60 + (parts[1] || 0);
  else if (parts.length === 1) secs = parts[0] || 0;
  return Number.isFinite(secs) ? secs : 0;
}

/** 秒数 → `mm:ss`。 */
function formatClock(sec: number): string {
  const s = Math.max(0, Math.floor(sec));
  const m = Math.floor(s / 60);
  const r = s % 60;
  return `${String(m).padStart(2, '0')}:${String(r).padStart(2, '0')}`;
}

/** 解析 SRT 文本为带起止时间的分句列表（用于「时间轴」展示）。 */
function parseSrt(srt: string): { start: number; text: string }[] {
  const blocks = srt.replace(/\r/g, '').split(/\n\s*\n/);
  const out: { start: number; text: string }[] = [];
  for (const block of blocks) {
    const lines = block.split('\n').map((l) => l.trim()).filter(Boolean);
    let start = 0;
    const textLines: string[] = [];
    for (const line of lines) {
      if (line.includes('-->')) {
        start = parseSrtTimestamp(line.split('-->')[0]);
        continue;
      }
      if (/^\d+$/.test(line)) continue; // 序号行
      textLines.push(line);
    }
    const text = textLines.join(' ').trim();
    if (text) out.push({ start, text });
  }
  return out;
}

/** 视频分析展示卡：时长 + 关键帧网格 + 语音转写（全文 / 时间轴）+ LLM 摘要。 */
function VideoAnalysisPanel({ analysis }: { analysis: PostVideoAnalysis }) {
  const [showAll, setShowAll] = useState(false);
  const [viewMode, setViewMode] = useState<'text' | 'timeline'>('text');
  const transcript = analysis.transcript.trim();
  const segs = parseSrt(analysis.transcriptSrt || '');
  const hasTimeline = segs.length > 0;
  const longTranscript = transcript.length > 600;
  return (
    <div className="rounded-lg border border-emerald-500/25 bg-emerald-500/[0.04] p-3 space-y-3">
      <div className="flex items-center gap-1.5 text-[12px] font-semibold text-emerald-600 dark:text-emerald-400">
        <Film className="w-3.5 h-3.5" />视频本体分析
        {analysis.durationSecs != null && (
          <span className="text-[11px] font-normal text-text-tertiary">
            · {Math.round(analysis.durationSecs)} 秒
          </span>
        )}
      </div>

      {analysis.frames.length > 0 && (
        <div>
          <div className="text-[11px] font-medium text-text-tertiary mb-1.5">
            关键帧（{analysis.frames.length} 张 / 每 5s 一帧）
          </div>
          <div className="grid grid-cols-4 gap-1.5">
            {analysis.frames.slice(0, 8).map((frame, i) => (
              <img
                key={i}
                src={resolveAssetUrl(frame)}
                alt={`关键帧 ${i + 1}`}
                className="w-full aspect-square object-cover rounded border border-border bg-surface-secondary"
                loading="lazy"
              />
            ))}
          </div>
          {analysis.frames.length > 8 && (
            <div className="text-[11px] text-text-tertiary mt-1">仅显示前 8 帧，共 {analysis.frames.length} 张</div>
          )}
        </div>
      )}

      {transcript ? (
        <div>
          <div className="flex items-center justify-between gap-2 mb-1">
            <span className="text-[11px] font-medium text-text-tertiary">语音转写（ASR）</span>
            {hasTimeline && (
              <div className="inline-flex rounded-md border border-border overflow-hidden text-[11px]">
                <button
                  onClick={() => setViewMode('text')}
                  className={clsx(
                    'px-2 py-0.5 transition-colors',
                    viewMode === 'text' ? 'bg-accent-primary text-white' : 'text-text-tertiary hover:text-accent-primary',
                  )}
                >
                  全文
                </button>
                <button
                  onClick={() => setViewMode('timeline')}
                  className={clsx(
                    'px-2 py-0.5 border-l border-border transition-colors',
                    viewMode === 'timeline' ? 'bg-accent-primary text-white' : 'text-text-tertiary hover:text-accent-primary',
                  )}
                >
                  时间轴
                </button>
              </div>
            )}
          </div>
          {viewMode === 'timeline' && hasTimeline ? (
            <ol className="max-h-60 overflow-auto pr-1 space-y-1.5">
              {segs.map((seg, i) => (
                <li key={i} className="flex gap-2 text-sm text-text-secondary leading-relaxed">
                  <span className="shrink-0 font-mono text-[11px] text-accent-primary/80 pt-0.5 tabular-nums">
                    {formatClock(seg.start)}
                  </span>
                  <span>{seg.text}</span>
                </li>
              ))}
            </ol>
          ) : (
            <>
              <p className="text-sm text-text-secondary whitespace-pre-wrap leading-relaxed">
                {showAll || !longTranscript ? transcript : `${transcript.slice(0, 600)}…`}
              </p>
              {longTranscript && (
                <button
                  onClick={() => setShowAll((v) => !v)}
                  className="mt-1 text-[11px] text-accent-primary hover:underline"
                >
                  {showAll ? '收起' : `展开全文（共 ${transcript.length} 字）`}
                </button>
              )}
            </>
          )}
        </div>
      ) : (
        <div className="text-[11px] text-text-tertiary">
          {analysis.transcriptStatus === 'no-audio'
            ? '该视频没有音轨（抖音 DASH 只提供视频轨），无法做语音转写。重新下载该视频会自动补音轨后再分析。'
            : analysis.transcriptStatus === 'not-configured'
              ? '未配置语音转写服务 —— 请到「设置」填写 transcription 端点 / 模型后重新分析。'
              : analysis.transcriptStatus === 'pending'
                ? '排队等待语音转写（下载阶段结束后自动补）。'
                : analysis.transcriptStatus === 'failed'
                  ? `语音转写失败：${analysis.transcriptError || '原因未知'}`
                  : '未取到语音转写 —— 可能是纯音乐 / 无口播视频，或转写服务未配置。'}
        </div>
      )}

      {analysis.summary && (
        <div>
          <div className="text-[11px] font-medium text-text-tertiary mb-1">内容摘要</div>
          <p className="text-sm text-text-secondary whitespace-pre-wrap leading-relaxed">{analysis.summary}</p>
        </div>
      )}

      <div className="text-[11px] text-text-tertiary border-t border-border pt-2">
        分析于 {formatTs(String(analysis.updatedAt))}
        {analysis.localPath ? ` · ${analysis.localPath}` : ''}
      </div>
    </div>
  );
}

/**
 * 本地视频播放器：优先播 720p/H.264 轻量代理（WebView 对 4K/HEVC 原片往往放不出来）；
 * 代理缺失时从原片路径派生 `<id>.preview.mp4` 试探（已下载的原片可能已有代理）；
 * 派生也 404 → onError 回退到原片（H.264 ≤1080p + faststart 在 asset 协议下可播）。
 *
 * 提示只在**回退原片后仍播放失败**时出现：H.264 ≤1080p 的原片本就不生成代理，
 * 若按「有没有代理路径」判断会误报成「旧版本下载」。
 */
function VideoPlayerWithFallback({
  localPath,
  previewPath,
}: {
  localPath: string;
  previewPath?: string;
}) {
  const derivedPreview = previewPath
    || (localPath && /\.mp4(\?|#|$)/i.test(localPath)
      ? localPath.replace(/\.mp4(\?|#|$)/i, '.preview.mp4$1')
      : null);
  const [src, setSrc] = useState<string | null>(derivedPreview);
  const [triedFallback, setTriedFallback] = useState(false);
  const [playbackFailed, setPlaybackFailed] = useState(false);

  // 详情弹窗内是同一个组件实例，切换帖子时必须重置播放状态
  useEffect(() => {
    setSrc(derivedPreview);
    setTriedFallback(false);
    setPlaybackFailed(false);
  }, [derivedPreview, localPath]);

  const onError = () => {
    if (!triedFallback && src && src !== localPath) {
      setTriedFallback(true);
      setSrc(localPath);
      return;
    }
    setPlaybackFailed(true);
  };
  if (!src) return null;
  return (
    <>
      <video
        key={src}
        src={resolveAssetUrl(src)}
        controls
        preload="metadata"
        onError={onError}
        className="w-full max-h-80 rounded-lg border border-border bg-black"
      />
      {playbackFailed && (
        <p className="text-[11px] text-amber-600 dark:text-amber-400">
          内置播放器无法播放该视频（多为 4K / HEVC 等 WebView 不支持的编码）。可点「重新下载」生成 720p 可播放版，或用系统播放器打开原片。
        </p>
      )}
    </>
  );
}

export function Monitor({ isActive = true }: { isActive?: boolean }) {
  const [watches, setWatches] = useState<CompetitorWatch[]>([]);
  const [selectedWatchId, setSelectedWatchId] = useState<string | null>(null);
  const [posts, setPosts] = useState<MonitoredPost[]>([]);

  const [loadingWatches, setLoadingWatches] = useState(true);
  const [loadingPosts, setLoadingPosts] = useState(false);
  const [fetching, setFetching] = useState(false);
  /** 正在抓取的博主 id（null=空闲；'__adhoc__'=临时抓取），只让该条目转圈。 */
  const [fetchingWatchId, setFetchingWatchId] = useState<string | null>(null);
  /** 「补历史」进行中的博主 id（null = 空闲）。深翻要滚几十秒，按钮各自转圈。 */
  const [historyBusyId, setHistoryBusyId] = useState<string | null>(null);
  /** 补历史实时进度：watchId -> { scanned, total }，驱动「已抓 X / 共 Y」进度条。 */
  const [backfillProgress, setBackfillProgress] = useState<Record<string, { scanned: number; total: number; phase?: string }>>({});
  /** 上次补历史的诊断（常驻显示，不靠一闪而过的 toast）：
   *  后端已经能算出「页面没有发生滚动 / 滚动后没有加载出更多内容（停在 N 条）」，
   *  但只弹 toast 会让人完全错过 —— 「只抓到 3 条」这类问题必须看得见。 */
  const [backfillDiag, setBackfillDiag] = useState<Record<string, { hint: string; scanned: number; detail?: string; level?: 'ok' | 'warn' }>>({});
  const [adhocUrl, setAdhocUrl] = useState('');
  const [fetchNote, setFetchNote] = useState<string | null>(null);
  const [toast, setToast] = useState<{ kind: 'ok' | 'err'; text: string } | null>(null);
  const [watchesError, setWatchesError] = useState<string | null>(null);
  const [postsError, setPostsError] = useState<string | null>(null);

  const [addOpen, setAddOpen] = useState(false);
  const [addForm, setAddForm] = useState<{
    name: string;
    platform: string;
    profileUrl: string;
    note: string;
  }>({
    name: '',
    // 平台必填但**不给默认值**：下拉显示「请选择平台」占位，用户必须显式选
    platform: '',
    profileUrl: '',
    note: '',
  });
  const [addError, setAddError] = useState('');

  // —— 本地详情弹窗（点标题在 Beav 内看正文，不跳平台）——
  const [detailPost, setDetailPost] = useState<MonitoredPost | null>(null);
  const [detailOpen, setDetailOpen] = useState(false);
  const [detailLoading, setDetailLoading] = useState(false);
  const [detailNote, setDetailNote] = useState<string | null>(null);
  const [detailImages, setDetailImages] = useState<string[]>([]);
  // 大图预览：点缩略图打开，记录当前查看的索引（支持左右切换 / ESC / 点背景关闭）
  const [lightboxIndex, setLightboxIndex] = useState<number | null>(null);

  // —— Chrome 头浏览器（小红书 / 抖音登录态抓取）——
  const [chromeStatus, setChromeStatus] = useState<MonitorChromeStatus | null>(null);
  const [chromeOpen, setChromeOpen] = useState(true);
  const [chromeBusy, setChromeBusy] = useState(false);
  const [chromeLoginPlatform, setChromeLoginPlatform] = useState<CdpPlatform>(CDP_PLATFORMS[0]);
  const [chromeTestResult, setChromeTestResult] = useState<string | null>(null);

  // —— 扫码登录弹窗 ——
  const [qrOpen, setQrOpen] = useState(false);
  const [qrPlatform, setQrPlatform] = useState<CdpPlatform>('xiaohongshu');
  const [qrImage, setQrImage] = useState<string | null>(null);
  const [qrBusy, setQrBusy] = useState(false);
  const [qrNote, setQrNote] = useState<string | null>(null);
  const [qrDiag, setQrDiag] = useState<string | null>(null);
  const [qrDone, setQrDone] = useState(false);
  // 服务器已登录该平台 → 需要用户确认是否强制更换账号
  const [qrNeedForce, setQrNeedForce] = useState(false);
  /** 取码进行中标记（用 ref 防重入，state 在闭包里会读到旧值） */
  const qrLoadingRef = useRef(false);
  // Web 普通用户不是管理员 → 隐藏扫码/退出等管理员专属操作
  const [isAdminUser, setIsAdminUser] = useState(true);
  /** 平台登录态：null = 未知/未检测，true = 已登录，false = 未登录。
   *  与「Chrome 已就绪」是两回事 —— Chrome 起着但平台没登录时，
   *  抓取只会拿到游客视角的首屏几条（这是「只抓到 3-6 条」的根因）。 */
  const [platformLoginState, setPlatformLoginState] = useState<boolean | null>(null);
  /** 登录诊断原始证据（点了「诊断」才取，避免频繁打扰页面） */
  const [loginDebug, setLoginDebug] = useState<string | null>(null);

  // 轮询平台登录态（10s 一次；后端有 cookie 指纹缓存，不会频繁动浏览器）
  const checkPlatformLogin = useCallback(async (refresh = false) => {
    try {
      const status = await window.ipcRenderer.monitor.chromeQrStatus(
        chromeLoginPlatform,
        refresh,
      );
      setPlatformLoginState(status?.loggedIn === true);
      return status?.loggedIn === true;
    } catch {
      setPlatformLoginState(null);
      return false;
    }
  }, [chromeLoginPlatform]);

  useEffect(() => {
    if (!isActive) return;
    void checkPlatformLogin(false);
    const timer = window.setInterval(() => void checkPlatformLogin(false), 10_000);
    return () => window.clearInterval(timer);
  }, [isActive, checkPlatformLogin]);

  useEffect(() => {
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

  // —— 爆文拆解（详情弹窗内）——
  const [deconResult, setDeconResult] = useState<PostDeconstruction | null>(null);
  const [deconLoading, setDeconLoading] = useState(false);
  const [deconError, setDeconError] = useState<string | null>(null);

  // —— 方法论库（已沉淀的爆文拆解列表，模态浏览）——
  const [methodsOpen, setMethodsOpen] = useState(false);
  const [methodsItems, setMethodsItems] = useState<PostDeconstruction[]>([]);
  const [methodsAuthors, setMethodsAuthors] = useState<string[]>([]);
  const [methodsLoading, setMethodsLoading] = useState(false);
  const [methodsAuthor, setMethodsAuthor] = useState('');
  const [methodsPlatform, setMethodsPlatform] = useState('');
  const [methodsExpanded, setMethodsExpanded] = useState<Set<string>>(new Set());

  // —— 视频下载 + 本体分析（详情弹窗内，仅 mediaType==='video' 的帖可用）——
  const [videoBusyId, setVideoBusyId] = useState<string | null>(null);
  const [videoAnalyzingId, setVideoAnalyzingId] = useState<string | null>(null);
  const [videoError, setVideoError] = useState<string | null>(null);
  const [videoAnalysis, setVideoAnalysis] = useState<PostVideoAnalysis | null>(null);
  // 分析视频时是否顺带做语音转写（关闭则只抽帧，不产出转写全文/时间轴）。
  const [transcribeOnAnalyze, setTranscribeOnAnalyze] = useState(true);

  const watchesRef = useRef<CompetitorWatch[]>([]);
  const selectedRef = useRef<string | null>(null);

  // 关闭详情弹窗时一并关闭大图预览
  useEffect(() => {
    if (!detailOpen) setLightboxIndex(null);
  }, [detailOpen]);

  // 大图预览的键盘操作：ESC 关闭，左右方向键切换（多图时）
  useEffect(() => {
    if (lightboxIndex === null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        setLightboxIndex(null);
      } else if (e.key === 'ArrowLeft' && detailImages.length > 1) {
        setLightboxIndex((i) => (i === null ? null : (i - 1 + detailImages.length) % detailImages.length));
      } else if (e.key === 'ArrowRight' && detailImages.length > 1) {
        setLightboxIndex((i) => (i === null ? null : (i + 1) % detailImages.length));
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [lightboxIndex, detailImages.length]);
  // 两个列表各自的加载序号 —— 不能共用一个 ref，否则先后发起的请求会互相作废，
  // 导致先发出的那个永远不落 loading=false（「一直加载中 / 请求超时」的真凶）。
  const watchesLoadSeq = useRef(0);
  const postsLoadSeq = useRef(0);

  useEffect(() => {
    watchesRef.current = watches;
    selectedRef.current = selectedWatchId;
  }, [watches, selectedWatchId]);

  const loadWatches = useCallback(async () => {
    const seq = watchesLoadSeq.current + 1;
    watchesLoadSeq.current = seq;
    setLoadingWatches(true);
    setWatchesError(null);
    try {
      const list = (await window.ipcRenderer.monitor.watchList({})) as CompetitorWatch[];
      if (seq !== watchesLoadSeq.current) return;
      setWatches(Array.isArray(list) ? list : []);
    } catch (e) {
      console.error('Failed to load watchlist:', e);
      if (seq === watchesLoadSeq.current) {
        setWatches([]);
        setWatchesError(e instanceof Error ? e.message : '加载监控列表失败');
      }
    } finally {
      if (seq === watchesLoadSeq.current) setLoadingWatches(false);
    }
  }, []);

  const loadPosts = useCallback(async () => {
    const seq = postsLoadSeq.current + 1;
    postsLoadSeq.current = seq;
    setLoadingPosts(true);
    setPostsError(null);
    try {
      const payload: Record<string, unknown> = {};
      if (selectedRef.current) payload.watchId = selectedRef.current;
      const list = (await window.ipcRenderer.monitor.postList(payload)) as MonitoredPost[];
      if (seq !== postsLoadSeq.current) return;
      // 切换博主时上百张卡片同时重建，用 transition 让输入/滚动不被阻塞。
      startTransition(() => setPosts(Array.isArray(list) ? list : []));
    } catch (e) {
      console.error('Failed to load posts:', e);
      if (seq === postsLoadSeq.current) {
        setPosts([]);
        setPostsError(e instanceof Error ? e.message : '加载动态失败');
      }
    } finally {
      if (seq === postsLoadSeq.current) setLoadingPosts(false);
    }
  }, []);

  const loadChromeStatus = useCallback(async () => {
    try {
      const status = (await window.ipcRenderer.monitor.chromeStatus(chromeLoginPlatform)) as MonitorChromeStatus;
      setChromeStatus(status);
    } catch (e) {
      console.warn('chrome status failed:', e);
    }
  }, [chromeLoginPlatform]);

  // 扫码登录：取二维码 → 轮询登录态
  // force=true：服务器该平台已登录时仍然更换账号（扫码本来就是换号的行为，
  // 普通 web 用户没有「退出登录」权限，硬拦会让她无路可走）。
  const loadQrCode = useCallback(async (platform: CdpPlatform, force = false) => {
    // 防重入：并发调用会各自取一次码，后一次会把前一次的码作废
    // （小红书登录码按会话唯一），表现为"两个标签 + 第一个显示过期"。
    if (qrLoadingRef.current) return;
    qrLoadingRef.current = true;
    setQrBusy(true);
    setQrDone(false);
    setQrNote(null);
    setQrNeedForce(false);
    try {
      const result = await window.ipcRenderer.monitor.chromeQrStart({ platform, force });
      if (!result?.success || !result.image) {
        setQrImage(null);
        const error = result?.error || '获取二维码失败';
        if (error.includes('已登录')) {
          setQrNote('服务器该平台已登录（账号对所有用户共享）。继续扫码会更换服务器上的账号。');
          setQrNeedForce(true);
        } else {
          setQrNote(error);
        }
        setQrDiag(null);
        return;
      }
      setQrImage(result.image);
      const bits: string[] = [];
      if (result.kind) bits.push(`来源=${result.kind}`);
      if (result.refreshed) bits.push('已自动点刷新');
      if (result.expired) bits.push('页面提示二维码已失效');
      if (result.snippet) bits.push(`页面文本：${String(result.snippet).slice(0, 120)}`);
      setQrDiag(bits.length ? bits.join(' | ') : null);
      setQrNote(
        result.needVerify
          ? '⚠️ 小红书要求身份验证：请用**已登录该账号的小红书 App** 扫这张码完成验证（验证后才会真正登录）'
          : '请用对应 App 扫描二维码',
      );
    } catch (e) {
      setQrImage(null);
      setQrNote(`获取二维码失败：${String(e)}`);
    } finally {
      qrLoadingRef.current = false;
      setQrBusy(false);
    }
  }, []);

  const openQrModal = useCallback((platform: CdpPlatform = 'xiaohongshu') => {
    setQrPlatform(platform);
    setQrOpen(true);
    void loadQrCode(platform);
  }, [loadQrCode]);

  // 弹窗打开时轮询登录态
  useEffect(() => {
    if (!qrOpen || qrDone) return;
    const timer = window.setInterval(() => {
      void (async () => {
        try {
          const status = await window.ipcRenderer.monitor.chromeQrStatus(qrPlatform);
          if (status?.loggedIn) {
            setQrDone(true);
            setQrNote('已登录 ✅');
            void loadChromeStatus();
            // 登录成功后自动关掉弹窗（用户反馈"手机显示成功，弹窗没关"）
            window.setTimeout(() => setQrOpen(false), 1200);
          }
        } catch {
          /* 轮询失败忽略 */
        }
      })();
    }, 5000);
    return () => window.clearInterval(timer);
  }, [qrOpen, qrDone, qrPlatform, loadChromeStatus]);

  // 注意：**不要**定时自动换码。
  // 小红书扫码登录靠登录页标签自己轮询状态、拿到会话后写 cookie；
  // 定时刷新/换标签会把正在等待回调的页面打断 → 手机提示成功、网页端却没登录。
  // 二维码过期时页面上就有「点击刷新」，后端提取时会自动点它。

  useEffect(() => {
    if (!isActive) return;
    void loadWatches();
  }, [isActive, loadWatches]);

  useEffect(() => {
    if (!isActive) return;
    void loadPosts();
  }, [isActive, selectedWatchId, loadPosts]);

  // 单次抓取条数已由后端固定为 30（SCAN_LIMIT），前端不再拉取/展示可配置项。

  useEffect(() => {
    if (!isActive) return;
    void loadChromeStatus();
  }, [isActive, loadChromeStatus]);

  // 看门狗：兜底防止 IPC 卡死导致永久 loading（16s 强制解锁并展示错误提示）
  useEffect(() => {
    if (!isActive) return;
    const timer = window.setTimeout(() => {
      setLoadingWatches((prev) => {
        if (prev && watchesRef.current.length === 0) {
          setWatchesError((msg) => msg || '请求超时（16s），请检查后端通道是否注册');
        }
        return false;
      });
      setLoadingPosts((prev) => {
        if (prev && posts.length === 0) {
          setPostsError((msg) => msg || '请求超时（16s），请检查后端通道是否注册');
        }
        return false;
      });
    }, 16_000);
    return () => window.clearTimeout(timer);
  }, [isActive, posts.length]);

  const showToast = useCallback((kind: 'ok' | 'err', text: string) => {
    setToast({ kind, text });
    window.setTimeout(() => setToast(null), 2600);
  }, []);

  // 监听后端「每日 00:00 批量预取」完成事件：常驻 APP 的帖子列表是内存旧快照，
  // 预取已把新帖的正文/媒体类型/视频落库，必须刷新列表才能命中 openDetail 的本地缓存 → 秒开。
  // 没有这一层，即使后端已补全，用户次日打开仍要走异步 fetch（不「秒见正文」）。
  useEffect(() => {
    if (!isActive) return;
    let unlisten: UnlistenFn | undefined;
    listen<{
      newPosts?: number;
      videosDownloaded?: number;
      videosTranscribed?: number;
      detailEnriched?: number;
      detailSkipped?: number;
      skipNote?: string | null;
      backfilled?: number;
      recoveredDownloading?: number;
    }>('monitor:prefetch-done', (event) => {
      void loadPosts();
      const p = event.payload ?? {};
      const parts: string[] = [];
      if ((p.newPosts ?? 0) > 0) parts.push(`新增 ${p.newPosts} 篇`);
      if ((p.detailEnriched ?? 0) > 0) parts.push(`补全正文 ${p.detailEnriched} 篇`);
      if ((p.backfilled ?? 0) > 0) parts.push(`回填存量 ${p.backfilled} 篇`);
      if ((p.videosTranscribed ?? 0) > 0) parts.push(`转写 ${p.videosTranscribed} 个视频`);
      if (parts.length) showToast('ok', `每日预取完成：${parts.join('、')}，已可秒开`);
      // 正文没抓到就明确告知原因（常见：该平台 Chrome 未连接），否则用户只看到「打开没正文」
      if ((p.detailSkipped ?? 0) > 0) {
        showToast(
          'err',
          `有 ${p.detailSkipped} 篇未抓到正文${p.skipNote ? `：${p.skipNote}` : ''}`,
        );
      }
    })
      .then((u) => {
        unlisten = u;
      })
      .catch(() => {});
    return () => {
      unlisten?.();
    };
  }, [isActive, loadPosts, showToast]);

  // 监听后端「补历史」实时进度：深翻是 Rust 侧逐轮滚动，每轮上报一次
  // { watchId, scanned, total }，前端据此刷新「已抓 X / 共 Y」进度条。
  useEffect(() => {
    if (!isActive) return;
    let unlisten: UnlistenFn | undefined;
    listen<{ watchId?: string; scanned?: number; total?: number; phase?: string }>(
      'monitor:backfill-progress',
      (event) => {
        const p = event.payload ?? {};
        if (!p.watchId) return;
        const wid = p.watchId as string;
        // phase=videos-done：本批视频处理结束。多批并行时，先完成的小批
        // 不能把还在跑的大批进度条删掉 → 只有完成批次 >= 当前记录的批次才清。
        if (p.phase === 'videos-done') {
          const incomingTotal = p.total ?? 0;
          setBackfillProgress((prev) => {
            const current = prev[wid];
            if (current && current.total > incomingTotal) return prev;
            const next = { ...prev };
            delete next[wid];
            return next;
          });
          return;
        }
        setBackfillProgress((prev) => ({
          ...prev,
          [wid]: {
            scanned: p.scanned ?? 0,
            total: p.total ?? 0,
            phase: p.phase,
          },
        }));
      },
    )
      .then((u) => {
        unlisten = u;
      })
      .catch(() => {});
    return () => {
      unlisten?.();
    };
  }, [isActive]);

  const openExternal = useCallback(async (url?: string | null) => {
    if (!url) return;
    // Web 模式下「原文」链接带的是**服务器会话**的 xsec_token：只有本地浏览器登录了
    // 同一个小红书账号才打得开，否则会 404/跳登录。与其让用户撞一脸登录框，
    // 不如复制链接并说明——正文/图片其实已经被服务器抓进本工作区了。
    if (!isTauriRuntime() && isPlatformUrl(url)) {
      try {
        await navigator.clipboard.writeText(url);
        showToast('ok', '链接已复制（该链接含服务器会话凭证；正文与图片已抓取到本工作区，直接看详情即可）');
      } catch {
        showToast('ok', url);
      }
      return;
    }
    try {
      await window.ipcRenderer.openExternalUrl(url);
    } catch (e) {
      console.warn('openExternalUrl failed:', e);
    }
  }, [showToast]);

  const handleFetchWatch = useCallback(async (watch: CompetitorWatch) => {
    setFetching(true);
    setFetchingWatchId(watch.id);
    setFetchNote(null);
    try {
      const result = (await window.ipcRenderer.monitor.fetch({ watchId: watch.id })) as {
        success?: boolean;
        scanned?: number;
        created?: MonitoredPost[];
        note?: string;
      };
      const createdCount = result?.created?.length ?? 0;
      setFetchNote(result?.note || `已扫描 ${result?.scanned ?? 0} 条链接，新增 ${createdCount} 条动态。`);
      await Promise.all([loadWatches(), loadPosts()]);
      showToast('ok', `「${watch.name}」抓取完成，新增 ${createdCount} 条动态`);
    } catch (e) {
      console.error('fetch failed:', e);
      showToast('err', e instanceof Error ? e.message : '抓取失败');
    } finally {
      setFetching(false);
      setFetchingWatchId(null);
    }
  }, [loadWatches, loadPosts, showToast]);

  const handleAdhocFetch = useCallback(async () => {
    const url = adhocUrl.trim();
    if (!url) {
      setAddError('请输入主页或单篇链接');
      return;
    }
    setFetching(true);
    setFetchingWatchId('__adhoc__');
    setFetchNote(null);
    try {
      const result = (await window.ipcRenderer.monitor.fetch({ url })) as {
        success?: boolean;
        scanned?: number;
        created?: MonitoredPost[];
        note?: string;
      };
      const createdCount = result?.created?.length ?? 0;
      setFetchNote(result?.note || `已扫描 ${result?.scanned ?? 0} 条链接，新增 ${createdCount} 条动态。`);
      await Promise.all([loadWatches(), loadPosts()]);
      showToast('ok', `抓取完成，新增 ${createdCount} 条动态`);
    } catch (e) {
      console.error('adhoc fetch failed:', e);
      showToast('err', e instanceof Error ? e.message : '抓取失败');
    } finally {
      setFetching(false);
      setFetchingWatchId(null);
    }
  }, [adhocUrl, loadWatches, loadPosts, showToast]);

  // 补历史：深翻博主主页，把更早的历史帖一次性抓下来（日常抓取只取最新 N 条）。
  const handleBackfillHistory = useCallback(async (watchId: string) => {
    setHistoryBusyId(watchId);
    try {
      const result = (await window.ipcRenderer.monitor.backfillHistory(watchId)) as {
        success?: boolean;
        watched?: number;
        scanned?: number;
        profileTotal?: number;
        newPosts?: number;
        detailEnriched?: number;
        detailSkipped?: number;
        skipNote?: string | null;
        pendingVideos?: number;
        notLoggedIn?: number;
        hint?: string | null;
        hintLevel?: string | null;
        failed?: number;
      };
      const added = result?.newPosts ?? 0;
      const enriched = result?.detailEnriched ?? 0;
      const scanned = result?.scanned ?? 0;
      const profileTotal = result?.profileTotal ?? 0;
      await Promise.all([loadWatches(), loadPosts()]);
      if (scanned === 0) {
        // 一条都没抓到 → 报出原因（最常见：Chrome 未连接）
        showToast(
          'err',
          `没抓到任何帖子${result?.skipNote ? `：${result.skipNote}` : '（请确认该平台 Chrome 已连接）'}`,
        );
      } else {
        const scrapedPosts = (result as { scrapedPosts?: number })?.scrapedPosts ?? 0;
        const updatedExisting = (result as { updatedExisting?: number })?.updatedExisting ?? 0;
        const droppedDomain = (result as { droppedDomain?: number })?.droppedDomain ?? 0;
        showToast(
          'ok',
          `补历史完成：抓到 ${scrapedPosts || scanned} 条${profileTotal > 0 ? `（主页共 ${profileTotal} 篇）` : ''}，新建 ${added} 篇`
          + (updatedExisting > 0 ? `，已存在 ${updatedExisting} 篇` : '')
          + (droppedDomain > 0 ? `，域名不符丢弃 ${droppedDomain} 条` : '')
          + `，补全正文 ${enriched} 篇`,
        );
      }
      if ((result?.detailSkipped ?? 0) > 0) {
        showToast(
          'err',
          `有 ${result?.detailSkipped} 篇未抓到正文${result?.skipNote ? `：${result.skipNote}` : ''}`,
        );
      }
      if ((result?.pendingVideos ?? 0) > 0) {
        showToast('ok', `已开始后台处理 ${result?.pendingVideos} 个历史视频（下载+转写），完成后列表自动刷新`);
      }
      if ((result?.notLoggedIn ?? 0) > 0) {
        showToast(
          'err',
          `服务器未登录该平台（${result?.notLoggedIn} 位博主）：游客视角只能拿到首屏几条，请先扫码登录再补历史`,
        );
      }
      if (result?.hint) {
        // 「已抓全 N 篇」是成功信息，不该弹红条
        showToast(result.hintLevel === 'ok' ? 'ok' : 'err', result.hint);
      }
      // 诊断常驻：toast 2.6 秒就没了，而"只抓到 N 条"这类问题需要事后还能看到
      const diagDetail = (() => {
        const diags = (result as { scrollDiags?: { name?: string; diag?: Record<string, unknown> }[] })?.scrollDiags;
        const first = diags?.[0]?.diag;
        // 落库计数：抓到多少 / 新建多少 / 已存在多少 —— 直接回答"47 条去哪了"
        const scrapedPosts = (result as { scrapedPosts?: number })?.scrapedPosts ?? 0;
        const updatedExisting = (result as { updatedExisting?: number })?.updatedExisting ?? 0;
        const droppedDomain = (result as { droppedDomain?: number })?.droppedDomain ?? 0;
        const added = result?.newPosts ?? 0;
        const parts: string[] = [];
        if (scrapedPosts > 0) {
          parts.push(
            `抓到 ${scrapedPosts} → 新建 ${added} / 已存在 ${updatedExisting}`
            + (droppedDomain > 0 ? ` / 丢弃 ${droppedDomain}` : ''),
          );
        }
        // 库内实况：直接回答"新建了 N 条为什么只显示 M 条"
        const storeForWatch = (result as { storePostsForWatch?: number })?.storePostsForWatch;
        const storeTotal = (result as { storePostsTotal?: number })?.storePostsTotal;
        const fetchRuns = (result as { fetchRuns?: number })?.fetchRuns;
        if (typeof storeForWatch === 'number') {
          parts.push(`库内该博主 ${storeForWatch} 条 / 全库 ${storeTotal ?? 0} 条`);
        }
        if ((fetchRuns ?? 0) > 1) {
          parts.push(`抓取执行了 ${fetchRuns} 次`);
        }
        // 视频诊断：直接回答"补齐了 N 篇为什么一个视频都没下载"
        const videoDiags = (result as {
          videoDiags?: { name?: string; posts?: number; typedVideo?: number; missingType?: number; pending?: number }[];
        })?.videoDiags;
        for (const item of videoDiags ?? []) {
          if (!item?.posts) continue;
          parts.push(
            `视频 ${item.typedVideo ?? 0}/${item.posts} 篇识别为视频`
            + (item.pending ? `，已启动 ${item.pending} 个下载` : '，未启动下载')
            + ((item.missingType ?? 0) > 0 ? `（${item.missingType} 篇缺媒体类型）` : ''),
          );
        }
        if (!first) return parts.length ? parts.join(' · ') : undefined;
        const scroller = typeof first.scroller === 'string' ? first.scroller : '';
        const scrolled = first.scrolled === true;
        const loggedIn = first.loggedIn !== false;
        const counts = Array.isArray(first.counts) ? (first.counts as number[]) : [];
        const reachedBottom = first.reachedBottom === true;
        return [
          loggedIn ? '已登录' : '未登录',
          reachedBottom ? '已到底' : (scrolled ? '发生了滚动' : '页面没有发生滚动'),
          scroller ? `容器 ${scroller}` : '',
          counts.length ? `每轮条数 ${counts.slice(0, 6).join('→')}` : '',
          ...parts,
        ].filter(Boolean).join(' · ');
      })();
      setBackfillDiag((prev) => ({
        ...prev,
        [watchId]: {
          hint: result?.hint || '滚动加载正常，未发现异常',
          scanned,
          detail: diagDetail,
          level: result?.hintLevel === 'ok' ? 'ok' : 'warn',
        },
      }));
    } catch (e) {
      console.error('backfill history failed:', e);
      showToast('err', e instanceof Error ? e.message : '补历史失败');
    } finally {
      setHistoryBusyId(null);
      setBackfillProgress((prev) => {
        // 视频还在后台批处理（phase=videos）→ 保留进度条，等 videos-done 事件再清
        if (prev[watchId]?.phase === 'videos') return prev;
        const next = { ...prev };
        delete next[watchId];
        return next;
      });
    }
  }, [loadWatches, loadPosts, showToast]);

  const handleRemove = useCallback(async (id: string) => {
    try {
      await window.ipcRenderer.monitor.watchRemove(id);
      setSelectedWatchId((prev) => (prev === id ? null : prev));
      await Promise.all([loadWatches(), loadPosts()]);
    } catch (e) {
      console.error('remove failed:', e);
      showToast('err', e instanceof Error ? e.message : '删除失败');
    }
  }, [loadWatches, loadPosts, showToast]);

  const handleClear = useCallback(async () => {
    const ok = window.confirm(
      '确定清空全部「对标博主」和「全部动态」数据吗？\n此操作不可恢复。',
    );
    if (!ok) return;
    try {
      const result = (await window.ipcRenderer.monitor.clear()) as {
        success?: boolean;
        removedWatchlist?: number;
        removedPosts?: number;
        error?: string;
      };
      // 同上：IPC 失败会被吞成 fallback，不检查 success 会误报「已清空 0 条」。
      if (result && result.success === false) {
        showToast('err', result.error || '清空失败（该操作可能仅管理员可用）');
        return;
      }
      setSelectedWatchId(null);
      await Promise.all([loadWatches(), loadPosts()]);
      showToast(
        'ok',
        `已清空：对标博主 ${result?.removedWatchlist ?? 0} 个、动态 ${result?.removedPosts ?? 0} 条`,
      );
    } catch (e) {
      console.error('clear failed:', e);
      showToast('err', e instanceof Error ? e.message : '清空失败');
    }
  }, [loadWatches, loadPosts, showToast]);

  const handlePostRemove = useCallback(async (postId: string) => {
    try {
      await window.ipcRenderer.monitor.postRemove(postId);
      await loadPosts();
    } catch (e) {
      console.error('post remove failed:', e);
      showToast('err', e instanceof Error ? e.message : '删除动态失败');
    }
  }, [loadPosts, showToast]);

  const handleClearPosts = useCallback(async () => {
    const ok = window.confirm('确定清空全部「动态」数据吗？\n此操作不可恢复（对标博主列表不受影响）。');
    if (!ok) return;
    try {
      const result = (await window.ipcRenderer.monitor.clearPosts()) as {
        success?: boolean;
        removedPosts?: number;
        error?: string;
      };
      // IPC 失败会被桥接层吞成 fallback（removedPosts=0），不检查就会显示
      // 「已清空 0 条动态」—— 让人以为清空成功，其实什么都没发生。
      // 典型场景：web 端清空是管理员专属操作，普通用户会被拒。
      if (result && result.success === false) {
        showToast('err', result.error || '清空动态失败（该操作可能仅管理员可用）');
        return;
      }
      await loadPosts();
      showToast('ok', `已清空 ${result?.removedPosts ?? 0} 条动态`);
    } catch (e) {
      console.error('clear posts failed:', e);
      showToast('err', e instanceof Error ? e.message : '清空动态失败');
    }
  }, [loadPosts, showToast]);

  // 本地详情：首次点开时后端经 CDP 抓正文（约 10-20s），之后走缓存秒开。
  const openDetail = useCallback(async (post: MonitoredPost, refresh = false) => {
    // 切换帖子时清空上一条的拆解结果，避免串味。
    setDeconResult(null);
    setDeconError(null);
    // 同理清空上一条的视频分析（下载态 / 错误 / 分析结果都跟着 detailPost 走）。
    setVideoAnalysis(null);
    setVideoError(null);
    // 已抓取过（正文有 or 已标记 detailFetched）且非主动刷新 → 直接本地展示，秒开、不请求后端。
    // 纯图/无正文笔记 content 永远空，但 detailFetched 为真 → 同样命中缓存，不再反复重抓。
    // 图片列表已随详情抓取落库（post.images）；老数据没图时用封面兜底，避免二次打开空白。
    const hasCachedBody = Boolean(post.content && post.content.trim());
    // 纯图/无正文笔记没有 body，但有图也算有效缓存（避免每次打开都重抓 10-20s）。
    const hasCachedVisual = Boolean(
      post.detailFetched && (((post.images && post.images.length) > 0) || post.coverUrl),
    );
    console.log(
      `[monitor] openDetail id=${post.id} contentLen=${post.content ? post.content.length : 0} ` +
        `detailFetched=${String(post.detailFetched)} images=${post.images ? post.images.length : 0} ` +
        `hasCachedBody=${hasCachedBody} hasCachedVisual=${hasCachedVisual}`,
    );
    if (!refresh && (hasCachedBody || hasCachedVisual)) {
      setDetailPost(post);
      setDetailImages(
        Array.isArray(post.images) && post.images.length
          ? post.images
          : post.coverUrl
            ? [post.coverUrl]
            : [],
      );
      setDetailNote(null);
      setDetailOpen(true);
      setDetailLoading(false);
      return;
    }
    if (!refresh) {
      // 非刷新：先打开面板并展示已有缓存（图片/封面兜底），避免二次打开空白，再异步覆盖。
      setDetailPost(post);
      setDetailImages(
        Array.isArray(post.images) && post.images.length
          ? post.images
          : post.coverUrl
            ? [post.coverUrl]
            : [],
      );
      setDetailNote(null);
      setDetailOpen(true);
    }
    setDetailLoading(true);
    try {
      const result = (await window.ipcRenderer.monitor.postDetail(post.id, refresh)) as {
        success?: boolean;
        post?: MonitoredPost;
        fetched?: boolean;
        images?: string[] | null;
        note?: string;
      };
      // 优先用后端回传的 post；失败回退到入参 post（保留当前展示）。
      // 防回归：后端这次没拿到正文（fetched:false / 超时）时，绝不能把已有正文覆盖成空。
      const incomingPost = result?.post ?? post;
      const keepOldBody = !incomingPost.content?.trim() && post.content?.trim();
      const mergedPost = keepOldBody
        ? { ...incomingPost, content: post.content, detailFetched: post.detailFetched ?? incomingPost.detailFetched }
        : incomingPost;
      console.log(
        `[monitor] openDetail result id=${post.id} fetched=${String(result?.fetched)} ` +
          `note=${JSON.stringify(String(result?.note || ''))} ` +
          `beforeLen=${post.content ? post.content.length : 0} ` +
          `afterLen=${mergedPost.content ? mergedPost.content.length : 0}`,
      );
      setDetailPost(mergedPost);
      // 抓到有效图片就用新的；否则保留现有缩略图（prefill 已兜底 cover/image），不清空。
      const incomingImages = Array.isArray(result?.images) ? result.images : [];
      if (incomingImages.length) setDetailImages(incomingImages);
      setDetailNote(result?.note || null);
      // 回写列表状态：让卡片带上正文+图片，下次直接命中客户端缓存，无需再进后端。
      // 注意：刷新（refresh=true）也要回写，否则二次打开仍读不到新抓到的图。
      if (result?.post) {
        setPosts((prev) =>
          prev.map((p) =>
            p.id === mergedPost.id
              ? {
                  ...p,
                  ...mergedPost,
                  // 同样保护：新结果正文为空时保留列表原有的正文。
                  content: mergedPost.content?.trim() ? mergedPost.content : p.content,
                }
              : p,
          ),
        );
      }
    } catch (e) {
      console.error('post detail failed:', e);
      // 抓取失败：保留当前 detailImages，仅提示，避免清空缩略图。
      setDetailNote(e instanceof Error ? e.message : '获取详情失败');
    } finally {
      setDetailLoading(false);
    }
  }, []);

  const handleAdd = useCallback(async () => {
    const name = addForm.name.trim();
    if (!name) {
      setAddError('请输入博主名称');
      return;
    }
    if (!addForm.platform) {
      setAddError('请选择平台');
      return;
    }
    const profileUrl = addForm.profileUrl.trim();
    if (!profileUrl) {
      setAddError('请输入主页链接');
      return;
    }
    try {
      const result = (await window.ipcRenderer.monitor.watchAdd({
        name,
        platform: addForm.platform,
        profileUrl,
        note: addForm.note.trim() || undefined,
        // 监控统一由凌晨增量任务跑：enabled / interval_hours 取后端默认值（true / 24）
      })) as { success?: boolean; error?: string } | undefined;
      // 后端失败（比如同一博主重复添加）会带上具体原因，必须展示出来 ——
      // 否则用户点了「添加」却什么都没发生，也不知道原因。
      if (result && result.success === false) {
        setAddError(result.error || '添加失败');
        return;
      }
      setAddOpen(false);
      setAddForm({ name: '', platform: '', profileUrl: '', note: '' });
      setAddError('');
      await loadWatches();
      showToast('ok', `已添加监控「${name}」`);
    } catch (e) {
      console.error('add failed:', e);
      setAddError(e instanceof Error ? e.message : '添加失败');
    }
  }, [addForm, loadWatches, showToast]);

  const handleChromeLaunch = useCallback(async () => {
    setChromeBusy(true);
    setChromeTestResult(null);
    try {
      const url = PLATFORM_HOME[chromeLoginPlatform];
      const result = (await window.ipcRenderer.monitor.chromeLaunch({
        platform: chromeLoginPlatform,
        url,
      })) as MonitorChromeLaunchResult;
      if (result.success) {
        showToast('ok', `已用 Chrome 打开登录页，请在浏览器中登录${platformLabel(chromeLoginPlatform)}`);
      } else {
        showToast('err', '启动 Chrome 失败');
      }
      await loadChromeStatus();
    } catch (e) {
      console.error('chrome launch failed:', e);
      showToast('err', e instanceof Error ? e.message : '启动 Chrome 失败');
    } finally {
      setChromeBusy(false);
    }
  }, [chromeLoginPlatform, loadChromeStatus, showToast]);


  const handleChromeLogout = useCallback(async () => {
    try {
      // 必须带 platform：后端要删掉该平台的 Chrome profile（cookie 在里面），
      // 只关进程的话下次启动还是登录状态。
      const result = await window.ipcRenderer.monitor.chromeLogout(chromeLoginPlatform);
      setChromeTestResult(null);
      setPlatformLoginState(false);
      setLoginDebug(null);
      showToast(result?.cleared === false ? 'err' : 'ok', result?.note || '已退出登录');
      await loadChromeStatus();
    } catch (e) {
      console.error('chrome logout failed:', e);
      showToast('err', e instanceof Error ? e.message : '退出失败');
    }
  }, [chromeLoginPlatform, loadChromeStatus, showToast]);

  const handleInject = useCallback(async (post: MonitoredPost) => {
    try {
      const result = (await window.ipcRenderer.monitor.injectTopic({ ids: [post.id] })) as {
        success?: boolean;
        count?: number;
      };
      await loadPosts();
      showToast('ok', `已加入选题（${result?.count ?? 1} 条）`);
    } catch (e) {
      console.error('inject failed:', e);
      showToast('err', e instanceof Error ? e.message : '加入选题失败');
    }
  }, [loadPosts, showToast]);

  const handleInjectAll = useCallback(async () => {
    const pending = posts.filter((p) => !p.topicInjected).map((p) => p.id);
    if (pending.length === 0) {
      showToast('err', '没有可加入的未选动态');
      return;
    }
    try {
      const result = (await window.ipcRenderer.monitor.injectTopic({ ids: pending })) as {
        success?: boolean;
        count?: number;
      };
      await loadPosts();
      showToast('ok', `已批量加入选题（${result?.count ?? pending.length} 条）`);
    } catch (e) {
      console.error('inject all failed:', e);
      showToast('err', e instanceof Error ? e.message : '批量加入失败');
    }
  }, [posts, loadPosts, showToast]);

  // 拆解一篇对标帖：调后端 LLM 拆解并落库，结果写入 deconResult 供详情弹窗展示。
  const runDeconstruct = useCallback(async (post: MonitoredPost) => {
    if (!post.content || !post.content.trim()) {
      setDeconError('该帖本地暂无正文，请先在详情里「重新抓正文」后再拆解');
      return;
    }
    setDeconLoading(true);
    setDeconError(null);
    try {
      const result = (await window.ipcRenderer.monitor.deconstruct(post.id)) as {
        success?: boolean;
        error?: string;
        deconstruction?: Record<string, unknown>;
      };
      if (!result?.success || !result.deconstruction) {
        setDeconError(result?.error || '拆解失败');
        return;
      }
      const decon = normalizeDecon(result.deconstruction);
      setDeconResult(decon);
      // 回写卡片：标记该帖已拆解，下次列表里直接带「已拆解」徽标。
      setPosts((prev) => prev.map((p) => (p.id === post.id ? { ...p, deconstructed: true } : p)));
      // 同步详情对象：重新打开同一帖子时（deconResult 被清空）能经 effect 自动取回沉淀。
      setDetailPost((prev) => (prev && prev.id === post.id ? { ...prev, deconstructed: true } : prev));
    } catch (e) {
      console.error('deconstruct failed:', e);
      setDeconError(e instanceof Error ? e.message : '拆解失败');
    } finally {
      setDeconLoading(false);
    }
  }, [normalizeDecon]);

  // 方法论库：加载已沉淀的爆文拆解列表（可按作者 / 平台过滤）。
  const loadMethods = useCallback(async () => {
    setMethodsLoading(true);
    try {
      const result = (await window.ipcRenderer.monitor.deconList({
        author: methodsAuthor || undefined,
        platform: methodsPlatform || undefined,
      })) as {
        success?: boolean;
        items?: Record<string, unknown>[];
        authors?: string[];
      };
      const items = Array.isArray(result?.items) ? (result.items.map((it) => normalizeDecon(it)).filter(Boolean) as PostDeconstruction[]) : [];
      // createdAt 倒序（最新沉淀在前）
      items.sort((a, b) => b.createdAt - a.createdAt);
      setMethodsItems(items);
      setMethodsAuthors(Array.isArray(result?.authors) ? (result.authors as string[]) : []);
    } catch (e) {
      console.error('decon list failed:', e);
      setMethodsItems([]);
      setMethodsAuthors([]);
    } finally {
      setMethodsLoading(false);
    }
  }, [methodsAuthor, methodsPlatform, normalizeDecon]);

  // 方法论库模态内：过滤条件变化或重新打开时，自动重载列表。
  useEffect(() => {
    if (methodsOpen) void loadMethods();
  }, [methodsOpen, methodsAuthor, methodsPlatform, loadMethods]);

  // 打开详情时，若该帖已被标记为「已拆解」，自动从方法论库取回沉淀结果展示（不重复调 LLM）。
  useEffect(() => {
    if (!detailPost || !detailPost.deconstructed) return;
    if (deconResult && deconResult.postId === detailPost.id) return;
    let cancelled = false;
    (async () => {
      try {
        const result = (await window.ipcRenderer.monitor.deconList({
          author: detailPost.author,
          platform: detailPost.platform,
        })) as { items?: Record<string, unknown>[] };
        if (cancelled) return;
        const found = (result?.items || [])
          .map((it) => normalizeDecon(it))
          .find((d) => d && d.postId === detailPost.id);
        if (found) setDeconResult(found);
      } catch (e) {
        console.warn('load existing decon failed:', e);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [detailPost, deconResult, normalizeDecon]);

  // 下载视频本体：后端 CDP 现抽签名直链 → ffmpeg 落盘到 workspace/monitor-videos/<post_id>.mp4。
  // 成功后回写卡片状态并让详情对象同步（否则再次打开详情仍是旧状态）。
  const runDownloadVideo = useCallback(async (post: MonitoredPost) => {
    setVideoBusyId(post.id);
    setVideoError(null);
    // 乐观置为「下载中」，让卡片立刻出 loading 态
    setPosts((prev) => prev.map((p) => (p.id === post.id ? { ...p, videoStatus: 'downloading' } : p)));
    setDetailPost((prev) => (prev && prev.id === post.id ? { ...prev, videoStatus: 'downloading' } : prev));
    try {
      const result = (await window.ipcRenderer.monitor.downloadVideo(post.id)) as {
        success?: boolean;
        localPath?: string;
        previewPath?: string | null;
        size?: number;
        hasAudio?: boolean;
        audioWarning?: string | null;
        error?: string;
        note?: string;
      };
      if (!result?.success) {
        const msg = result?.error || result?.note || '下载视频失败';
        setVideoError(msg);
        setPosts((prev) => prev.map((p) => (p.id === post.id ? { ...p, videoStatus: 'failed', videoError: msg } : p)));
        setDetailPost((prev) => (prev && prev.id === post.id ? { ...prev, videoStatus: 'failed', videoError: msg } : prev));
        return;
      }
      const size = Number(result.size ?? 0);
      const preview = result.previewPath ?? null;
      // 后端会在下载后探测音轨（抖音 DASH 只给视频轨时会补下音频再合并），
      // 这里必须把 hasAudio / 下载时间一起写回本地状态 ——
      // 漏写会导致「明明重新下载成功了，界面还挂着『没有音轨』的黄条 + 旧的下载时间」。
      const hasAudio = result.hasAudio;
      const downloadedAt = Date.now();
      setPosts((prev) =>
        prev.map((p) =>
          p.id === post.id
            ? {
                ...p,
                videoStatus: 'done',
                videoLocalPath: result.localPath ?? p.videoLocalPath ?? null,
                videoPreviewPath: preview,
                videoSize: size,
                videoError: null,
                videoDownloadedAt: downloadedAt,
                ...(typeof hasAudio === 'boolean' ? { videoHasAudio: hasAudio } : {}),
              }
            : p,
        ),
      );
      setDetailPost((prev) =>
        prev && prev.id === post.id
          ? {
              ...prev,
              videoStatus: 'done',
              videoLocalPath: result.localPath ?? prev.videoLocalPath ?? null,
              videoPreviewPath: preview,
              videoSize: size,
              videoError: null,
              videoDownloadedAt: downloadedAt,
              ...(typeof hasAudio === 'boolean' ? { videoHasAudio: hasAudio } : {}),
            }
          : prev,
      );
      if (hasAudio === false) {
        showToast('err', result.audioWarning || '视频已下载，但仍未取到音轨（页面未提供音频地址）');
      } else {
        showToast('ok', preview ? '视频已下载（已生成轻量播放版）' : '视频已下载到本地');
      }
    } catch (e) {
      console.error('download video failed:', e);
      const msg = e instanceof Error ? e.message : '下载视频失败';
      setVideoError(msg);
      setPosts((prev) => prev.map((p) => (p.id === post.id ? { ...p, videoStatus: 'failed', videoError: msg } : p)));
      setDetailPost((prev) => (prev && prev.id === post.id ? { ...prev, videoStatus: 'failed', videoError: msg } : prev));
    } finally {
      setVideoBusyId(null);
    }
  }, [showToast]);

  // 分析已下载的视频：抽帧 + ASR 转写（+ 预留 LLM 摘要），结果写回 videoAnalysis。
  const runAnalyzeVideo = useCallback(async (post: MonitoredPost, skipTranscript = false) => {
    setVideoAnalyzingId(post.id);
    setVideoError(null);
    try {
      const result = (await window.ipcRenderer.monitor.analyzeVideo(post.id, { skipTranscript })) as {
        success?: boolean;
        analysis?: Record<string, unknown>;
        error?: string;
        note?: string;
      };
      if (!result?.success) {
        setVideoError(result?.error || result?.note || '分析视频失败');
        return;
      }
      setVideoAnalysis(normalizeVideoAnalysis(result.analysis));
    } catch (e) {
      console.error('analyze video failed:', e);
      setVideoError(e instanceof Error ? e.message : '分析视频失败');
    } finally {
      setVideoAnalyzingId(null);
    }
  }, []);

  // 打开详情时若该帖已下载过视频，自动取回已沉淀的分析结果（不重复跑 ffmpeg/ASR）。
  useEffect(() => {
    if (!detailPost || detailPost.videoStatus !== 'done') return;
    if (videoAnalysis && videoAnalysis.postId === detailPost.id) return;
    let cancelled = false;
    (async () => {
      try {
        const result = (await window.ipcRenderer.monitor.videoAnalysis(detailPost.id)) as {
          success?: boolean;
          analysis?: Record<string, unknown> | null;
        };
        if (cancelled) return;
        const found = normalizeVideoAnalysis(result?.analysis);
        if (found) setVideoAnalysis(found);
      } catch (e) {
        console.warn('load existing video analysis failed:', e);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [detailPost, videoAnalysis]);

  const selectedWatch = watches.find((w) => w.id === selectedWatchId) || null;
  const uninjectedCount = posts.filter((p) => !p.topicInjected).length;

  return (
    <div className="flex h-full">
      {/* 监控列表 - 左栏 */}
      <div className="w-72 border-r border-border bg-surface-secondary/30 flex flex-col">
        <div className="p-4 border-b border-border flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Radar className="w-4 h-4 text-accent-primary" strokeWidth={1.8} />
            <h2 className="text-sm font-semibold text-text-primary">对标博主</h2>
            <span className="text-[11px] text-text-tertiary">{watches.length}</span>
          </div>
          <button
            onClick={() => { setAddError(''); setAddOpen(true); }}
            className="p-1.5 text-text-tertiary hover:text-accent-primary hover:bg-surface-primary rounded transition-colors"
            title="添加监控"
          >
            <Plus className="w-4 h-4" />
          </button>
          <button
            onClick={() => void handleClear()}
            disabled={watches.length === 0 && posts.length === 0}
            className="p-1.5 text-text-tertiary hover:text-red-500 hover:bg-surface-primary rounded transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
            title="清空对标博主与全部动态"
          >
            <Trash2 className="w-4 h-4" />
          </button>
        </div>

        {/* 登录管理 - Chrome 头浏览器（小红书 / 抖音登录态抓取） */}
        <div className="mx-2 mt-2 rounded-lg border border-border bg-surface-primary overflow-hidden">
          <button
            onClick={() => setChromeOpen((prev) => !prev)}
            className="w-full flex items-center justify-between px-3 py-2.5 text-left hover:bg-surface-secondary/50 transition-colors"
          >
            <div className="flex items-center gap-2">
              <Lock className="w-4 h-4 text-accent-primary" strokeWidth={1.8} />
              <span className="text-sm font-semibold text-text-primary">登录管理</span>
              {/* 普通用户只看平台登录态；「已就绪/未就绪」是 Chrome 调试端口的术语，只给管理员看 */}
              {isAdminUser && (chromeStatus?.debugReady ? (
                <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-emerald-50 text-emerald-600">已就绪</span>
              ) : (
                <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-surface-secondary text-text-tertiary">未就绪</span>
              ))}
              {/* 登录态 ≠ Chrome 就绪：Chrome 起着但平台没登录时，抓取只能拿游客首屏几条 */}
              {platformLoginState === false && (
                <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-amber-50 text-amber-600">平台未登录</span>
              )}
              {platformLoginState === true && (
                <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-emerald-50 text-emerald-600">已登录</span>
              )}
            </div>
            <ChevronDown className={clsx('w-4 h-4 text-text-tertiary transition-transform', chromeOpen && 'rotate-180')} />
          </button>

          {chromeOpen && (
            <div className="px-3 pb-3 pt-1 space-y-3">
              {/* 普通 web 用户只给状态 + 一句人话：扫码/cookie/诊断这些技术操作
                  全部收进管理员视图 —— 用户端的人不需要、也不该接触这些。 */}
              {!isAdminUser ? (
                <div className="space-y-1.5 text-[11px] leading-relaxed">
                  <div className="flex items-center gap-2">
                    <span className="text-text-secondary">平台账号</span>
                    {platformLoginState === true ? (
                      <span className="text-emerald-600">已登录</span>
                    ) : (
                      <span className="text-amber-600">未登录（管理员在处理）</span>
                    )}
                  </div>
                  <p className="text-text-tertiary">
                    抓取用的平台账号由管理员在服务器上统一维护，
                    <b className="text-text-secondary">你不需要登录任何平台账号</b>。
                    未登录时只能抓到游客可见的少量笔记，之后会由管理员补上。
                  </p>
                  <p className="text-text-tertiary">当前平台：{platformLabel(chromeLoginPlatform)}</p>
                </div>
              ) : (
                <>
              <div className="text-[11px] leading-relaxed text-text-tertiary">
                {chromeStatus === null ? '正在检测本机 Chrome…' : (
                  chromeStatus.chromeInstalled ? (
                    <>{platformLabel(chromeLoginPlatform)} Chrome
                      {chromeStatus.debugReady ? (
                        <> · 调试端口 <span className="text-emerald-600">{chromeStatus.debugPort}</span>（已就绪）</>
                      ) : (
                        <span className="text-amber-600">
                          {' · 未启动'}
                          {isTauriRuntime()
                            ? '，请点下面「打开 Chrome 登录」'
                            : '，可打开 /admin/qr 扫码登录'}
                        </span>
                      )}</>
                  ) : (
                    <span className="text-amber-600">未检测到 Chrome / Edge / Brave，请先安装 Google Chrome</span>
                  )
                )}
              </div>

              <p className="text-[11px] leading-relaxed text-text-tertiary">
                小红书 / 抖音 / B 站需要登录态抓取。
                {isTauriRuntime()
                  ? '先选平台点「打开登录页」，在弹出的 Chrome 内完成登录（cookie 仅存于本机独立 profile）；也可点「扫码登录」用手机 App 扫。之后 Beav 复用登录态抓取。'
                  : '点「扫码登录」用手机 App 扫服务器上的二维码即可（服务器上的账号对所有用户共享）。'}
              </p>

              <div className="flex gap-1.5">
                {CDP_PLATFORMS.map((p) => (
                  <button
                    key={p}
                    onClick={() => {
                      // 只切换平台，不弹扫码窗（扫码入口是下面那个明确的「扫码登录」按钮）。
                      // 之前点页签就弹窗，用户反馈「点 3 个平台按钮就出现弹窗」。
                      setChromeLoginPlatform(p);
                    }}
                    className={clsx(
                      'flex-1 text-xs px-2 py-1.5 rounded-md border transition-colors',
                      chromeLoginPlatform === p
                        ? 'bg-accent-primary/10 text-accent-primary border-accent-primary/30'
                        : 'border-border text-text-secondary hover:text-text-primary',
                    )}
                  >
                    {platformLabel(p)}
                  </button>
                ))}
              </div>

              <div className="flex flex-col gap-1.5">
                {isTauriRuntime() && (
                  <button
                    onClick={() => void handleChromeLaunch()}
                    disabled={chromeBusy || !chromeStatus?.chromeInstalled}
                    className="w-full inline-flex items-center justify-center gap-1.5 px-3 py-2 text-sm rounded-lg bg-accent-primary text-white hover:bg-accent-primary/90 transition-colors disabled:opacity-50"
                  >
                    {chromeBusy ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Lock className="w-3.5 h-3.5" />}
                    打开 Chrome 登录{platformLabel(chromeLoginPlatform)}
                  </button>
                )}

                <div className="flex gap-1.5">
                  <button
                    onClick={() => void checkPlatformLogin(true)}
                    className="flex-1 inline-flex items-center justify-center gap-1.5 px-2 py-1.5 text-[11px] rounded-md border border-border text-text-secondary hover:text-accent-primary hover:border-accent-primary/40 transition-colors"
                  >
                    <RefreshCw className="w-3 h-3" />检测登录状态
                  </button>
                  {/* 桌面端不用扫码：它自己就有「打开 Chrome 登录」，扫码只是冗余入口 */}
                  {!isTauriRuntime() && (
                    <button
                      onClick={() => openQrModal(chromeLoginPlatform)}
                      className="flex-1 inline-flex items-center justify-center gap-1.5 px-2 py-1.5 text-[11px] rounded-md border border-border text-text-secondary hover:text-accent-primary hover:border-accent-primary/40 transition-colors"
                    >
                      <Lock className="w-3 h-3" />扫码登录
                    </button>
                  )}
                  {isAdminUser && (
                    <button
                      onClick={() => {
                        void (async () => {
                          try {
                            const result = await window.ipcRenderer.monitor.chromeLoginDebug(chromeLoginPlatform);
                            setLoginDebug(JSON.stringify(result, null, 2));
                          } catch (e) {
                            setLoginDebug(`诊断失败：${String(e)}`);
                          }
                        })();
                      }}
                      className="flex-1 inline-flex items-center justify-center gap-1.5 px-2 py-1.5 text-[11px] rounded-md border border-border text-text-secondary hover:text-accent-primary hover:border-accent-primary/40 transition-colors"
                      title="取回服务器 Chrome 里该平台页面的真实登录状态"
                    >
                      <Eye className="w-3 h-3" />诊断
                    </button>
                  )}
                  {isAdminUser && (
                    <button
                      onClick={() => void handleChromeLogout()}
                      disabled={!chromeStatus?.debugReady}
                      className="flex-1 inline-flex items-center justify-center gap-1.5 px-2 py-1.5 text-[11px] rounded-md border border-border text-text-secondary hover:text-red-500 hover:border-red-200 transition-colors disabled:opacity-50"
                    >
                      <LogOut className="w-3 h-3" />退出登录
                    </button>
                  )}
                </div>

                {/* 平台登录态：主要靠管理员在服务器上登录，web 端只做检测与展示 */}
                {platformLoginState === false && (
                  <div className="rounded-md border border-amber-200 bg-amber-50/60 px-2.5 py-2 text-[11px] leading-relaxed text-amber-800">
                    平台未登录。请在<b>服务器桌面端</b>完成登录：
                    <br />
                    桌面端 → 对标监控 → 登录管理 → <b>打开 Chrome 登录</b>
                    <br />
                    登录完成后点上面「检测登录状态」即可（web 端无需扫码）。
                    <button
                      onClick={() => {
                        void (async () => {
                          try {
                            const result = await window.ipcRenderer.monitor.chromeLoginDebug(chromeLoginPlatform);
                            setLoginDebug(JSON.stringify(result, null, 2));
                          } catch (e) {
                            setLoginDebug(`诊断失败：${String(e)}`);
                          }
                        })();
                      }}
                      className="mt-1.5 underline decoration-dotted hover:text-amber-900"
                    >
                      查看登录诊断
                    </button>
                  </div>
                )}
                {loginDebug && (
                  <div className="rounded-md border border-border bg-surface-secondary/60 px-2 py-1.5">
                    <div className="flex items-center justify-between mb-1">
                      <span className="text-[10px] text-text-tertiary">登录诊断（可复制发我）</span>
                      <button
                        onClick={() => {
                          void navigator.clipboard.writeText(loginDebug).then(
                            () => showToast('ok', '诊断已复制'),
                            () => showToast('err', '复制失败'),
                          );
                        }}
                        className="text-[10px] text-accent-primary hover:underline"
                      >
                        复制
                      </button>
                    </div>
                    <pre className="text-[10px] leading-relaxed text-text-secondary whitespace-pre-wrap break-all max-h-48 overflow-auto">
                      {loginDebug}
                    </pre>
                  </div>
                )}
                {platformLoginState === true && (
                  <div className="text-[11px] text-emerald-600">平台已登录，抓取会使用该账号的完整视角。</div>
                )}

                {/* 登录态迁移：平台对新设备扫码会风控验证，扫码走不通时用这个 */}
                <details className="rounded-md border border-border bg-surface-secondary/40 px-2.5 py-1.5">
                  <summary className="text-[11px] cursor-pointer text-text-secondary hover:text-text-primary">
                    扫码过不去？用「登录态迁移」（绕开风控）
                  </summary>
                  <div className="mt-2 space-y-2 text-[11px] leading-relaxed text-text-tertiary">
                    <div>
                      1. 在<b className="text-text-secondary">已登录的机器</b>（比如你自己的电脑）点「导出」，
                      把复制的 JSON 发到这台服务器；
                      <br />
                      2. 在这里粘贴 JSON 后点「导入」；
                      <br />
                      3. 点「检测登录状态」确认。
                    </div>
                    <div className="flex gap-1.5">
                      <button
                        onClick={() => {
                          void (async () => {
                            try {
                              const result = await window.ipcRenderer.monitor.chromeExportCookies(chromeLoginPlatform);
                              if (!result?.count) {
                                showToast('err', '没导出到 cookie：这台机器的 Chrome 可能未启动或未登录');
                                return;
                              }
                              await navigator.clipboard.writeText(JSON.stringify(result));
                              showToast('ok', `已复制 ${result.count} 条 cookie，贴到另一台机器的导入框`);
                            } catch (e) {
                              showToast('err', `导出失败：${String(e)}`);
                            }
                          })();
                        }}
                        className="flex-1 px-2 py-1 rounded border border-border hover:text-accent-primary hover:border-accent-primary/40 transition-colors"
                      >
                        导出到剪贴板
                      </button>
                      <button
                        onClick={() => {
                          void (async () => {
                            try {
                              const text = await navigator.clipboard.readText();
                              const parsed = JSON.parse(text) as { cookies?: unknown[] };
                              const cookies = Array.isArray(parsed) ? parsed : parsed?.cookies;
                              if (!Array.isArray(cookies) || !cookies.length) {
                                showToast('err', '剪贴板里没有有效的 cookie JSON');
                                return;
                              }
                              const result = await window.ipcRenderer.monitor.chromeImportCookies(
                                chromeLoginPlatform,
                                cookies,
                              );
                              if (result?.success) {
                                showToast('ok', `已导入 ${result.imported} 条 cookie，正在检测登录状态…`);
                                await checkPlatformLogin(true);
                              } else {
                                showToast('err', '导入失败');
                              }
                            } catch (e) {
                              showToast('err', `导入失败：${String(e)}`);
                            }
                          })();
                        }}
                        className="flex-1 px-2 py-1 rounded border border-border hover:text-accent-primary hover:border-accent-primary/40 transition-colors"
                      >
                        从剪贴板导入
                      </button>
                    </div>
                    <div className="text-[10px] text-amber-700/80">
                      ⚠️ cookie 等同登录凭证，只在两台自己的机器之间传递，别发到公开地方。
                    </div>
                  </div>
                </details>
              </div>

              {chromeTestResult && (
                <div className={clsx(
                  'text-[11px] rounded-md px-2 py-1.5 border',
                  chromeTestResult.startsWith('连接成功')
                    ? 'bg-emerald-50 text-emerald-700 border-emerald-200'
                    : 'bg-red-50 text-red-600 border-red-200',
                )}>
                  {chromeTestResult}
                </div>
              )}
                </>
              )}
            </div>
          )}
        </div>

        <div className="flex-1 overflow-auto p-2 space-y-1">
          <button
            onClick={() => setSelectedWatchId(null)}
            className={clsx(
              'w-full text-left px-3 py-2.5 rounded-lg transition-colors text-sm',
              selectedWatchId === null
                ? 'bg-accent-primary/10 text-accent-primary border border-accent-primary/30'
                : 'hover:bg-surface-primary text-text-primary',
            )}
          >
            <div className="flex items-center gap-2">
              <Eye className="w-4 h-4 shrink-0" strokeWidth={1.7} />
              <span className="font-medium truncate">全部动态</span>
            </div>
            <div className="text-xs text-text-tertiary mt-0.5 truncate">查看所有抓取到的帖子</div>
          </button>

          {watchesError && watches.length === 0 ? (
            <div className="text-center text-xs py-6 px-3">
              <p className="text-red-500 mb-2">加载监控列表失败</p>
              <p className="text-text-tertiary break-words mb-3">{watchesError}</p>
              <button
                onClick={() => void loadWatches()}
                className="inline-flex items-center gap-1 px-2.5 py-1 text-[11px] rounded-md border border-border text-text-secondary hover:text-accent-primary hover:border-accent-primary/40"
              >
                <RefreshCw className="w-3 h-3" />重试
              </button>
            </div>
          ) : loadingWatches && watches.length === 0 ? (
            <div className="text-center text-text-tertiary text-xs py-8">加载中...</div>
          ) : watches.length === 0 ? (
            <div className="text-center text-text-tertiary text-xs py-8">
              <Radar className="w-8 h-8 mx-auto mb-2 opacity-30" />
              <p>还没有监控对象</p>
              <button onClick={() => { setAddError(''); setAddOpen(true); }} className="mt-2 text-accent-primary hover:underline">
                添加一个博主
              </button>
            </div>
          ) : (
            watches.map((watch) => (
              <div
                key={watch.id}
                className={clsx(
                  'group relative w-full text-left px-3 py-2.5 rounded-lg transition-colors border',
                  selectedWatchId === watch.id
                    ? 'bg-accent-primary/10 border-accent-primary/30'
                    : 'border-transparent hover:bg-surface-primary',
                )}
              >
                <button
                  onClick={() => setSelectedWatchId(watch.id)}
                  className="w-full text-left pr-7"
                >
                  <div className="flex items-center gap-2">
                    <span className={clsx(
                      'w-1.5 h-1.5 rounded-full shrink-0',
                      watch.enabled ? 'bg-emerald-500' : 'bg-text-tertiary/40',
                    )} />
                    <span className="text-sm font-medium text-text-primary truncate flex-1">{watch.name}</span>
                    {watch.platform && (
                      <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-surface-primary text-text-tertiary shrink-0">
                        {platformLabel(watch.platform)}
                      </span>
                    )}
                  </div>
                  <div className="text-[11px] text-text-tertiary mt-1 truncate">
                    上次抓取：{formatTs(watch.lastFetchAt)} · 已收录 {watch.postCount ?? posts.filter((p) => p.watchId === watch.id).length} 篇
                  </div>
                  {/* 补历史进行中：实时进度条（已抓 X / 共 Y）。
                      分母 Y 滚动阶段用主页声明的作品总数；翻到底后后端会发 phase=scrolling-done
                      把分母修正为「实际可抓总量」（声明数常含私密/已删/审核中不渲染的条目，
                      否则进度会永远停在 73/87）。
                      phase=videos：正文/列表已抓完，后台正在并行「下载+转写」历史视频。 */}
                  {(historyBusyId === watch.id || backfillProgress[watch.id]?.phase === 'videos') ? (
                    <div className="mt-1">
                      <div className="h-1.5 w-full rounded-full bg-surface-primary overflow-hidden">
                        {(() => {
                          const prog = backfillProgress[watch.id];
                          const total = prog?.total ?? 0;
                          const pct = total > 0 ? Math.min(100, Math.round(((prog?.scanned ?? 0) / total) * 100)) : 0;
                          return (
                            <div
                              className={clsx(
                                'h-full rounded-full transition-all duration-300',
                                total > 0 ? 'bg-accent-primary' : 'bg-accent-primary/60 animate-pulse',
                              )}
                              style={{ width: total > 0 ? `${pct}%` : '45%' }}
                            />
                          );
                        })()}
                      </div>
                      <div className="text-[10px] text-text-tertiary mt-0.5 truncate">
                        {(() => {
                          const prog = backfillProgress[watch.id];
                          const total = prog?.total ?? 0;
                          if (prog?.phase === 'videos') {
                            return `历史视频处理中：${prog?.scanned ?? 0}${total > 0 ? ` / 共 ${total}` : ''} 个`;
                          }
                          if (prog?.phase === 'transcribe') {
                            return `语音转写中：${prog?.scanned ?? 0}${total > 0 ? ` / 共 ${total}` : ''} 个`;
                          }
                          return `补历史中：已抓 ${prog?.scanned ?? 0}${total > 0 ? ` / 共 ${total}` : ''} 篇`;
                        })()}
                      </div>
                    </div>
                  ) : (
                    !watch.historyBackfilledAt &&
                    watch.enabled && (
                      <div className="text-[10px] text-amber-600 mt-0.5 truncate">
                        历史未补齐（点右上角补历史）
                      </div>
                    )
                  )}
                </button>
                {/* 补历史：绝对定位在条目右上角。
                    放在 <button> 之外 —— 嵌套 button 是非法 HTML，点击也会被父级抢走。 */}
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    if (historyBusyId !== null) {
                      showToast('err', '有补历史任务正在进行，请稍候再试');
                      return;
                    }
                    void handleBackfillHistory(watch.id);
                  }}
                  aria-disabled={historyBusyId !== null}
                  className={clsx(
                    'absolute right-2 top-2 p-1 rounded transition-colors',
                    historyBusyId !== null
                      ? 'text-text-tertiary opacity-40 cursor-not-allowed'
                      : 'text-text-tertiary hover:text-accent-primary hover:bg-surface-primary',
                  )}
                  title={
                    historyBusyId !== null
                      ? '有补历史任务正在进行，请稍候'
                      : `补历史：把「${watch.name}」主页更早的历史帖抓下来`
                  }
                >
                  {historyBusyId === watch.id ? (
                    <Loader2 className="w-3.5 h-3.5 animate-spin" />
                  ) : (
                    <History className="w-3.5 h-3.5" />
                  )}
                </button>
                {/* 上次补历史的诊断（常驻）：只抓到 N 条时能直接看出是"没滚动"还是"滚不动"。 */}
                {backfillDiag[watch.id] && (
                  <div className={clsx(
                    'mt-1 text-[10px] leading-relaxed',
                    backfillDiag[watch.id].level === 'ok'
                      ? 'text-emerald-700/90 dark:text-emerald-500/90'
                      : 'text-amber-700/90 dark:text-amber-500/90',
                  )}>
                    补历史诊断：{backfillDiag[watch.id].hint}
                    {backfillDiag[watch.id].detail ? `（${backfillDiag[watch.id].detail}）` : ''}
                  </div>
                )}
                <div className="flex items-center gap-1 mt-2 opacity-0 group-hover:opacity-100 transition-opacity">
                  <button
                    onClick={() => {
                      if (fetching) {
                        showToast('err', '有抓取任务正在进行，请稍候再试');
                        return;
                      }
                      void handleFetchWatch(watch);
                    }}
                    aria-disabled={fetching}
                    className={clsx(
                      'flex-1 inline-flex items-center justify-center gap-1 text-[11px] px-2 py-1 rounded-md border border-border text-text-secondary transition-colors',
                      fetching
                        ? 'opacity-50 cursor-not-allowed'
                        : 'hover:text-accent-primary hover:border-accent-primary/40',
                    )}
                    title={fetching ? '有抓取任务正在进行，请稍候' : '抓取该博主'}
                  >
                    <RefreshCw className={clsx('w-3 h-3', fetchingWatchId === watch.id && 'animate-spin')} />
                    抓取
                  </button>
                  <button
                    onClick={() => void handleRemove(watch.id)}
                    className="inline-flex items-center justify-center w-7 h-7 rounded-md text-text-tertiary hover:text-red-500 hover:bg-surface-primary transition-colors"
                    title="删除监控"
                  >
                    <Trash2 className="w-3.5 h-3.5" />
                  </button>
                </div>
              </div>
            ))
          )}
        </div>
      </div>

      {/* 帖子动态 - 右栏 */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* 头部 */}
        <div className="px-6 py-4 border-b border-border">
          <div className="flex items-center justify-between gap-3 flex-wrap">
            <div className="min-w-0">
              <h1 className="text-lg font-semibold text-text-primary flex items-center gap-2">
                {selectedWatch ? (
                  <>
                    {selectedWatch.name}
                    <span className="text-[11px] font-normal px-2 py-0.5 rounded-full bg-surface-secondary text-text-tertiary">
                      {platformLabel(selectedWatch.platform)}
                    </span>
                  </>
                ) : (
                  '全部动态'
                )}
              </h1>
              <p className="text-xs text-text-tertiary mt-0.5">
                {selectedWatch?.profileUrl ? (
                  <button onClick={() => void openExternal(selectedWatch.profileUrl)} className="inline-flex items-center gap-1 hover:text-accent-primary">
                    <Link2 className="w-3 h-3" />主页链接
                  </button>
                ) : '抓取到的对标帖子'}
              </p>
            </div>
            <div className="flex items-center gap-2">
              <button
                onClick={() => { setMethodsAuthor(''); setMethodsPlatform(''); setMethodsOpen(true); }}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-border text-text-secondary hover:text-accent-primary hover:border-accent-primary/40 transition-colors"
                title="浏览已沉淀的爆文拆解方法论库"
              >
                <BookOpen className="w-3.5 h-3.5" />
                方法论库
              </button>
              {uninjectedCount > 0 && (
                <button
                  onClick={() => void handleInjectAll()}
                  className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md bg-accent-primary text-white hover:bg-accent-primary/90 transition-colors"
                >
                  <Sparkles className="w-3.5 h-3.5" />
                  全部加入选题
                </button>
              )}
              <button
                onClick={() => void loadPosts()}
                className="p-1.5 text-text-tertiary hover:text-accent-primary hover:bg-surface-primary rounded transition-colors"
                title="刷新动态"
              >
                <RefreshCw className={clsx('w-4 h-4', loadingPosts && 'animate-spin')} />
              </button>
              <button
                onClick={() => void handleClearPosts()}
                disabled={posts.length === 0}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-border text-text-secondary hover:text-red-500 hover:border-red-200 transition-colors disabled:opacity-40 disabled:cursor-not-allowed"
                title="清空全部动态（不影响对标博主）"
              >
                <Trash2 className="w-3.5 h-3.5" />
                清空动态
              </button>
            </div>
          </div>

          {/* 临时抓取链接 */}
          <div className="mt-3 flex items-center gap-2">
            <input
              value={adhocUrl}
              onChange={(e) => setAdhocUrl(e.target.value)}
              onKeyDown={(e) => e.key === 'Enter' && void handleAdhocFetch()}
              placeholder="粘贴博主主页或单篇帖子链接，临时抓取（不加入监控列表）"
              className="flex-1 bg-surface-secondary border border-border rounded-lg px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-accent-primary"
            />
            <button
              onClick={() => {
                if (fetching) {
                  showToast('err', '有抓取任务正在进行，请稍候再试');
                  return;
                }
                void handleAdhocFetch();
              }}
              aria-disabled={fetching || !adhocUrl.trim()}
              className={clsx(
                'inline-flex items-center gap-1.5 px-3 py-2 text-sm rounded-lg bg-accent-primary text-white transition-colors',
                fetching || !adhocUrl.trim() ? 'opacity-50 cursor-not-allowed' : 'hover:bg-accent-primary/90',
              )}
            >
              {fetchingWatchId === '__adhoc__' ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <RefreshCw className="w-3.5 h-3.5" />}
              抓取
            </button>
          </div>
          {/* 抓取条数已固定，不再开放配置（后端 SCAN_LIMIT=30） */}
          <div className="mt-2 text-[11px] text-text-tertiary">
            单次抓取固定 30 条（web / 小红书 / 抖音 / B 站统一生效）；「补历史」走深翻不受此限制，B 站还会在补历史时额外补全该 UP 的全部合集与收藏夹。
          </div>
          {fetchNote && (
            <div className="mt-2 text-[12px] text-text-tertiary bg-surface-secondary/60 border border-border rounded-lg px-3 py-2">
              {fetchNote}
            </div>
          )}
        </div>

        {/* 帖子列表 */}
        <div className="flex-1 overflow-auto p-4 space-y-3">
          {postsError && posts.length === 0 ? (
            <div className="text-center text-sm py-16">
              <p className="text-red-500 mb-2">加载动态失败</p>
              <p className="text-text-tertiary text-xs mb-3 break-words max-w-md mx-auto">{postsError}</p>
              <button
                onClick={() => void loadPosts()}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-border text-text-secondary hover:text-accent-primary hover:border-accent-primary/40"
              >
                <RefreshCw className="w-3.5 h-3.5" />重试
              </button>
            </div>
          ) : loadingPosts && posts.length === 0 ? (
            <div className="text-center text-text-tertiary text-sm py-16">加载中...</div>
          ) : posts.length === 0 ? (
            <div className="text-center text-text-tertiary py-16">
              <Zap className="w-10 h-10 mx-auto mb-3 opacity-30" />
              <p className="text-sm">还没有抓取到的动态</p>
              <p className="text-xs mt-1">从左侧选择一个博主点击「抓取」，或在上方粘贴链接。</p>
            </div>
          ) : (
            posts.map((post) => (
              <div
                key={post.id}
                className="rounded-xl border border-border bg-surface-primary p-4 transition-colors"
                // 跳过屏外卡片的渲染/布局，长列表（100+）滚动和切换明显更快。
                style={{ contentVisibility: 'auto', containIntrinsicSize: '220px' }}
              >
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 flex-wrap">
                      {post.isNew && (
                        <span className="inline-flex items-center gap-1 text-[10px] font-semibold px-2 py-0.5 rounded-full bg-sky-100 text-sky-700">
                          <Sparkles className="w-3 h-3" />新抓取
                        </span>
                      )}
                      {post.deconstructed && (
                        <span className="inline-flex items-center gap-1 text-[10px] font-semibold px-2 py-0.5 rounded-full bg-accent-primary/10 text-accent-primary">
                          <FlaskConical className="w-3 h-3" />已拆解
                        </span>
                      )}
                      <span className="text-[11px] px-1.5 py-0.5 rounded-full bg-surface-secondary text-text-tertiary">
                        {platformLabel(post.platform)}
                      </span>
                      {post.mediaType === 'video' && (
                        <span className="inline-flex items-center gap-1 text-[10px] font-semibold px-2 py-0.5 rounded-full bg-purple-100 text-purple-700">
                          视频
                        </span>
                      )}
                      {post.mediaType === 'video' && post.videoStatus === 'done' && (
                        <span
                          className="inline-flex items-center gap-1 text-[10px] font-semibold px-2 py-0.5 rounded-full bg-emerald-100 text-emerald-700"
                          title="视频本体已下载到本地，可进详情播放与分析"
                        >
                          <Check className="w-2.5 h-2.5" />已下载
                        </span>
                      )}
                      {post.mediaType === 'video' && post.videoStatus === 'downloading' && (
                        <span className="inline-flex items-center gap-1 text-[10px] font-semibold px-2 py-0.5 rounded-full bg-amber-100 text-amber-700">
                          <Loader2 className="w-2.5 h-2.5 animate-spin" />下载中
                        </span>
                      )}
                      {post.mediaType === 'video' && post.videoStatus === 'failed' && (
                        <span
                          className="inline-flex items-center gap-1 text-[10px] font-semibold px-2 py-0.5 rounded-full bg-red-100 text-red-700"
                          title={post.videoError || '视频下载失败，可进详情重试'}
                        >
                          下载失败
                        </span>
                      )}
                    </div>
                    <button
                      onClick={() => void openDetail(post)}
                      className="mt-2 text-left block w-full"
                      title="在 Beav 内查看详情"
                    >
                      <h3 className="text-sm font-semibold text-text-primary hover:text-accent-primary line-clamp-2 leading-snug">
                        {post.title || '(无标题)'}
                      </h3>
                    </button>
                    <div className="text-xs text-text-tertiary mt-1">
                      {post.author}
                      {post.authorUrl && (
                        <button onClick={() => void openExternal(post.authorUrl)} className="ml-1 inline-flex items-center hover:text-accent-primary">
                          <ExternalLink className="w-3 h-3" />
                        </button>
                      )}
                      <span className="mx-1.5">·</span>
                      {post.publishedAt ? (
                        <>
                          发布 {post.publishedAt}
                          <span className="mx-1.5">·</span>
                          抓取 {formatTs(post.updatedAt)}
                        </>
                      ) : (
                        <>抓取 {formatTs(post.updatedAt)}</>
                      )}
                    </div>
                    {post.excerpt && (
                      <p className="text-xs text-text-secondary mt-2 line-clamp-3 leading-relaxed">
                        {post.excerpt}
                      </p>
                    )}
                  </div>

                  <div className="shrink-0 flex flex-col items-end gap-2">
                    {post.topicInjected ? (
                      <span className="inline-flex items-center gap-1 text-[11px] text-emerald-600 bg-emerald-50 px-2 py-1 rounded-md">
                        <Check className="w-3 h-3" />已入选题
                      </span>
                    ) : (
                      <button
                        onClick={() => void handleInject(post)}
                        className="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md border border-border text-text-secondary hover:text-accent-primary hover:border-accent-primary/40 transition-colors whitespace-nowrap"
                        title="加入选题中心"
                      >
                        <Sparkles className="w-3.5 h-3.5" />加入选题
                      </button>
                    )}
                    <button
                      onClick={() => { void openDetail(post); if (post.content?.trim()) void runDeconstruct(post); }}
                      disabled={!post.content?.trim() || deconLoading}
                      className="inline-flex items-center gap-1 text-[11px] text-text-tertiary hover:text-accent-primary disabled:opacity-40"
                      title={post.content?.trim() ? '拆解这篇爆文的方法论' : '需先抓到正文'}
                    >
                      <FlaskConical className="w-3 h-3" />拆解
                    </button>
                    <button
                      onClick={() => void openExternal(post.postUrl)}
                      className="inline-flex items-center gap-1 text-[11px] text-text-tertiary hover:text-accent-primary"
                      title="打开原文"
                    >
                      <ExternalLink className="w-3 h-3" />原文
                    </button>
                    <button
                      onClick={() => void handlePostRemove(post.id)}
                      className="inline-flex items-center gap-1 text-[11px] text-text-tertiary hover:text-red-500"
                      title="删除这条动态"
                    >
                      <Trash2 className="w-3 h-3" />删除
                    </button>
                  </div>
                </div>
              </div>
            ))
          )}
        </div>
      </div>

      {/* 本地详情弹窗 —— 点卡片标题打开，正文在 Beav 内看，不跳平台 */}
      {detailOpen && detailPost && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
          onClick={() => setDetailOpen(false)}
        >
          <div
            className="w-full max-w-3xl max-h-[88vh] mx-4 bg-surface-primary rounded-xl border border-border shadow-2xl overflow-hidden flex flex-col"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="px-6 py-4 border-b border-border flex items-center justify-between gap-3 shrink-0">
              <div className="flex items-center gap-2 min-w-0">
                <span className="text-[11px] px-2 py-0.5 rounded-full bg-surface-secondary text-text-tertiary shrink-0">
                  {platformLabel(detailPost.platform)}
                </span>
                {detailPost.mediaType === 'video' && (
                  <span className="inline-flex items-center gap-1 text-[10px] font-semibold px-2 py-0.5 rounded-full bg-purple-100 text-purple-700 shrink-0">
                    视频
                  </span>
                )}
                <span className="text-xs text-text-tertiary truncate">{detailPost.author}</span>
                {detailPost.publishedAt && (
                  <span className="text-xs text-text-tertiary shrink-0">· {detailPost.publishedAt}</span>
                )}
              </div>
              <button onClick={() => setDetailOpen(false)} className="text-text-tertiary hover:text-text-primary shrink-0">
                <X className="w-4 h-4" />
              </button>
            </div>

            <div className="flex-1 overflow-auto px-6 py-4 space-y-4">
              <h2 className="text-base font-semibold text-text-primary leading-snug">
                {detailPost.title || '(无标题)'}
              </h2>

              {detailLoading && (
                <div className="flex items-center gap-2 text-xs text-text-tertiary">
                  <Loader2 className="w-3.5 h-3.5 animate-spin" />
                  正在从平台抓取正文…（首次约 10-20 秒，之后走本地缓存秒开）
                </div>
              )}

              {detailPost.coverUrl && (
                <img
                  src={resolveAssetUrl(detailPost.coverUrl)}
                  alt="封面"
                  className="w-full max-h-80 object-cover rounded-lg border border-border"
                  // 本地缓存缺失 / 加载失败就整块隐藏，不要留一个裂图占位
                  onError={(e) => {
                    e.currentTarget.style.display = 'none';
                  }}
                />
              )}
              {detailPost.mediaType === 'video' && (
                <div className="space-y-2">
                  {/* 已下载 → 本地直接播。
                      播放源用 720p/H.264 代理视频：原片常是 4K/HEVC/数百 MB，
                      WebView2 多数环境无 HEVC 解码器，且 asset 协议不支持 Range 分段请求，
                      直接播原片会一直转圈。代理文件缺失（老数据 / 转码失败）时退回原片。 */}
                  {detailPost.videoStatus === 'done' && detailPost.videoLocalPath && (
                    <VideoPlayerWithFallback
                      localPath={detailPost.videoLocalPath}
                      previewPath={detailPost.videoPreviewPath ?? undefined}
                    />
                  )}
                  {detailPost.videoStatus === 'downloading' && (
                    <div className="flex items-center gap-2 text-xs text-text-tertiary">
                      <Loader2 className="w-3.5 h-3.5 animate-spin" />
                      正在下载视频…（CDP 抽直链 + ffmpeg 落盘，稍等）
                    </div>
                  )}
                  {detailPost.videoStatus !== 'done' && detailPost.videoStatus !== 'downloading' && (
                    <p className="text-xs text-text-tertiary">
                      尚未下载视频本体 —— 点底部「下载视频」用登录态 Chrome 抓下来后，这里可直接播放并做转写分析；否则请点「原文」跳转平台。
                    </p>
                  )}
                  {detailPost.videoSize ? (
                    <p className="text-[11px] text-text-tertiary">
                      本地文件 {(detailPost.videoSize / 1024 / 1024).toFixed(1)} MB
                      {detailPost.videoDownloadedAt
                        ? ` · ${formatTs(String(detailPost.videoDownloadedAt))}`
                        : ''}
                    </p>
                  ) : null}
                  {detailPost.videoStatus === 'done' && detailPost.videoHasAudio === false && (
                    <div className="text-[12px] text-amber-700 dark:text-amber-400 bg-amber-50 dark:bg-amber-500/10 border border-amber-200 dark:border-amber-500/30 rounded-lg px-3 py-2">
                      这个视频只有画面、没有声音（抖音 DASH 只提供了视频轨），所以无法做语音转写。重新下载该视频会自动补上音频轨后再分析。
                    </div>
                  )}
                  {videoError && (
                    <div className="text-[12px] text-red-600 bg-red-50 border border-red-200 rounded-lg px-3 py-2 break-words">
                      {videoError}
                    </div>
                  )}
                  {videoAnalyzingId === detailPost.id && (
                    <div className="flex items-center gap-2 text-xs text-text-tertiary">
                      <Loader2 className="w-3.5 h-3.5 animate-spin" />
                      正在抽帧 + 语音转写…（视频越长越慢）
                    </div>
                  )}
                  {videoAnalysis && videoAnalysis.postId === detailPost.id && (
                    <VideoAnalysisPanel analysis={videoAnalysis} />
                  )}
                </div>
              )}

              {detailPost.content ? (
                <p className="text-sm text-text-secondary whitespace-pre-wrap leading-relaxed">
                  {detailPost.content}
                </p>
              ) : (
                !detailLoading && (
                  <p className="text-xs text-text-tertiary">
                    暂无本地正文
                    {detailPost.excerpt ? `：${detailPost.excerpt}` : ''}
                  </p>
                )
              )}

              {detailImages.length > 0 && (
                <div className="grid grid-cols-3 gap-2">
                  {detailImages.map((src, index) => (
                    <button
                      key={index}
                      type="button"
                      onClick={() => setLightboxIndex(index)}
                      title="点击查看大图"
                      className="relative overflow-hidden rounded-lg border border-border cursor-zoom-in focus:outline-none focus:ring-2 focus:ring-accent-primary/40"
                    >
                      <img
                        src={resolveAssetUrl(src)}
                        alt={`配图 ${index + 1}`}
                        className="w-full h-28 object-cover transition-transform duration-150 hover:scale-105"
                        // 加载失败的图直接移出列表（本地缓存丢了/老数据残留），不显示裂图
                        onError={() => {
                          setDetailImages((prev) => prev.filter((item) => item !== src));
                        }}
                      />
                    </button>
                  ))}
                </div>
              )}

              {detailNote && (
                <div className="text-[12px] text-amber-600 bg-amber-50 border border-amber-200 rounded-lg px-3 py-2">
                  {detailNote}
                </div>
              )}

              {/* 爆文拆解结果区：拆解中 / 错误 / 已沉淀六要素 */}
              {deconLoading && (
                <div className="flex items-center gap-2 text-xs text-text-tertiary border border-border rounded-lg px-3 py-2.5">
                  <Loader2 className="w-3.5 h-3.5 animate-spin" />
                  正在用 AI 拆解这篇爆文的方法论…（约 10-30 秒）
                </div>
              )}
              {deconError && (
                <div className="text-[12px] text-red-600 bg-red-50 border border-red-200 rounded-lg px-3 py-2">
                  {deconError}
                </div>
              )}
              {deconResult && deconResult.postId === detailPost.id && (
                <DeconFields decon={deconResult} />
              )}
            </div>

            <div className="px-6 py-3 border-t border-border flex flex-wrap items-center justify-between gap-3 shrink-0">
              <div className="flex items-center gap-4 text-xs text-text-tertiary shrink-0">
                <span>赞 {detailPost.likes}</span>
                <span>藏 {detailPost.collects}</span>
                <span>评 {detailPost.comments}</span>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                {detailPost.mediaType === 'video' && (
                  <>
                    <button
                      onClick={() => void runDownloadVideo(detailPost)}
                      disabled={videoBusyId === detailPost.id}
                      className="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md border border-border text-text-secondary hover:text-accent-primary hover:border-accent-primary/40 transition-colors whitespace-nowrap disabled:opacity-50"
                      title={
                        detailPost.videoStatus === 'downloading'
                          ? '当前处于「下载中」：若卡住（如 Chrome 被关闭），点此可强制重新下载'
                          : '用已登录的小红书 / 抖音 / B 站 Chrome 抽出真实视频直链并下载到本地'
                      }
                    >
                      <Download className={clsx('w-3 h-3', videoBusyId === detailPost.id && 'animate-bounce')} />
                      {detailPost.videoStatus === 'done' ? '重新下载' : '下载视频'}
                    </button>
                    <button
                      onClick={() => void runAnalyzeVideo(detailPost, !transcribeOnAnalyze)}
                      disabled={videoAnalyzingId === detailPost.id || detailPost.videoStatus !== 'done'}
                      className="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md border border-border text-text-secondary hover:text-accent-primary hover:border-accent-primary/40 transition-colors whitespace-nowrap disabled:opacity-50"
                      title={detailPost.videoStatus === 'done' ? '抽关键帧 + 语音转写' : '请先下载视频本体'}
                    >
                      <Video className={clsx('w-3 h-3', videoAnalyzingId === detailPost.id && 'animate-pulse')} />
                      分析视频
                    </button>
                    <button
                      type="button"
                      onClick={() => setTranscribeOnAnalyze((v) => !v)}
                      className={clsx(
                        'inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md border transition-colors whitespace-nowrap',
                        transcribeOnAnalyze
                          ? 'border-accent-primary/50 bg-accent-primary/10 text-accent-primary'
                          : 'border-border text-text-tertiary hover:text-text-secondary',
                      )}
                      title={transcribeOnAnalyze ? '分析时会做语音转写（产出转写全文/时间轴）' : '分析时只抽帧，不做语音转写'}
                    >
                      <AudioLines className={clsx('w-3 h-3', transcribeOnAnalyze && 'text-accent-primary')} />
                      语音转写
                      <span className={clsx('text-[10px] font-semibold', transcribeOnAnalyze ? 'text-accent-primary' : 'text-text-tertiary')}>
                        {transcribeOnAnalyze ? '开' : '关'}
                      </span>
                    </button>
                  </>
                )}
                <button
                  onClick={() => void runDeconstruct(detailPost)}
                  disabled={deconLoading || !detailPost.content || !detailPost.content.trim()}
                  className="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md border border-border text-text-secondary hover:text-accent-primary hover:border-accent-primary/40 transition-colors whitespace-nowrap disabled:opacity-50"
                  title={detailPost.content?.trim() ? '用 AI 拆解这篇爆文的方法论' : '请先抓到正文再拆解'}
                >
                  <FlaskConical className={clsx('w-3 h-3', deconLoading && 'animate-spin')} />
                  {deconResult && deconResult.postId === detailPost.id ? '重新拆解' : '拆解'}
                </button>
                {deconResult && deconResult.postId === detailPost.id && (
                  <button
                    onClick={() => void handleInject(detailPost)}
                    disabled={detailPost.topicInjected}
                    className="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md bg-accent-primary text-white hover:bg-accent-primary/90 transition-colors disabled:opacity-50"
                  >
                    <Sparkles className="w-3 h-3" />
                    {detailPost.topicInjected ? '已入选题' : '加入选题'}
                  </button>
                )}
                <button
                  onClick={async () => {
                    await openDetail(detailPost, true);
                    // 视频帖若还没下载完成（含卡在「下载中」/ 失败），重抓正文时一并触发视频下载，
                    // 避免用户点「重新抓正文」后视频仍不下载的困惑。
                    if (detailPost.mediaType === 'video' && detailPost.videoStatus !== 'done') {
                      void runDownloadVideo(detailPost);
                    }
                  }}
                  disabled={detailLoading}
                  className="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md border border-border text-text-secondary hover:text-accent-primary hover:border-accent-primary/40 transition-colors whitespace-nowrap disabled:opacity-50"
                >
                  <RefreshCw className={clsx('w-3 h-3', detailLoading && 'animate-spin')} />
                  重新抓正文
                </button>
                <button
                  onClick={() => void openExternal(detailPost.postUrl)}
                  className="inline-flex items-center gap-1.5 px-2.5 py-1.5 text-xs rounded-md border border-border text-text-secondary hover:text-accent-primary hover:border-accent-primary/40 transition-colors whitespace-nowrap"
                >
                  <ExternalLink className="w-3 h-3" />原文
                </button>
              </div>
            </div>
          </div>
        </div>
      )}

      {/* 大图预览 —— 点缩略图打开：背景关闭 / ESC / 左右切换，多图显示「当前/总数」
          Portal 到 body：`.app-main-shell--layered` 带 `isolation:isolate; z-index:1`，
          会把页面内任何 z-index 封在 z=1 的上下文里，压不住根级 z-70 的标题栏
          （表现为预览期间仍能点到窗口「关闭」按钮）。 */}
      {lightboxIndex !== null && detailImages[lightboxIndex] && createPortal(
        <div
          className="fixed inset-0 z-[80] flex items-center justify-center bg-black/90 backdrop-blur-sm"
          onClick={() => setLightboxIndex(null)}
        >
          {/* 关闭按钮下移到标题栏之下：避免与窗口右上角的「关闭程序」按钮重叠误触 */}
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              setLightboxIndex(null);
            }}
            className="absolute right-3 top-[calc(var(--app-titlebar-height)+0.5rem)] flex h-9 w-9 items-center justify-center rounded-full bg-white/10 text-white/85 transition-colors hover:bg-white/20 hover:text-white"
            title="关闭预览（ESC）"
          >
            <X className="w-5 h-5" />
          </button>
          {detailImages.length > 1 && (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                setLightboxIndex((i) => (i === null ? null : (i - 1 + detailImages.length) % detailImages.length));
              }}
              className="absolute left-4 top-1/2 -translate-y-1/2 text-white/80 hover:text-white transition-colors"
              title="上一张"
            >
              <ChevronLeft className="w-9 h-9" />
            </button>
          )}
          {detailImages.length > 1 && (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                setLightboxIndex((i) => (i === null ? null : (i + 1) % detailImages.length));
              }}
              className="absolute right-4 top-1/2 -translate-y-1/2 text-white/80 hover:text-white transition-colors"
              title="下一张"
            >
              <ChevronRight className="w-9 h-9" />
            </button>
          )}
          <img
            src={resolveAssetUrl(detailImages[lightboxIndex])}
            alt={`配图 ${lightboxIndex + 1}`}
            className="max-w-[92vw] max-h-[88vh] object-contain rounded-lg shadow-2xl"
            onClick={(e) => e.stopPropagation()}
          />
          {detailImages.length > 1 && (
            <span className="absolute bottom-4 left-1/2 -translate-x-1/2 text-white/70 text-xs bg-black/40 px-2 py-1 rounded-full">
              {lightboxIndex + 1} / {detailImages.length}
            </span>
          )}
        </div>,
        document.body,
      )}

      {/* 方法论库 —— 已沉淀的爆文拆解列表，可按作者 / 平台过滤，点开看六要素，一键加入选题 */}
      {methodsOpen && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
          onClick={() => setMethodsOpen(false)}
        >
          <div
            className="w-full max-w-3xl max-h-[82vh] mx-4 bg-surface-primary rounded-xl border border-border shadow-2xl overflow-hidden flex flex-col"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="px-6 py-4 border-b border-border flex items-center justify-between gap-3 shrink-0">
              <div className="flex items-center gap-2 min-w-0">
                <BookOpen className="w-4 h-4 text-accent-primary" strokeWidth={1.8} />
                <h3 className="text-base font-semibold text-text-primary">方法论库</h3>
                <span className="text-[11px] text-text-tertiary">{methodsItems.length} 篇已拆解</span>
              </div>
              <button onClick={() => setMethodsOpen(false)} className="text-text-tertiary hover:text-text-primary shrink-0">
                <X className="w-4 h-4" />
              </button>
            </div>

            {/* 过滤栏 */}
            <div className="px-6 py-3 border-b border-border flex items-center gap-2 flex-wrap shrink-0">
              <label className="text-xs text-text-secondary whitespace-nowrap">作者</label>
              <select
                value={methodsAuthor}
                onChange={(e) => { setMethodsAuthor(e.target.value); }}
                className="bg-surface-secondary border border-border rounded-lg px-2 py-1.5 text-sm focus:outline-none focus:ring-1 focus:ring-accent-primary"
              >
                <option value="">全部</option>
                {methodsAuthors.map((a) => (
                  <option key={a} value={a}>{a}</option>
                ))}
              </select>
              <label className="text-xs text-text-secondary whitespace-nowrap ml-2">平台</label>
              <select
                value={methodsPlatform}
                onChange={(e) => { setMethodsPlatform(e.target.value); }}
                className="bg-surface-secondary border border-border rounded-lg px-2 py-1.5 text-sm focus:outline-none focus:ring-1 focus:ring-accent-primary"
              >
                <option value="">全部</option>
                {PLATFORM_OPTIONS.filter((p) => p.value).map((p) => (
                  <option key={p.value} value={p.value}>{p.label}</option>
                ))}
              </select>
              <button
                onClick={() => void loadMethods()}
                disabled={methodsLoading}
                className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-border text-text-secondary hover:text-accent-primary hover:border-accent-primary/40 transition-colors disabled:opacity-50"
              >
                <RefreshCw className={clsx('w-3 h-3', methodsLoading && 'animate-spin')} />刷新
              </button>
            </div>

            <div className="flex-1 overflow-auto px-6 py-4 space-y-3">
              {methodsLoading ? (
                <div className="text-center text-text-tertiary text-sm py-16">加载中...</div>
              ) : methodsItems.length === 0 ? (
                <div className="text-center text-text-tertiary py-16">
                  <FlaskConical className="w-10 h-10 mx-auto mb-3 opacity-30" />
                  <p className="text-sm">方法论库还是空的</p>
                  <p className="text-xs mt-1">在帖子的「拆解」里用 AI 拆出第一篇爆文方法论吧。</p>
                </div>
              ) : (
                methodsItems.map((item) => {
                  const expanded = methodsExpanded.has(item.id);
                  return (
                    <div key={item.id} className="rounded-xl border border-border bg-surface-primary overflow-hidden">
                      <button
                        onClick={() => setMethodsExpanded((prev) => {
                          const next = new Set(prev);
                          if (next.has(item.id)) next.delete(item.id); else next.add(item.id);
                          return next;
                        })}
                        className="w-full flex items-center justify-between gap-3 px-4 py-3 text-left hover:bg-surface-secondary/40 transition-colors"
                      >
                        <div className="min-w-0">
                          <div className="flex items-center gap-2 flex-wrap">
                            <span className="text-[11px] px-1.5 py-0.5 rounded-full bg-surface-secondary text-text-tertiary">
                              {platformLabel(item.platform)}
                            </span>
                            <span className="text-[11px] text-text-tertiary">{item.author}</span>
                            <span className="text-xs text-text-secondary font-medium truncate">{item.postTitle || '(无标题)'}</span>
                          </div>
                          <div className="text-[11px] text-text-tertiary mt-0.5">
                            {item.createdAt ? formatTs(String(item.createdAt)) : ''}
                          </div>
                        </div>
                        <ChevronRight className={clsx('w-4 h-4 text-text-tertiary transition-transform shrink-0', expanded && 'rotate-90')} />
                      </button>

                      {expanded && (
                        <div className="px-4 pb-4 space-y-3 border-t border-border pt-3">
                          <DeconFields decon={item} />
                          <div className="flex items-center gap-2">
                            <button
                              onClick={() => void (async () => {
                                try {
                                  const result = (await window.ipcRenderer.monitor.injectTopic({ ids: [item.postId] })) as { success?: boolean; count?: number };
                                  showToast('ok', `已加入选题（${result?.count ?? 1} 条）`);
                                } catch (e) {
                                  showToast('err', e instanceof Error ? e.message : '加入选题失败');
                                }
                              })()}
                              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md bg-accent-primary text-white hover:bg-accent-primary/90 transition-colors"
                            >
                              <Sparkles className="w-3 h-3" />加入选题
                            </button>
                            {item.postUrl && (
                              <button
                                onClick={() => void openExternal(item.postUrl)}
                                className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs rounded-md border border-border text-text-secondary hover:text-accent-primary hover:border-accent-primary/40 transition-colors"
                              >
                                <ExternalLink className="w-3 h-3" />原文
                              </button>
                            )}
                          </div>
                        </div>
                      )}
                    </div>
                  );
                })
              )}
            </div>
          </div>
        </div>
      )}

      {/* 添加监控弹窗 */}
      {qrOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm">
          <div className="w-full max-w-md mx-4 bg-surface-primary rounded-xl border border-border shadow-2xl overflow-hidden">
            <div className="px-6 py-4 border-b border-border flex items-center justify-between">
              <h3 className="text-base font-semibold text-text-primary">扫码登录平台</h3>
              <button
                onClick={() => setQrOpen(false)}
                className="text-text-tertiary hover:text-text-primary"
              >
                <X className="w-4 h-4" />
              </button>
            </div>

            <div className="px-6 py-4 space-y-3">
              <p className="text-[11px] leading-relaxed text-text-tertiary">
                服务器上的平台账号是<b className="text-text-secondary">所有用户共享</b>的：扫码会更换服务器上的账号（原会话被顶掉）。
                抓取本身所有用户都共用这个登录态，所以 web 端也能直接扫码续期。
              </p>

              <div className="flex gap-1.5">
                {CDP_PLATFORMS.map((p) => (
                  <button
                    key={p}
                    onClick={() => { setQrPlatform(p); void loadQrCode(p); }}
                    className={clsx(
                      'flex-1 text-xs px-2 py-1.5 rounded-md border transition-colors',
                      qrPlatform === p
                        ? 'bg-accent-primary/10 text-accent-primary border-accent-primary/30'
                        : 'border-border text-text-secondary hover:text-text-primary',
                    )}
                  >
                    {platformLabel(p)}
                  </button>
                ))}
              </div>

              <div className="flex items-center justify-center py-2 min-h-[220px]">
                {qrBusy ? (
                  <div className="flex items-center gap-2 text-xs text-text-tertiary">
                    <Loader2 className="w-4 h-4 animate-spin" />
                    获取中…（首次启动 Chrome 约 10-30s）
                  </div>
                ) : qrImage ? (
                  <img
                    src={qrImage}
                    alt="登录二维码"
                    className="max-w-[240px] rounded-lg bg-white p-2"
                  />
                ) : (
                  <div className="text-xs text-text-tertiary text-center px-4">
                    {qrNote || '暂无二维码'}
                  </div>
                )}
              </div>

              <div className="flex items-center gap-2">
                <button
                  onClick={() => void loadQrCode(qrPlatform)}
                  disabled={qrBusy}
                  className="flex-1 inline-flex items-center justify-center gap-1.5 px-3 py-2 text-xs rounded-md border border-border text-text-secondary hover:text-accent-primary hover:border-accent-primary/40 transition-colors disabled:opacity-50"
                >
                  <RefreshCw className={clsx('w-3 h-3', qrBusy && 'animate-spin')} />刷新二维码
                </button>
                {qrNeedForce && (
                  <button
                    onClick={() => void loadQrCode(qrPlatform, true)}
                    disabled={qrBusy}
                    className="flex-1 inline-flex items-center justify-center gap-1.5 px-3 py-2 text-xs rounded-md border border-amber-300 text-amber-700 hover:bg-amber-50 transition-colors disabled:opacity-50"
                  >
                    仍然更换账号
                  </button>
                )}
                {qrDone && (
                  <span className="flex-1 text-center text-xs text-emerald-600">{qrNote}</span>
                )}
              </div>

              {qrDiag && (
                <div className="text-[11px] text-text-tertiary/80 break-all">{qrDiag}</div>
              )}
            </div>
          </div>
        </div>
      )}

      {addOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm">
          <div className="w-full max-w-md mx-4 bg-surface-primary rounded-xl border border-border shadow-2xl overflow-hidden">
            <div className="px-6 py-4 border-b border-border flex items-center justify-between">
              <h3 className="text-base font-semibold text-text-primary">添加对标博主</h3>
              <button onClick={() => setAddOpen(false)} className="text-text-tertiary hover:text-text-primary">
                <X className="w-4 h-4" />
              </button>
            </div>

            <div className="px-6 py-4 space-y-4">
              <div>
                <label className="block text-xs font-medium text-text-secondary mb-1.5">博主名称 *</label>
                <input
                  value={addForm.name}
                  onChange={(e) => { setAddForm((f) => ({ ...f, name: e.target.value })); setAddError(''); }}
                  onKeyDown={(e) => e.key === 'Enter' && void handleAdd()}
                  placeholder="例如：XX 说 AI"
                  className="w-full bg-surface-secondary border border-border rounded-lg px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-accent-primary"
                  autoFocus
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-text-secondary mb-1.5">平台 *</label>
                <select
                  value={addForm.platform}
                  onChange={(e) => { setAddForm((f) => ({ ...f, platform: e.target.value })); setAddError(''); }}
                  className={clsx(
                    'w-full bg-surface-secondary border border-border rounded-lg px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-accent-primary',
                    !addForm.platform && 'text-text-tertiary',
                  )}
                >
                  {/* 无默认值：未选时显示占位，且不可选中 */}
                  <option value="" disabled>请选择平台</option>
                  {PLATFORM_OPTIONS.map((opt) => (
                    <option key={opt.value} value={opt.value}>{opt.label}</option>
                  ))}
                </select>
              </div>
              <div>
                <label className="block text-xs font-medium text-text-secondary mb-1.5">主页链接 *</label>
                <input
                  value={addForm.profileUrl}
                  onChange={(e) => { setAddForm((f) => ({ ...f, profileUrl: e.target.value })); setAddError(''); }}
                  placeholder="https://space.bilibili.com/xxxx"
                  className="w-full bg-surface-secondary border border-border rounded-lg px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-accent-primary"
                />
                {addForm.platform === 'bilibili' && (
                  <p className="mt-1.5 text-[11px] leading-relaxed text-text-tertiary">
                    B 站支持三种链接：UP 主空间（…/&lt;mid&gt;，会自动补全该 UP 的全部合集）、
                    单个合集（…/lists/&lt;sid&gt;）、收藏夹（…/favlist?fid=&lt;fid&gt;）。
                  </p>
                )}
              </div>
              <div>
                <label className="block text-xs font-medium text-text-secondary mb-1.5">备注</label>
                <input
                  value={addForm.note}
                  onChange={(e) => setAddForm((f) => ({ ...f, note: e.target.value }))}
                  placeholder="可选，例如：重点关注选题方向"
                  className="w-full bg-surface-secondary border border-border rounded-lg px-3 py-2 text-sm focus:outline-none focus:ring-1 focus:ring-accent-primary"
                />
              </div>
              {addError && <p className="text-xs text-red-500">{addError}</p>}
            </div>

            <div className="px-6 py-4 bg-surface-secondary border-t border-border flex items-center justify-end gap-3">
              <button
                onClick={() => setAddOpen(false)}
                className="px-4 py-2 text-sm text-text-secondary hover:text-text-primary border border-border rounded-lg transition-colors"
              >
                取消
              </button>
              <button
                onClick={() => void handleAdd()}
                className="px-4 py-2 text-sm text-white bg-accent-primary hover:bg-accent-primary/90 rounded-lg transition-colors"
              >
                添加
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Toast */}
      {toast && (
        <div className={clsx(
          'fixed bottom-6 left-1/2 -translate-x-1/2 z-[60] px-4 py-2.5 rounded-lg shadow-lg text-sm font-medium',
          toast.kind === 'ok' ? 'bg-emerald-600 text-white' : 'bg-red-600 text-white',
        )}>
          {toast.text}
        </div>
      )}
    </div>
  );
}
