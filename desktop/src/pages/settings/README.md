# `src/pages/settings/`

本目录承载设置页的大块分区和设置页内部共享 UI/类型。

## Current Files

- `SettingsSections.tsx`
- `shared.tsx`
- `../Settings.tsx`：设置页入口，只组合 controller 与 view，保持对外 props 不变。
- `SettingsPageView.tsx`：页面布局、表单和弹窗，props 由 controller 返回类型推导。
- `useSettingsController.tsx`：设置加载/保存、分区切换、按需刷新及尚未独立的分区逻辑。
- `useSettingsAi.tsx`：AI 来源、模型路由、模型列表与自动保存。
- `useSettingsRuntime.ts`：任务、会话、后台队列、工具诊断、性能采样和日志。
- `useSettingsCliRuntime.ts`：CLI 环境、安装队列及诊断。
- `useSettingsTeam.ts`：团队顾问状态和编辑操作。
- `TeamSettingsSection.tsx` / `EcommercePlatformsSettingsSection.tsx`：团队和电商平台分区视图。
- `settingsSupport.ts`：上述分区共用的类型、常量和纯辅助函数。
- `../../features/settings/settingsModel.ts`：设置页纯 model/helper，包含价格表、AI route、MCP draft 和 runtime perf preset。

## Characteristics

- 数据域很多，依赖多个 host channel
- 必须允许局部刷新，不能因为单个设置区失败而把整个设置页打空
- 常见区域包括代理、插件、守护进程、MCP、runtime 诊断、记忆维护等
- ToolRouter / tool plan 属于底层日志和 checkpoint 审计信息，不新增用户可见设置项或诊断面板

## Rules

- 设置页说明文字只描述用户可配置项和后果，不写开发实现说明。
- 与宿主状态强绑定的设置项，必须提供默认值和降级显示。
- 纯解析/格式化/默认值逻辑优先放到 `features/settings/settingsModel.ts`，页面只做加载、保存和 section composition。
- Hook 保持无条件调用；分区是否可见、页面是否激活的判断放在各自 Effect 内，保留清理函数。跨分区加载/保存由 controller 编排。

## Verification

- 打开设置页无阻塞
- 单个分区失败不影响其他分区显示
- 保存、刷新、状态轮询行为正确
