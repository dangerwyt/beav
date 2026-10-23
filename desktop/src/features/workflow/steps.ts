import type { ViewType, AppIntent } from '../app-shell/types';

export type WorkflowStepId =
  | 'collect'
  | 'topics'
  | 'planning'
  | 'write'
  | 'image'
  | 'edit'
  | 'schedule';

export type WorkflowStepAction = {
  intent?: AppIntent;
  view?: ViewType;
  navigate?: () => void;
};

/**
 * 7 步运营工作流 → 当前 Beav 2.5.0 实际入口的映射。
 *
 * 约束：
 *  - 只用已有 ViewType + 已有 AppIntent（`view.open` / `redclaw.open` / `settings.open` / `generation.open`）。
 *  - 当前没有 "主页" 和 "运营日历" 的 ViewType，分别用 wander（选题中心）和 automation（自动化）兜底，
 *    并在 UI 上诚实标注。
 *  - "采集与调研"（步骤 1）入口分散：浏览器插件是独立程序、对话是 redclaw、知识库是 knowledge，
 *    默认跳到知识库（最常用），按钮 hover 给出"前往"提示。
 */
export function resolveWorkflowStepAction(id: WorkflowStepId): WorkflowStepAction | null {
  switch (id) {
    case 'collect':
      // 知识库作为"采集结果查看"的最近入口；浏览器插件本身由 Plugin 子包提供。
      return { view: 'knowledge' };

    case 'topics':
      // "主页/从选题开始"在当前 2.5.0 公开版未挂载；选题中心是 wander。
      return { view: 'wander' };

    case 'planning':
      // "账号运营规划"在主页小工具里；当前 2.5.0 公开版未挂载，回退到 settings 的 platforms（账号配置）。
      return { intent: { type: 'settings.open', tab: 'platforms' } };

    case 'write':
      // 写稿：新对话（redclaw 'new' 会拉起新聊天）。
      // 注意：改稿**不走这里**。改稿必须先在 RedClaw 稿件列表里打开某篇稿进入编辑器，
      // 会话才会被 `buildEditorSessionBinding` 绑定成 `agentProfile: 'manuscript-editor'`
      // （allowedTools 仅 workflow、writeTarget=manuscripts://current）；空对话既不知道改哪篇，
      // 也没有写入当前稿件的通道。所以本步骤只标"写稿"。
      return { intent: { type: 'redclaw.open', action: 'new' } };

    case 'image':
      // 媒体工坊 → generation-studio（生图生视频统一入口）。
      return { intent: { type: 'generation.open', intent: { mode: 'image', source: 'standalone' } } };

    case 'edit':
      // 智能剪口播 → generation-studio 视频模式。
      return { intent: { type: 'generation.open', intent: { mode: 'video', source: 'standalone' } } };

    case 'schedule':
      // "运营日历"在当前 2.5.0 公开版没有独立 ViewType，自动化是最近替代。
      return { view: 'automation' };

    default:
      return null;
  }
}
