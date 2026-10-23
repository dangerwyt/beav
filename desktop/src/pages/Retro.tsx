import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  LineChart as LineChartIcon,
  RefreshCw,
  ExternalLink,
  Loader2,
  TrendingUp,
} from 'lucide-react';
import { clsx } from 'clsx';
import type {
  MonitorRetroResult,
  RetroSeriesPoint,
  RetroTopPost,
} from '../bridge/domains/monitorBridge';

const DAY_OPTIONS = [
  { value: 7, label: '近 7 天' },
  { value: 30, label: '近 30 天' },
  { value: 90, label: '近 90 天' },
];

const SERIES_META: { key: keyof RetroSeriesPoint; label: string; color: string }[] = [
  { key: 'total', label: '总互动', color: '#6366f1' },
  { key: 'likes', label: '点赞', color: '#ef4444' },
  { key: 'comments', label: '评论', color: '#10b981' },
  { key: 'collects', label: '收藏', color: '#f59e0b' },
];

function formatNum(n: number): string {
  if (n >= 10_000) return `${(n / 10_000).toFixed(1)}万`;
  return String(n);
}

function dateLabel(ts: number): string {
  const d = new Date(ts);
  const mm = `${d.getMonth() + 1}`.padStart(2, '0');
  const dd = `${d.getDate()}`.padStart(2, '0');
  return `${mm}-${dd}`;
}

/// 纯 SVG 走势折线图（无图表依赖）。支持 0~N 个点，点数不足时给引导提示。
function TrendChart({ series }: { series: RetroSeriesPoint[] }) {
  const W = 720;
  const H = 260;
  const padL = 44;
  const padR = 16;
  const padT = 16;
  const padB = 28;
  const innerW = W - padL - padR;
  const innerH = H - padT - padB;

  const n = series.length;
  const maxVal = Math.max(1, ...series.map((p) => Number(p.total) || 0));

  const xAt = (i: number) => (n <= 1 ? padL + innerW / 2 : padL + (i / (n - 1)) * innerW);
  const yAt = (v: number) => padT + innerH - (v / maxVal) * innerH;

  if (n === 0) {
    return (
      <div className="flex h-40 items-center justify-center rounded-lg border border-border bg-surface/40 text-sm text-text-secondary">
        暂无历史快照。请先在「对标监控」重新抓取该博主或打开详情，积累互动数据点后会自动出图。
      </div>
    );
  }

  const yTicks = [0, 0.25, 0.5, 0.75, 1].map((r) => ({
    v: Math.round(maxVal * r),
    y: padT + innerH - r * innerH,
  }));

  const labelEvery = n <= 8 ? 1 : Math.ceil(n / 8);

  return (
    <svg
      viewBox={`0 0 ${W} ${H}`}
      className="w-full rounded-lg border border-border bg-surface/40"
      role="img"
      aria-label="互动走势折线图"
    >
      {/* Y 轴网格 + 刻度 */}
      {yTicks.map((t) => (
        <g key={t.v}>
          <line
            x1={padL}
            x2={W - padR}
            y1={t.y}
            y2={t.y}
            stroke="#e5e7eb"
            strokeWidth={1}
          />
          <text x={padL - 6} y={t.y + 4} textAnchor="end" fontSize={10} fill="#9ca3af">
            {formatNum(t.v)}
          </text>
        </g>
      ))}
      {/* X 轴日期 */}
      {series.map((p, i) =>
        labelEvery > 0 && i % labelEvery === 0 ? (
          <text
            key={p.ts}
            x={xAt(i)}
            y={H - 8}
            textAnchor="middle"
            fontSize={10}
            fill="#9ca3af"
          >
            {dateLabel(p.ts)}
          </text>
        ) : null,
      )}
      {/* 各指标折线 */}
      {SERIES_META.map((meta) => (
        <polyline
          key={meta.key}
          fill="none"
          stroke={meta.color}
          strokeWidth={2}
          strokeLinejoin="round"
          strokeLinecap="round"
          points={series
            .map((p, i) => `${xAt(i)},${yAt(Number(p[meta.key]) || 0)}`)
            .join(' ')}
        />
      ))}
      {/* 数据点 */}
      {SERIES_META.map((meta) =>
        series.map((p, i) => (
          <circle
            key={`${meta.key}-${p.ts}`}
            cx={xAt(i)}
            cy={yAt(Number(p[meta.key]) || 0)}
            r={2.5}
            fill={meta.color}
          />
        )),
      )}
    </svg>
  );
}

function SummaryCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-border bg-surface/60 px-4 py-3">
      <div className="text-xs text-text-secondary">{label}</div>
      <div className="mt-1 text-xl font-semibold text-text-primary">{value}</div>
    </div>
  );
}

