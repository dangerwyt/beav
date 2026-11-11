import clsx from 'clsx';
import { Store } from 'lucide-react';
import {
  ECOMMERCE_PLATFORM_GROUPS,
  ECOMMERCE_PLATFORM_IDS,
  ecommercePlatformIconPath,
  type EcommercePlatformsSettings,
} from "../../features/ecommerce-platforms/catalog";

export function EcommercePlatformsSettingsSection({
  settings,
  onTogglePlatform,
}: {
  settings: EcommercePlatformsSettings;
  onTogglePlatform: (platformId: string, enabled: boolean) => void;
}) {
  const enabledCount = ECOMMERCE_PLATFORM_IDS.filter((id) => settings.enabledById[id] !== false).length;
  return (
    <section className="space-y-5">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <h2 className="text-lg font-medium text-text-primary">电商平台</h2>
          <p className="mt-1 text-sm text-text-tertiary">启用的平台会作为后续目标平台生成的候选范围。</p>
        </div>
        <div className="shrink-0 rounded-full border border-border bg-surface-secondary px-3 py-1 text-xs font-medium text-text-secondary">
          {enabledCount}/{ECOMMERCE_PLATFORM_IDS.length} 已开启
        </div>
      </div>

      <div className="space-y-4">
        {ECOMMERCE_PLATFORM_GROUPS.map((group) => {
          const groupEnabledCount = group.platforms.filter((platform) => settings.enabledById[platform.id] !== false).length;
          return (
            <div key={group.region} className="overflow-hidden rounded-xl border border-border bg-surface-primary">
              <div className="flex items-center justify-between gap-3 border-b border-border/70 bg-surface-secondary/40 px-4 py-3">
                <div className="min-w-0">
                  <h3 className="text-sm font-semibold text-text-primary">{group.region}</h3>
                </div>
                <span className="shrink-0 text-xs text-text-tertiary">{groupEnabledCount}/{group.platforms.length}</span>
              </div>
              <div className="divide-y divide-border/70">
                {group.platforms.map((platform) => {
                  const enabled = settings.enabledById[platform.id] !== false;
                  const iconPath = ecommercePlatformIconPath(platform.id);
                  return (
                    <div key={platform.id} className="flex items-center gap-3 px-4 py-3">
                      <div className="flex h-9 w-9 shrink-0 items-center justify-center overflow-hidden rounded-lg border border-border bg-surface-secondary">
                        {iconPath ? (
                          <img src={iconPath} alt="" className="h-6 w-6 object-contain" loading="lazy" />
                        ) : (
                          <Store className="h-4 w-4 text-text-tertiary" />
                        )}
                      </div>
                      <div className="min-w-0 flex-1">
                        <div className="truncate text-sm font-medium text-text-primary">{platform.name}</div>
                        <div className="mt-0.5 truncate text-xs text-text-tertiary">
                          {platform.market} · {platform.platformType}
                        </div>
                      </div>
                      <button
                        type="button"
                        onClick={() => onTogglePlatform(platform.id, !enabled)}
                        className={clsx(
                          'relative h-7 w-[3.25rem] shrink-0 rounded-full transition-colors duration-200',
                          enabled ? 'bg-[#34c759]' : 'bg-[#d1d1d6]'
                        )}
                        title={enabled ? '已开启' : '已关闭'}
                        aria-label={`${enabled ? '关闭' : '开启'} ${platform.name}`}
                      >
                        <span
                          className={clsx(
                            'absolute left-0.5 top-0.5 h-6 w-6 rounded-full bg-white shadow-[0_2px_5px_rgba(0,0,0,0.22)] transition-transform duration-200',
                            enabled ? 'translate-x-6' : 'translate-x-0'
                          )}
                        />
                      </button>
                    </div>
                  );
                })}
              </div>
            </div>
          );
        })}
      </div>
    </section>
  );
}
