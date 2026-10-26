import type { BridgeCore } from '../types';

export interface MonitorWatchPayload {
  name: string;
  platform?: string;
  profileUrl?: string;
  handle?: string;
  note?: string;
  enabled?: boolean;
  intervalHours?: number;
}

export interface MonitorFetchPayload {
  watchId?: string;
  url?: string;
  limit?: number;
}

export interface MonitorChromeStatus {
  chromeInstalled: boolean;
  chromePath?: string;
  platform?: string;
  debugPort: number;
  debugReady: boolean;
  readyPlatforms?: string[];
}

export interface MonitorChromeLaunchPayload {
  platform: string;
  url?: string;
}

export interface MonitorChromeLaunchResult {
  success: boolean;
  platform: string;
  debugPort: number;
  profileDir: string;
  note: string;
}

export interface MonitorChromeTestPayload {
  platform?: string;
  url?: string;
  expression?: string;
}

export interface MonitorChromeTestResult {
  success: boolean;
  result?: unknown;
  error?: string;
}

export interface MonitorPostDetailResult {
  success: boolean;
  post?: unknown;
  fetched?: boolean;
  images?: string[] | null;
  note?: string;
}

export interface RetroSeriesPoint {
  ts: number;
  likes: number;
  comments: number;
  collects: number;
  total: number;
  posts: number;
}

export interface RetroTopPost {
  id: string;
  title: string;
  platform: string;
  author: string;
  likes: number;
  comments: number;
  collects: number;
  total: number;
  url: string;
  mediaType: string;
  coverUrl?: string | null;
  updatedAt: string;
}

export interface RetroSummary {
  trackedPosts: number;
  snapshots: number;
  activeDays: number;
  totalEngagement: number;
  avgPerPost: number;
  topLikes: number;
}

export interface MonitorRetroResult {
  success: boolean;
  authors: string[];
  days: number;
  author: string;
  platform: string;
  series: RetroSeriesPoint[];
  topPosts: RetroTopPost[];
  summary: RetroSummary;
}

export interface MonitorRetroPayload {
  author?: string;
  platform?: string;
  days?: number;
}

const READ_TIMEOUT_MS = 15_000;
const FETCH_TIMEOUT_MS = 60_000;
const CHROME_TIMEOUT_MS = 30_000;

