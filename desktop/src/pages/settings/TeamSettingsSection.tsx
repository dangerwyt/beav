import clsx from 'clsx';
import { GripVertical, Plus, RefreshCw, Settings as SettingsIcon } from 'lucide-react';
import { APP_BRAND } from "../../config/brand";
import { hasRenderableAssetUrl, resolveAssetUrl } from "../../utils/pathManager";
import { type Advisor } from "../Advisors";
import { advisorAvatarLabel } from './settingsSupport';

export function TeamSettingsSection({
  advisors,
  loading,
  busyAdvisorId,
  draggingAdvisorId,
  onCreateAdvisor,
  onToggleVisible,
  onOpenSettings,
  onDragStart,
  onDragOver,
  onDragEnd,
}: {
  advisors: Advisor[];
  loading: boolean;
  busyAdvisorId: string | null;
  draggingAdvisorId: string | null;
  onCreateAdvisor: () => void;
  onToggleVisible: (advisor: Advisor) => void;
  onOpenSettings: (advisor: Advisor) => void;
  onDragStart: (advisorId: string) => void;
  onDragOver: (advisorId: string) => void;
  onDragEnd: () => void;
}) {
  return (
    <section className="space-y-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <h2 className="text-lg font-medium text-text-primary">团队</h2>
          <p className="mt-1 text-sm text-text-tertiary">管理 {APP_BRAND.aiDisplayName} 新对话里出现的成员和顺序。</p>
        </div>
        <button
          type="button"
          onClick={onCreateAdvisor}
          className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md border border-border px-3 text-xs font-medium text-text-secondary transition-colors hover:bg-surface-secondary hover:text-text-primary"
        >
          <Plus className="h-3.5 w-3.5" />
          新增成员
        </button>
      </div>

      <div className="overflow-hidden rounded-xl border border-border bg-surface-primary">
        {loading ? (
          <div className="flex items-center gap-2 px-4 py-5 text-sm text-text-tertiary">
            <RefreshCw className="h-4 w-4 animate-spin" />
            正在读取成员
          </div>
        ) : advisors.length === 0 ? (
          <div className="flex flex-col items-center gap-3 px-4 py-8 text-center text-sm text-text-tertiary">
            <span>暂无成员</span>
            <button
              type="button"
              onClick={onCreateAdvisor}
              className="inline-flex h-8 items-center gap-1.5 rounded-md border border-border px-3 text-xs font-medium text-text-secondary transition-colors hover:bg-surface-secondary hover:text-text-primary"
            >
              <Plus className="h-3.5 w-3.5" />
              新增成员
            </button>
          </div>
        ) : (
          <div className="divide-y divide-border/70">
            {advisors.map((advisor) => {
              const visible = advisor.redclawVisible !== false;
              const busy = busyAdvisorId === advisor.id;
              const isDragging = draggingAdvisorId === advisor.id;
              return (
                <div
                  key={advisor.id}
                  draggable
                  onDragStart={(event) => {
                    event.dataTransfer.effectAllowed = 'move';
                    event.dataTransfer.setData('text/plain', advisor.id);
                    onDragStart(advisor.id);
                  }}
                  onDragOver={(event) => {
                    event.preventDefault();
                    event.dataTransfer.dropEffect = 'move';
                    onDragOver(advisor.id);
                  }}
                  onDragEnter={(event) => {
                    event.preventDefault();
                    onDragOver(advisor.id);
                  }}
                  onDrop={(event) => event.preventDefault()}
                  onDragEnd={onDragEnd}
                  className={clsx(
                    'flex items-center gap-3 px-3 py-3 transition-colors',
                    isDragging ? 'bg-surface-secondary/80' : 'bg-surface-primary'
                  )}
                >
                  <button
                    type="button"
                    draggable
                    onDragStart={(event) => {
                      event.dataTransfer.effectAllowed = 'move';
                      event.dataTransfer.setData('text/plain', advisor.id);
                      onDragStart(advisor.id);
                    }}
                    onDragEnd={onDragEnd}
                    className="flex h-8 w-8 shrink-0 cursor-grab items-center justify-center rounded-md text-text-tertiary transition-colors hover:bg-surface-secondary hover:text-text-primary active:cursor-grabbing"
                    title="拖动排序"
                    aria-label="拖动排序"
                  >
                    <GripVertical className="h-4 w-4" />
                  </button>

                  <div className="h-10 w-10 shrink-0 overflow-hidden rounded-full bg-surface-secondary text-sm font-semibold text-text-secondary">
                    {hasRenderableAssetUrl(advisor.avatar) ? (
                      <img src={resolveAssetUrl(advisor.avatar)} alt="" className="h-full w-full object-cover" />
                    ) : (
                      <div className="flex h-full w-full items-center justify-center">{advisorAvatarLabel(advisor)}</div>
                    )}
                  </div>

                  <div className="min-w-0 flex-1">
                    <div className="truncate text-sm font-semibold text-text-primary">{advisor.name || '未命名成员'}</div>
                    <div className="mt-0.5 truncate text-xs text-text-tertiary">{advisor.personality || '未设置描述'}</div>
                  </div>

                  <button
                    type="button"
                    onClick={() => onToggleVisible(advisor)}
                    disabled={busy}
                    className={clsx(
                      'relative h-7 w-[3.25rem] shrink-0 rounded-full transition-colors duration-200 disabled:opacity-50',
                      visible ? 'bg-[#34c759]' : 'bg-[#d1d1d6]'
                    )}
                    title={visible ? '已展示' : '已隐藏'}
                    aria-label={visible ? `在 ${APP_BRAND.aiDisplayName} 展示` : `不在 ${APP_BRAND.aiDisplayName} 展示`}
                  >
                    <span
                      className={clsx(
                        'absolute left-0.5 top-0.5 h-6 w-6 rounded-full bg-white shadow-[0_2px_5px_rgba(0,0,0,0.22)] transition-transform duration-200',
                        visible ? 'translate-x-6' : 'translate-x-0'
                      )}
                    />
                  </button>

                  <button
                    type="button"
                    onClick={() => onOpenSettings(advisor)}
                    className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-text-tertiary transition-colors hover:bg-surface-secondary hover:text-text-primary"
                    title="设置成员"
                    aria-label="设置成员"
                  >
                    <SettingsIcon className="h-4 w-4" />
                  </button>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </section>
  );
}
