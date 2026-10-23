import { useEffect, useState } from 'react';
import {
  BookOpen,
  Calendar,
  ChevronRight,
  Compass,
  FileEdit,
  Image as ImageIcon,
  Lightbulb,
  Scissors,
  Sparkles,
  X,
} from 'lucide-react';
import { useI18n, type I18nKey } from '../../i18n';
import { dispatchAppIntent } from '../app-shell/appIntent';
import { useWorkflowDrawerStore } from './store';
import { resolveWorkflowStepAction, type WorkflowStepId } from './steps';

// 与 README「运营工作流」7 步一致；每步绑一个真实的"最近入口"。
// 入口选择遵循 Beav 2.5.0 实际架构：能跳到 ViewType 就用 view.open；
// 当前 ViewType 没有的（运营日历等）走"最接近"的现有模块。
const STEPS: Array<{ id: WorkflowStepId; icon: typeof Compass }> = [
  { id: 'collect', icon: BookOpen },
  { id: 'topics', icon: Lightbulb },
  { id: 'planning', icon: Sparkles },
  { id: 'write', icon: FileEdit },
  { id: 'image', icon: ImageIcon },
  { id: 'edit', icon: Scissors },
  { id: 'schedule', icon: Calendar },
];

type StepI18nKeys = {
  titleKey: I18nKey;
  descKey: I18nKey;
  entryKey: I18nKey;
};

// 显式声明每个步骤对应的 i18n key，避免模板字符串拼出的 key 不在 I18nKey 联合类型里。
const WORKFLOW_STEP_I18N: StepI18nKeys[] = [
  { titleKey: 'workflow.step1.title', descKey: 'workflow.step1.desc', entryKey: 'workflow.step1.entry' },
  { titleKey: 'workflow.step2.title', descKey: 'workflow.step2.desc', entryKey: 'workflow.step2.entry' },
  { titleKey: 'workflow.step3.title', descKey: 'workflow.step3.desc', entryKey: 'workflow.step3.entry' },
  { titleKey: 'workflow.step4.title', descKey: 'workflow.step4.desc', entryKey: 'workflow.step4.entry' },
  { titleKey: 'workflow.step5.title', descKey: 'workflow.step5.desc', entryKey: 'workflow.step5.entry' },
  { titleKey: 'workflow.step6.title', descKey: 'workflow.step6.desc', entryKey: 'workflow.step6.entry' },
  { titleKey: 'workflow.step7.title', descKey: 'workflow.step7.desc', entryKey: 'workflow.step7.entry' },
];

type DrawerPhase = 'opening' | 'open' | 'closing';

export function WorkflowDrawer() {
  const { t } = useI18n();
  const drawerOpen = useWorkflowDrawerStore((state) => state.drawerOpen);
  const setDrawerOpen = useWorkflowDrawerStore((state) => state.setDrawerOpen);
  const [visible, setVisible] = useState(drawerOpen);
  const [phase, setPhase] = useState<DrawerPhase>(drawerOpen ? 'open' : 'closing');

  useEffect(() => {
    if (drawerOpen) {
      setVisible(true);
      setPhase('opening');
      const frame = window.requestAnimationFrame(() => {
        window.setTimeout(() => setPhase('open'), 16);
      });
      return () => window.cancelAnimationFrame(frame);
    }
    setPhase('closing');
    const timer = window.setTimeout(() => setVisible(false), 220);
    return () => window.clearTimeout(timer);
  }, [drawerOpen]);

  if (!visible) return null;

  const close = () => setDrawerOpen(false);

  const handleStepClick = (id: WorkflowStepId) => {
    const action = resolveWorkflowStepAction(id);
    if (!action) return;
    if (action.intent) {
      dispatchAppIntent(action.intent);
    } else if (action.view) {
      dispatchAppIntent({ type: 'view.open', view: action.view });
    } else if (action.navigate) {
      action.navigate();
    }
    setDrawerOpen(false);
  };

  return (
    <div
      className="fixed inset-0 z-[1200] flex"
      role="dialog"
      aria-modal="true"
      aria-label={t('workflow.title')}
    >
      <button
        type="button"
        className="flex-1 bg-black/40 transition-opacity duration-200"
        style={{ opacity: phase === 'open' ? 1 : 0 }}
        onClick={close}
        aria-label="close"
      />
      <aside
        className={`w-[420px] max-w-[90vw] h-full bg-[rgb(var(--color-surface-primary))] border-l border-border shadow-2xl transition-transform duration-200 ease-out ${
          phase === 'open' ? 'translate-x-0' : 'translate-x-full'
        }`}
      >
        <header className="flex items-center justify-between px-5 py-4 border-b border-border">
          <div className="flex items-center gap-2">
            <Compass className="w-4 h-4 text-accent-primary" />
            <h2 className="text-sm font-medium text-text-primary">{t('workflow.title')}</h2>
          </div>
          <button
            type="button"
            onClick={close}
            className="p-1 rounded hover:bg-surface-secondary/60 text-text-tertiary hover:text-text-primary"
            aria-label="close"
          >
            <X className="w-4 h-4" />
          </button>
        </header>

        <p className="px-5 py-2 text-xs text-text-tertiary border-b border-border/60">
          {t('workflow.subtitle')}
        </p>

        <ol className="overflow-y-auto h-[calc(100%-92px)] px-3 py-3 space-y-2">
          {STEPS.map((step, index) => {
            const action = resolveWorkflowStepAction(step.id);
            const Icon = step.icon;
            return (
              <li key={step.id}>
                <button
                  type="button"
                  onClick={() => handleStepClick(step.id)}
                  disabled={!action}
                  className="group w-full text-left rounded-lg border border-border bg-surface-primary hover:bg-surface-secondary/50 disabled:opacity-60 disabled:cursor-not-allowed transition-colors p-3"
                >
                  <div className="flex items-start gap-3">
                    <div className="shrink-0 w-8 h-8 rounded-md bg-accent-primary/10 text-accent-primary flex items-center justify-center">
                      <Icon className="w-4 h-4" />
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <span className="text-[10px] font-mono text-text-tertiary">
                          {String(index + 1).padStart(2, '0')}
                        </span>
                        <h3 className="text-sm font-medium text-text-primary">
                          {WORKFLOW_STEP_I18N[index]?.titleKey ? t(WORKFLOW_STEP_I18N[index].titleKey) : ''}
                        </h3>
                      </div>
                      <p className="mt-1 text-xs text-text-secondary leading-relaxed">
                        {WORKFLOW_STEP_I18N[index]?.descKey ? t(WORKFLOW_STEP_I18N[index].descKey) : ''}
                      </p>
                      <div className="mt-2 flex items-center justify-between gap-2">
                        <span className="text-[11px] text-text-tertiary truncate">
                          {WORKFLOW_STEP_I18N[index]?.entryKey ? t(WORKFLOW_STEP_I18N[index].entryKey) : ''}
                        </span>
                        <span className="shrink-0 inline-flex items-center gap-1 text-[11px] text-accent-primary opacity-0 group-hover:opacity-100 transition-opacity">
                          {t('workflow.action.open')}
                          <ChevronRight className="w-3 h-3" />
                        </span>
                      </div>
                    </div>
                  </div>
                </button>
              </li>
            );
          })}
        </ol>
      </aside>
    </div>
  );
}