export function createMonitorBridge(core: BridgeCore) {
  return {
    monitor: {
      watchAdd: <T = unknown>(payload: MonitorWatchPayload) =>
        core.invokeChannelGuarded<T>('monitor:watch-add', payload, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, error: 'add timed out' }) as unknown as T,
        }),
      watchList: <T = unknown>(payload?: Record<string, unknown>) =>
        core.invokeChannelGuarded<T>('monitor:watch-list', payload || {}, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => [] as unknown as T,
        }),
      watchRemove: <T = unknown>(id: string) =>
        core.invokeChannelGuarded<T>('monitor:watch-remove', { id }, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, error: 'remove timed out' }) as unknown as T,
        }),
      fetch: <T = unknown>(payload: MonitorFetchPayload) =>
        core.invokeChannelGuarded<T>('monitor:fetch', payload, {
          timeoutMs: FETCH_TIMEOUT_MS,
          fallback: () => ({ success: false, error: 'fetch timed out' }) as unknown as T,
        }),
      postList: <T = unknown>(payload?: Record<string, unknown>) =>
        core.invokeChannelGuarded<T>('monitor:post-list', payload || {}, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => [] as unknown as T,
        }),
      injectTopic: <T = unknown>(payload: { id?: string; ids?: string[] }) =>
        core.invokeChannelGuarded<T>('monitor:inject-topic', payload, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, error: 'inject timed out', count: 0, items: [] }) as unknown as T,
        }),
      // —— Chrome 头浏览器（小红书 / 抖音登录态抓取）——
      chromeStatus: <T = MonitorChromeStatus>(platform?: string) =>
        core.invokeChannelGuarded<T>('monitor:chrome-status', platform ? { platform } : {}, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ chromeInstalled: false, debugPort: 0, debugReady: false, readyPlatforms: [] }) as unknown as T,
        }),
      chromeLaunch: <T = MonitorChromeLaunchResult>(payload: MonitorChromeLaunchPayload) =>
        core.invokeChannelGuarded<T>('monitor:chrome-launch', payload, {
          timeoutMs: CHROME_TIMEOUT_MS,
          fallback: () => ({ success: false, platform: payload.platform, debugPort: 0, profileDir: '', note: '启动 Chrome 超时' }) as unknown as T,
        }),
      chromeTest: <T = MonitorChromeTestResult>(payload: MonitorChromeTestPayload) =>
        core.invokeChannelGuarded<T>('monitor:chrome-test', payload, {
          timeoutMs: CHROME_TIMEOUT_MS,
          fallback: () => ({ success: false, error: 'chrome test timed out' }) as unknown as T,
        }),
      chromeLogout: <T = { success: boolean; note: string }>() =>
        core.invokeChannelGuarded<T>('monitor:chrome-shutdown', {}, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, note: '退出登录超时' }) as unknown as T,
        }),
      clear: <T = { success: boolean; removedWatchlist: number; removedPosts: number }>() =>
        core.invokeChannelGuarded<T>('monitor:clear', {}, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, removedWatchlist: 0, removedPosts: 0 }) as unknown as T,
        }),
      postRemove: <T = { success: boolean }>(id: string) =>
        core.invokeChannelGuarded<T>('monitor:post-remove', { id }, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false }) as unknown as T,
        }),
      clearPosts: <T = { success: boolean; removedPosts: number }>() =>
        core.invokeChannelGuarded<T>('monitor:clear-posts', {}, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, removedPosts: 0 }) as unknown as T,
        }),
      // 本地详情：CDP 抓正文（首次 ~20s，之后走缓存），不跳外部平台
      postDetail: <T = MonitorPostDetailResult>(id: string, refresh?: boolean) =>
        core.invokeChannelGuarded<T>('monitor:post-detail', { id, refresh: refresh ?? false }, {
          timeoutMs: 45_000,
          fallback: () => ({ success: false, note: '获取详情超时' }) as unknown as T,
        }),
      // 数据复盘聚合：按博主 / 平台 / 时间窗聚合互动走势 + Top 帖子
      retro: <T = MonitorRetroResult>(payload: MonitorRetroPayload) =>
        core.invokeChannelGuarded<T>('monitor:retro', payload || {}, {
          timeoutMs: 15_000,
          fallback: () =>
            ({
              success: false,
              authors: [],
              days: payload?.days ?? 30,
              author: payload?.author ?? '',
              platform: payload?.platform ?? '',
              series: [],
              topPosts: [],
              summary: {
                trackedPosts: 0,
                snapshots: 0,
                activeDays: 0,
                totalEngagement: 0,
                avgPerPost: 0,
                topLikes: 0,
              },
            }) as unknown as T,
        }),
      // 配置单次抓取条数（持久化到 store）
      setScanLimit: <T = { success: boolean; limit: number }>(limit: number) =>
        core.invokeChannelGuarded<T>('monitor:set-scan-limit', { limit }, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, limit }) as unknown as T,
        }),
      getScanLimit: <T = { success: boolean; limit: number }>() =>
        core.invokeChannelGuarded<T>('monitor:get-scan-limit', {}, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, limit: 12 }) as unknown as T,
        }),
      // 配置异常倍数阈值（持久化到 store）
      setSpikeThreshold: <T = { success: boolean; threshold: number }>(threshold: number) =>
        core.invokeChannelGuarded<T>('monitor:set-spike-threshold', { threshold }, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, threshold }) as unknown as T,
        }),
      getSpikeThreshold: <T = { success: boolean; threshold: number }>() =>
        core.invokeChannelGuarded<T>('monitor:get-spike-threshold', {}, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, threshold: 3 }) as unknown as T,
        }),
      // 全局「导出根目录」（与工作区解耦）：空 = 跟随工作区根
      getExportRoot: <T = { success: boolean; root: string }>() =>
        core.invokeChannelGuarded<T>('monitor:get-export-root', {}, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, root: '' }) as unknown as T,
        }),
      setExportRoot: <T = { success: boolean; root: string }>(root: string) =>
        core.invokeChannelGuarded<T>('monitor:set-export-root', { root }, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, root }) as unknown as T,
        }),
      // 弹系统目录选择器选「导出根目录」
      pickExportRoot: <T = { success: boolean; canceled?: boolean; path?: string | null }>() =>
        core.invokeChannelGuarded<T>('monitor:pick-export-root', {}, {
          timeoutMs: 120_000,
          fallback: () => ({ success: false, canceled: true }) as unknown as T,
        }),
      // 「视频分析成功后删除原片」开关（仅保留预览）
      getDeleteOriginal: <T = { success: boolean; enabled: boolean }>() =>
        core.invokeChannelGuarded<T>('monitor:get-delete-original', {}, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, enabled: false }) as unknown as T,
        }),
      setDeleteOriginal: <T = { success: boolean; enabled: boolean }>(enabled: boolean) =>
        core.invokeChannelGuarded<T>('monitor:set-delete-original', { enabled }, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, enabled }) as unknown as T,
        }),
      // 爆文拆解：把一篇对标帖文案交给 LLM 拆成方法论要素，落库到 post_deconstructions
      deconstruct: <T = { success: boolean; deconstruction?: Record<string, unknown> }>(postId: string) =>
        core.invokeChannelGuarded<T>('monitor:deconstruct', { postId }, {
          timeoutMs: 90_000,
          fallback: () => ({ success: false }) as unknown as T,
        }),
      // 列出已沉淀的爆文拆解（方法论库）
      deconList: <T = { success: boolean; items: Record<string, unknown>[]; authors: string[] }>(
        payload?: { author?: string; platform?: string },
      ) =>
        core.invokeChannelGuarded<T>('monitor:decon-list', payload ?? {}, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, items: [], authors: [] }) as unknown as T,
        }),
      // 下载视频：CDP 抽真实流地址 → ffmpeg 直落盘（需已登录的小红书/抖音 Chrome），耗时较长。
      // 下载流程内部会自动探测音轨：抖音 DASH 只给视频轨时会补下音频轨再合并，
      // 所以「重新下载」既刷新清晰度也补齐声音（无需单独的「补音轨」入口）。
      downloadVideo: <T = { success: boolean; localPath?: string; note?: string }>(
        id: string,
        options?: { patchAudio?: boolean },
      ) =>
        core.invokeChannelGuarded<T>('monitor:download-video', { id, ...(options ?? {}) }, {
          timeoutMs: 600_000,
          fallback: () => ({ success: false, note: '下载视频超时（含转码，最长约 10 分钟）' }) as unknown as T,
        }),
      // 只补语音转写（不重抽帧、不动已有帧）
      transcribeVideo: <T = { success: boolean; status?: string; analysis?: Record<string, unknown> }>(
        id: string,
      ) =>
        core.invokeChannelGuarded<T>('monitor:transcribe-video', { id }, {
          timeoutMs: 240_000,
          fallback: () => ({ success: false }) as unknown as T,
        }),
      // 分析视频：抽帧 + ASR 转写 + 摘要，落库 post_video_analyses
      // 4 分钟以上的视频（抽帧 + 长音频 ASR）可能超过 4 分钟，这里放宽到 15 分钟。
      analyzeVideo: <T = { success: boolean; note?: string; analysis?: Record<string, unknown> }>(
        id: string,
        options?: { force?: boolean },
      ) =>
        core.invokeChannelGuarded<T>('monitor:analyze-video', { id, ...(options ?? {}) }, {
          timeoutMs: 900_000,
          fallback: () => ({ success: false, note: '分析视频超时（含抽帧 + 语音转写，最长约 15 分钟）' }) as unknown as T,
        }),
      // 读取已有的视频分析结果
      videoAnalysis: <T = { success: boolean; analysis?: Record<string, unknown> | null }>(id: string) =>
        core.invokeChannelGuarded<T>('monitor:video-analysis', { id }, {
          timeoutMs: READ_TIMEOUT_MS,
          fallback: () => ({ success: false, analysis: null }) as unknown as T,
        }),
      // 手动触发「每日 00:00 批量预取」：列表(URL 去重)→补全正文→视频下载+转写。
      // 主要用于调试（不必等到半夜），正式触发由后端 00:00 常驻循环驱动；耗时可能较长（含视频转码）。
      runDaily: <
        T = {
          success: boolean;
          watched: number;
          /** 实际使用的增量窗口（每博主每次抓取的上限） */
          limit: number;
          newPosts: number;
          detailEnriched: number;
          detailSkipped: number;
          skipNote?: string | null;
          /** 本轮回填的存量帖数（历史上没抓过详情的旧帖） */
          backfilled: number;
          /** 回收的僵尸 downloading 视频数 */
          recoveredDownloading: number;
          videosDownloaded: number;
          videosTranscribed: number;
          failed: number;
          ranAt: number;
        },
      >() =>
        core.invokeChannelGuarded<T>('monitor:run-daily', {}, {
          timeoutMs: 600_000,
          fallback: () => ({ success: false }) as unknown as T,
        }),
      // 补历史：深翻博主主页，把更早的历史帖一次性落库（日常抓取只取最新 N 条）。
      // 耗时较长（滚动加载 + 限量补正文），故超时放宽到 10 分钟。
      backfillHistory: <
        T = {
          success: boolean;
          watched: number;
          scanned: number;
          /** 主页声明的作品总数（进度条分母；未知为 0） */
          profileTotal: number;
          limit: number;
          newPosts: number;
          detailEnriched: number;
          detailSkipped: number;
          skipNote?: string | null;
          backfilled: number;
          alreadyBackfilled: number;
          limitReached: number;
          /** 一句话诊断提示（为什么只翻到首屏 / 滚动是否正常） */
          hint?: string | null;
          pendingVideos: number;
          failed: number;
          ranAt: number;
        },
      >(watchId?: string) =>
        core.invokeChannelGuarded<T>(
          'monitor:backfill-history',
          // 手动点击 = 用户明确要求补历史 → 强制执行，不被「已补过」标记挡住
          watchId ? { watchId, force: true } : { force: true },
          {
            timeoutMs: 600_000,
            fallback: () => ({ success: false }) as unknown as T,
          },
        ),
    },
  };
}