export function Retro({ isActive = true }: { isActive?: boolean }) {
  const [authors, setAuthors] = useState<string[]>([]);
  const [author, setAuthor] = useState('');
  const [days, setDays] = useState(30);
  const [data, setData] = useState<MonitorRetroResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<{ kind: 'ok' | 'err'; text: string } | null>(null);

  const loadSeq = useRef(0);
  const toastTimer = useRef<number | undefined>(undefined);

  const showToast = useCallback((kind: 'ok' | 'err', text: string) => {
    setToast({ kind, text });
    if (toastTimer.current) window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 2600);
  }, []);

  const loadRetro = useCallback(async () => {
    const seq = loadSeq.current + 1;
    loadSeq.current = seq;
    setLoading(true);
    setError(null);
    try {
      const result = (await window.ipcRenderer.monitor.retro({
        author: author || undefined,
        days,
      })) as MonitorRetroResult;
      if (seq !== loadSeq.current) return;
      // 首次加载时用返回的作者列表初始化下拉
      if (Array.isArray(result?.authors) && result.authors.length > 0) {
        setAuthors((prev) =>
          prev.length === result.authors.length && prev.every((a) => result.authors.includes(a))
            ? prev
            : result.authors,
        );
      }
      setData(result);
    } catch (e) {
      console.error('Failed to load retro:', e);
      if (seq === loadSeq.current) {
        setData(null);
        setError(e instanceof Error ? e.message : '加载复盘数据失败');
      }
    } finally {
      if (seq === loadSeq.current) setLoading(false);
    }
  }, [author, days]);

  // 进入页面 / 切换博主 / 切换时间窗 → 重新聚合
  useEffect(() => {
    if (!isActive) return;
    void loadRetro();
  }, [isActive, loadRetro]);

  // 看门狗：兜底解锁 loading，防止 IPC 静默失败导致永久「加载中」
  useEffect(() => {
    if (!isActive) return;
    const timer = window.setTimeout(() => {
      setLoading((prev) => {
        if (prev && !data) {
          setError((msg) => msg || '请求超时（15s），请检查后端 monitor:retro 通道');
        }
        return false;
      });
    }, 16_000);
    return () => window.clearTimeout(timer);
  }, [isActive, data, author, days]);

  const openExternal = useCallback(
    async (url?: string | null) => {
      if (!url) {
        showToast('err', '没有可打开的链接');
        return;
      }
      try {
        await window.ipcRenderer.openExternalUrl(url);
      } catch (e) {
        console.warn('openExternalUrl failed:', e);
        showToast('err', '打开原文失败');
      }
    },
    [showToast],
  );

  const summary = data?.summary;
  const series = useMemo<RetroSeriesPoint[]>(() => data?.series ?? [], [data]);
  const topPosts = useMemo<RetroTopPost[]>(() => data?.topPosts ?? [], [data]);

  return (
    <div className="h-full min-h-0 flex flex-col">
      {/* 顶栏：标题 + 筛选 + 刷新 */}
      <div className="flex flex-wrap items-center gap-3 border-b border-border px-5 py-3">
        <div className="flex items-center gap-2 text-text-primary">
          <LineChartIcon size={18} className="text-accent-primary" />
          <span className="text-base font-semibold">数据复盘</span>
        </div>
        <span className="text-xs text-text-secondary">
          基于「对标监控」抓取的互动历史，画博主互动走势 + Top 帖子
        </span>
        <div className="ml-auto flex flex-wrap items-center gap-2">
          <select
            value={author}
            onChange={(e) => setAuthor(e.target.value)}
            className="rounded-md border border-border bg-surface px-2 py-1.5 text-sm text-text-primary outline-none focus:border-accent-primary"
          >
            <option value="">全部博主</option>
            {authors.map((a) => (
              <option key={a} value={a}>
                {a}
              </option>
            ))}
          </select>
          <div className="flex overflow-hidden rounded-md border border-border">
            {DAY_OPTIONS.map((opt) => (
              <button
                key={opt.value}
                onClick={() => setDays(opt.value)}
                className={clsx(
                  'px-2.5 py-1.5 text-xs transition-colors',
                  days === opt.value
                    ? 'bg-accent-primary text-white'
                    : 'bg-surface text-text-secondary hover:text-text-primary',
                )}
              >
                {opt.label}
              </button>
            ))}
          </div>
          <button
            onClick={() => void loadRetro()}
            disabled={loading}
            className="inline-flex items-center gap-1.5 rounded-md border border-border bg-surface px-2.5 py-1.5 text-xs text-text-secondary hover:text-accent-primary disabled:opacity-50"
          >
            <RefreshCw size={13} className={clsx(loading && 'animate-spin')} />
            刷新
          </button>
        </div>
      </div>

      {/* 主体 */}
      <div className="min-h-0 flex-1 overflow-auto px-5 py-4">
        {loading && !data ? (
          <div className="flex h-40 items-center justify-center text-text-secondary">
            <Loader2 className="mr-2 animate-spin" size={16} /> 加载中…
          </div>
        ) : error && !data ? (
          <div className="flex h-40 items-center justify-center text-sm text-red-500">{error}</div>
        ) : (
          <div className="space-y-5">
            {/* 汇总卡片 */}
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-5">
              <SummaryCard label="跟踪帖数" value={formatNum(summary?.trackedPosts ?? 0)} />
              <SummaryCard label="活跃天数" value={formatNum(summary?.activeDays ?? 0)} />
              <SummaryCard label="总互动" value={formatNum(summary?.totalEngagement ?? 0)} />
              <SummaryCard label="篇均互动" value={formatNum(summary?.avgPerPost ?? 0)} />
              <SummaryCard label="最高赞" value={formatNum(summary?.topLikes ?? 0)} />
            </div>

            {/* 走势图 */}
            <section>
              <div className="mb-2 flex items-center gap-3">
                <h3 className="text-sm font-semibold text-text-primary">互动走势</h3>
                <div className="flex flex-wrap items-center gap-3">
                  {SERIES_META.map((meta) => (
                    <span key={meta.key} className="flex items-center gap-1 text-xs text-text-secondary">
                      <span
                        className="inline-block h-2 w-2 rounded-full"
                        style={{ backgroundColor: meta.color }}
                      />
                      {meta.label}
                    </span>
                  ))}
                </div>
              </div>
              <TrendChart series={series} />
              {series.length > 0 && series.length < 2 && (
                <p className="mt-2 text-xs text-text-secondary">
                  当前仅 1 个数据点，走势线暂未成形；继续抓取该博主 / 打开详情即可累积更多点。
                </p>
              )}
            </section>

            {/* Top 帖子 */}
            <section>
              <h3 className="mb-2 flex items-center gap-1.5 text-sm font-semibold text-text-primary">
                <TrendingUp size={15} className="text-accent-primary" /> 互动 Top 帖子
              </h3>
              {topPosts.length === 0 ? (
                <div className="rounded-lg border border-border bg-surface/40 px-4 py-6 text-center text-sm text-text-secondary">
                  暂无帖子数据
                </div>
              ) : (
                <div className="overflow-hidden rounded-lg border border-border">
                  <table className="w-full text-left text-sm">
                    <thead className="bg-surface/60 text-xs text-text-secondary">
                      <tr>
                        <th className="px-3 py-2 font-medium">标题</th>
                        <th className="px-3 py-2 font-medium">平台</th>
                        <th className="px-3 py-2 text-right font-medium">点赞</th>
                        <th className="px-3 py-2 text-right font-medium">评论</th>
                        <th className="px-3 py-2 text-right font-medium">收藏</th>
                        <th className="px-3 py-2 text-right font-medium">合计</th>
                        <th className="px-3 py-2" />
                      </tr>
                    </thead>
                    <tbody>
                      {topPosts.map((post) => (
                        <tr
                          key={post.id}
                          className="border-t border-border hover:bg-surface/40"
                        >
                          <td className="max-w-[320px] truncate px-3 py-2 text-text-primary" title={post.title}>
                            {post.title || '(无标题)'}
                          </td>
                          <td className="px-3 py-2 text-text-secondary">{post.platform}</td>
                          <td className="px-3 py-2 text-right text-text-primary">{formatNum(post.likes)}</td>
                          <td className="px-3 py-2 text-right text-text-primary">{formatNum(post.comments)}</td>
                          <td className="px-3 py-2 text-right text-text-primary">{formatNum(post.collects)}</td>
                          <td className="px-3 py-2 text-right font-medium text-accent-primary">
                            {formatNum(post.total)}
                          </td>
                          <td className="px-3 py-2 text-right">
                            <button
                              onClick={() => void openExternal(post.url)}
                              title="打开原文"
                              className="inline-flex items-center text-text-secondary hover:text-accent-primary"
                            >
                              <ExternalLink size={14} />
                            </button>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </section>
          </div>
        )}
      </div>

      {/* Toast */}
      {toast && (
        <div
          className={clsx(
            'fixed bottom-6 left-1/2 z-50 -translate-x-1/2 rounded-lg px-4 py-2 text-sm text-white shadow-lg',
            toast.kind === 'ok' ? 'bg-emerald-600' : 'bg-red-600',
          )}
        >
          {toast.text}
        </div>
      )}
    </div>
  );
}
