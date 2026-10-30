import { useCallback, useEffect, useRef, useState } from 'react';
import {
  Send,
  Plus,
  Trash2,
  ExternalLink,
  Loader2,
  Check,
  X,
  Pencil,
  Megaphone,
} from 'lucide-react';
import { clsx } from 'clsx';
import type { PublishRecord, PublishPlatform } from '../bridge/domains/publishBridge';
import { appConfirm } from '../utils/appDialogs';

const EMPTY_FORM = {
  platform: 'xiaohongshu',
  postUrl: '',
  title: '',
  publishedAt: '',
  status: 'planned',
  views: 0,
  likes: 0,
  comments: 0,
  collects: 0,
  notes: '',
};

const STATUS_META: Record<string, { label: string; cls: string }> = {
  planned: { label: '待发布', cls: 'bg-amber-50 text-amber-600' },
  published: { label: '已发布', cls: 'bg-emerald-50 text-emerald-600' },
  failed: { label: '失败', cls: 'bg-red-50 text-red-600' },
};

const METRIC_LABELS: Record<'views' | 'likes' | 'comments' | 'collects', string> = {
  views: '播放',
  likes: '点赞',
  comments: '评论',
  collects: '收藏',
};

export function Publish({ isActive = true }: { isActive?: boolean }) {
  const [items, setItems] = useState<PublishRecord[]>([]);
  const [platforms, setPlatforms] = useState<PublishPlatform[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [toast, setToast] = useState<{ kind: 'ok' | 'err'; text: string } | null>(null);

  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<PublishRecord | null>(null);
  const [form, setForm] = useState({ ...EMPTY_FORM });
  const [saving, setSaving] = useState(false);

  const itemsRef = useRef<PublishRecord[]>([]);
  const loadSeq = useRef(0);
  const toastTimer = useRef<number | undefined>(undefined);

  useEffect(() => {
    itemsRef.current = items;
  }, [items]);

  const showToast = useCallback((kind: 'ok' | 'err', text: string) => {
    setToast({ kind, text });
    if (toastTimer.current) window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 2600);
  }, []);

  const loadPlatforms = useCallback(async () => {
    try {
      const list = (await window.ipcRenderer.publish.platforms()) as PublishPlatform[];
      if (Array.isArray(list)) setPlatforms(list);
    } catch (e) {
      console.warn('Failed to load platforms:', e);
    }
  }, []);

  const loadItems = useCallback(async () => {
    const seq = loadSeq.current + 1;
    loadSeq.current = seq;
    setLoading(true);
    setError(null);
    try {
      const result = (await window.ipcRenderer.publish.list()) as {
        success: boolean;
        items: PublishRecord[];
      };
      if (seq !== loadSeq.current) return;
      setItems(Array.isArray(result?.items) ? result.items : []);
    } catch (e) {
      console.error('Failed to load publish log:', e);
      if (seq === loadSeq.current) {
        setItems([]);
        setError(e instanceof Error ? e.message : '加载发布记录失败');
      }
    } finally {
      if (seq === loadSeq.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!isActive) return;
    void loadPlatforms();
    void loadItems();
  }, [isActive, loadPlatforms, loadItems]);

  // 看门狗：兜底解锁 loading，防止 IPC 静默失败导致永久「加载中」
  useEffect(() => {
    if (!isActive) return;
    const timer = window.setTimeout(() => {
      setLoading((prev) => {
        if (prev && itemsRef.current.length === 0) {
          setError((msg) => msg || '请求超时（16s），请检查后端 publish 通道是否注册');
        }
        return false;
      });
    }, 16_000);
    return () => window.clearTimeout(timer);
  }, [isActive, items, error, formOpen]);

  const platformLabel = useCallback(
    (key: string) => platforms.find((p) => p.key === key)?.label ?? key,
    [platforms],
  );
  const consoleUrl = useCallback(
    (key: string) => platforms.find((p) => p.key === key)?.consoleUrl ?? '',
    [platforms],
  );

  const openExternal = useCallback(async (url?: string | null) => {
    if (!url) {
      showToast('err', '没有可打开的链接');
      return;
    }
    try {
      await window.ipcRenderer.openExternalUrl(url);
    } catch (e) {
      console.warn('openExternalUrl failed:', e);
      showToast('err', '打开外部链接失败');
    }
  }, [showToast]);

  const handleSave = useCallback(async () => {
    if (!form.title.trim()) {
      showToast('err', '请填写标题');
      return;
    }
    setSaving(true);
    try {
      if (editing) {
        const result = (await window.ipcRenderer.publish.update({
          id: editing.id,
          platform: form.platform,
          postUrl: form.postUrl || undefined,
          title: form.title.trim(),
          publishedAt: form.publishedAt || undefined,
          status: form.status,
          views: form.views,
          likes: form.likes,
          comments: form.comments,
          collects: form.collects,
          notes: form.notes || undefined,
        })) as { success: boolean; item: PublishRecord };
        if (!result?.success) throw new Error('更新失败');
        showToast('ok', '已更新发布记录');
      } else {
        const result = (await window.ipcRenderer.publish.add({
          platform: form.platform,
          postUrl: form.postUrl || undefined,
          title: form.title.trim(),
          publishedAt: form.publishedAt || undefined,
          status: form.status,
          views: form.views,
          likes: form.likes,
          comments: form.comments,
          collects: form.collects,
          notes: form.notes || undefined,
        })) as { success: boolean; item: PublishRecord };
        if (!result?.success) throw new Error('登记失败');
        showToast('ok', '已登记发布');
      }
      setFormOpen(false);
      setEditing(null);
      setForm({ ...EMPTY_FORM });
      void loadItems();
    } catch (e) {
      console.error('Save publish failed:', e);
      showToast('err', e instanceof Error ? e.message : '保存失败');
    } finally {
      setSaving(false);
    }
  }, [editing, form, showToast, loadItems]);

  const handleRemove = useCallback(
    async (id: string) => {
      // 用应用内对话框而不是 window.confirm：Tauri 的 macOS WKWebView 不实现
      // 原生 confirm 面板，window.confirm 静默返回 false → 点了完全没反应。
      const confirmed = await appConfirm('确定删除这条发布记录？', {
        title: '删除发布记录',
        confirmLabel: '删除',
        cancelLabel: '取消',
        tone: 'danger',
      });
      if (!confirmed) return;
      try {
        const result = (await window.ipcRenderer.publish.remove(id)) as {
          success: boolean;
          removed: number;
        };
        if (!result?.success) throw new Error('删除失败');
        showToast('ok', '已删除');
        void loadItems();
      } catch (e) {
        console.error('Remove publish failed:', e);
        showToast('err', e instanceof Error ? e.message : '删除失败');
      }
    },
    [showToast, loadItems],
  );

  const openAdd = useCallback(() => {
    setEditing(null);
    setForm({ ...EMPTY_FORM });
    setFormOpen(true);
  }, []);

  const openEdit = useCallback((record: PublishRecord) => {
    setEditing(record);
    setForm({
      platform: record.platform,
      postUrl: record.postUrl ?? '',
      title: record.title,
      publishedAt: record.publishedAt ?? '',
      status: record.status,
      views: record.views,
      likes: record.likes,
      comments: record.comments,
      collects: record.collects,
      notes: record.notes ?? '',
    });
    setFormOpen(true);
  }, []);

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between px-5 py-4 border-b border-border">
        <div className="flex items-center gap-2">
          <Send className="w-4 h-4 text-accent-primary" strokeWidth={1.8} />
          <h2 className="text-sm font-semibold text-text-primary">发布登记</h2>
          <span className="text-[11px] text-text-tertiary">{items.length}</span>
        </div>
        <button
          onClick={openAdd}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-lg bg-accent-primary text-white hover:bg-accent-primary/90 transition-colors"
        >
          {formOpen ? <X className="w-3.5 h-3.5" /> : <Plus className="w-3.5 h-3.5" />}
          {formOpen ? '收起' : '登记发布'}
        </button>
      </div>

      {toast && (
        <div
          className={clsx(
            'mx-5 mt-3 px-3 py-2 text-[12px] rounded-lg',
            toast.kind === 'ok'
              ? 'bg-emerald-50 text-emerald-700'
              : 'bg-red-50 text-red-700',
          )}
        >
          {toast.text}
        </div>
      )}

      {formOpen && (
        <div className="mx-5 mt-3 p-4 rounded-lg border border-border bg-surface-primary space-y-3">
          <div className="grid grid-cols-2 gap-3">
            <label className="flex flex-col gap-1 text-[11px] text-text-tertiary">
              平台
              <select
                value={form.platform}
                onChange={(e) => setForm((f) => ({ ...f, platform: e.target.value }))}
                className="px-2 py-1.5 rounded-md border border-border bg-surface-secondary text-sm text-text-primary"
              >
                {platforms.map((p) => (
                  <option key={p.key} value={p.key}>
                    {p.label}
                  </option>
                ))}
              </select>
            </label>
            <label className="flex flex-col gap-1 text-[11px] text-text-tertiary">
              状态
              <select
                value={form.status}
                onChange={(e) => setForm((f) => ({ ...f, status: e.target.value }))}
                className="px-2 py-1.5 rounded-md border border-border bg-surface-secondary text-sm text-text-primary"
              >
                <option value="planned">待发布</option>
                <option value="published">已发布</option>
                <option value="failed">失败</option>
              </select>
            </label>
          </div>
          <label className="flex flex-col gap-1 text-[11px] text-text-tertiary">
            标题
            <input
              value={form.title}
              onChange={(e) => setForm((f) => ({ ...f, title: e.target.value }))}
              placeholder="发布内容标题"
              className="px-2 py-1.5 rounded-md border border-border bg-surface-secondary text-sm text-text-primary"
            />
          </label>
          <div className="grid grid-cols-2 gap-3">
            <label className="flex flex-col gap-1 text-[11px] text-text-tertiary">
              发布链接
              <input
                value={form.postUrl}
                onChange={(e) => setForm((f) => ({ ...f, postUrl: e.target.value }))}
                placeholder="https://..."
                className="px-2 py-1.5 rounded-md border border-border bg-surface-secondary text-sm text-text-primary"
              />
            </label>
            <label className="flex flex-col gap-1 text-[11px] text-text-tertiary">
              发布时间
              <input
                type="date"
                value={form.publishedAt}
                onChange={(e) => setForm((f) => ({ ...f, publishedAt: e.target.value }))}
                className="px-2 py-1.5 rounded-md border border-border bg-surface-secondary text-sm text-text-primary"
              />
            </label>
          </div>
          <div className="grid grid-cols-4 gap-2">
            {(['views', 'likes', 'comments', 'collects'] as const).map((m) => (
              <label key={m} className="flex flex-col gap-1 text-[11px] text-text-tertiary">
                {METRIC_LABELS[m]}
                <input
                  type="number"
                  min={0}
                  value={form[m]}
                  onChange={(e) =>
                    setForm((f) => ({ ...f, [m]: Number(e.target.value) || 0 }))
                  }
                  className="px-2 py-1.5 rounded-md border border-border bg-surface-secondary text-sm text-text-primary"
                />
              </label>
            ))}
          </div>
          <label className="flex flex-col gap-1 text-[11px] text-text-tertiary">
            备注
            <textarea
              value={form.notes}
              onChange={(e) => setForm((f) => ({ ...f, notes: e.target.value }))}
              rows={2}
              placeholder="可选：发布要点 / 数据小结"
              className="px-2 py-1.5 rounded-md border border-border bg-surface-secondary text-sm text-text-primary resize-none"
            />
          </label>
          <div className="flex justify-end gap-2">
            <button
              onClick={() => {
                setFormOpen(false);
                setEditing(null);
              }}
              className="px-3 py-1.5 text-sm rounded-lg border border-border text-text-secondary hover:text-text-primary transition-colors"
            >
              取消
            </button>
            <button
              onClick={() => void handleSave()}
              disabled={saving}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 text-sm rounded-lg bg-accent-primary text-white hover:bg-accent-primary/90 transition-colors disabled:opacity-50"
            >
              {saving ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : <Check className="w-3.5 h-3.5" />}
              {editing ? '保存修改' : '登记'}
            </button>
          </div>
        </div>
      )}

      <div className="flex-1 overflow-auto p-3 space-y-2">
        {loading && (
          <div className="flex items-center gap-2 text-[12px] text-text-tertiary px-2 py-6">
            <Loader2 className="w-3.5 h-3.5 animate-spin" /> 加载中…
          </div>
        )}
        {!loading && error && (
          <div className="text-[12px] text-red-600 px-2 py-6">{error}</div>
        )}
        {!loading && !error && items.length === 0 && (
          <div className="flex flex-col items-center gap-2 text-text-tertiary px-2 py-10">
            <Megaphone className="w-6 h-6" strokeWidth={1.5} />
            <p className="text-[12px]">还没有发布记录，点右上角「登记发布」开始</p>
          </div>
        )}
        {!loading &&
          items.map((record) => {
            const status = STATUS_META[record.status] ?? STATUS_META.planned;
            const consoleUrlValue = consoleUrl(record.platform);
            return (
              <div
                key={record.id}
                className="rounded-lg border border-border bg-surface-primary p-3 space-y-2"
              >
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span className="text-[10px] px-1.5 py-0.5 rounded-full bg-surface-secondary text-text-secondary">
                        {platformLabel(record.platform)}
                      </span>
                      <span
                        className={clsx(
                          'text-[10px] px-1.5 py-0.5 rounded-full',
                          status.cls,
                        )}
                      >
                        {status.label}
                      </span>
                    </div>
                    <p className="mt-1 text-sm text-text-primary truncate">
                      {record.title || '(无标题)'}
                    </p>
                    {record.publishedAt && (
                      <p className="text-[11px] text-text-tertiary">
                        发布时间：{record.publishedAt}
                      </p>
                    )}
                  </div>
                  <div className="flex items-center gap-1 shrink-0">
                    {consoleUrlValue && (
                      <button
                        onClick={() => void openExternal(consoleUrlValue)}
                        title="打开创作者后台"
                        className="p-1.5 text-text-tertiary hover:text-accent-primary hover:bg-surface-secondary rounded transition-colors"
                      >
                        <ExternalLink className="w-3.5 h-3.5" />
                      </button>
                    )}
                    {record.postUrl && (
                      <button
                        onClick={() => void openExternal(record.postUrl)}
                        title="打开发布链接"
                        className="p-1.5 text-text-tertiary hover:text-accent-primary hover:bg-surface-secondary rounded transition-colors"
                      >
                        <Send className="w-3.5 h-3.5" />
                      </button>
                    )}
                    <button
                      onClick={() => openEdit(record)}
                      title="编辑"
                      className="p-1.5 text-text-tertiary hover:text-accent-primary hover:bg-surface-secondary rounded transition-colors"
                    >
                      <Pencil className="w-3.5 h-3.5" />
                    </button>
                    <button
                      onClick={() => void handleRemove(record.id)}
                      title="删除"
                      className="p-1.5 text-text-tertiary hover:text-red-500 hover:bg-surface-secondary rounded transition-colors"
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                </div>
                {(record.views > 0 ||
                  record.likes > 0 ||
                  record.comments > 0 ||
                  record.collects > 0) && (
                  <div className="flex flex-wrap gap-3 text-[11px] text-text-tertiary">
                    {record.views > 0 && <span>播放 {record.views}</span>}
                    {record.likes > 0 && <span>赞 {record.likes}</span>}
                    {record.comments > 0 && <span>评 {record.comments}</span>}
                    {record.collects > 0 && <span>藏 {record.collects}</span>}
                  </div>
                )}
                {record.notes && (
                  <p className="text-[11px] text-text-tertiary leading-relaxed">
                    {record.notes}
                  </p>
                )}
              </div>
            );
          })}
      </div>
    </div>
  );
}
