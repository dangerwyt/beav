export function isTauriRuntime(): boolean {
  if (typeof window === 'undefined') {
    return false;
  }
  const tauriWindow = window as unknown as {
    __TAURI__?: unknown;
    __TAURI_INTERNALS__?: unknown;
  };
  return Boolean(tauriWindow.__TAURI_INTERNALS__ || tauriWindow.__TAURI__);
}

function readInjectedIpcBaseUrl(): string {
  if (typeof window === 'undefined') return '';
  const injected = (window as unknown as { __BEAV_IPC_BASE__?: unknown }).__BEAV_IPC_BASE__;
  return typeof injected === 'string' ? injected.trim() : '';
}

function readEnvIpcBaseUrl(): string {
  try {
    // 构建期注入（Vite 会把它替换成字面量）；非 Vite 环境访问会抛错，兜底为空。
    return String(import.meta.env.VITE_BROWSER_IPC_BASE || '').trim();
  } catch {
    return '';
  }
}

/**
 * 浏览器模式（非 Tauri 环境）下的 IPC 基地址：
 * 1) `window.__BEAV_IPC_BASE__` 运行时注入（优先级最高）
 * 2) `VITE_BROWSER_IPC_BASE` 构建期注入
 * 3) 默认同源 `/api/ipc` —— 由 `desktop/scripts/web-server.mjs` 反代到本机 daemon
 *
 * 桌面端（Tauri）不会读取这里，行为完全不变。
 */
function defaultIpcBaseUrl(): string {
  // Vite dev 直接在浏览器打开（1420）时，沿用旧行为：直连本机 daemon。
  try {
    if (import.meta.env.DEV) return 'http://127.0.0.1:31937/api/ipc';
  } catch {
    /* 非 Vite 环境，忽略 */
  }
  // 生产构建：默认同源，由 scripts/web-server.mjs 反代。
  return '/api/ipc';
}

export const BROWSER_IPC_BASE_URL =
  readInjectedIpcBaseUrl() || readEnvIpcBaseUrl() || defaultIpcBaseUrl();

/** 浏览器模式：把本机绝对路径转成同源文件代理地址（由 web-server 读取磁盘并按白名单返回）。 */
export function browserFileUrl(absolutePath: string): string {
  const path = String(absolutePath || '').trim();
  if (!path) return '';
  // 与 IPC 同一基座：DEV 直连 31937，生产构建走同源（由 web-server 反代）。
  const base = BROWSER_IPC_BASE_URL.replace(/\/api\/ipc$/, '');
  return `${base}/api/file?path=${encodeURIComponent(path)}`;
}

async function postIpc(baseUrl: string, channel: string, payload: unknown) {
  const response = await fetch(`${baseUrl}/invoke`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ channel, payload: payload ?? null }),
  });
  const value = await response.json().catch(() => null);
  return { response, value };
}

function ipcErrorMessage(response: Response, value: any): string {
  const message = value && typeof value === 'object' && 'error' in value
    ? String((value as { error?: unknown }).error || response.statusText)
    : response.statusText;
  return message || `HTTP ${response.status}`;
}

function loopbackDaemonBaseUrl(): string {
  const host = typeof location !== 'undefined' && location.hostname ? location.hostname : '127.0.0.1';
  return `http://${host}:31937/api/ipc`;
}

export async function invokeBrowserHost(channel: string, payload?: unknown): Promise<any> {
  let primary: { response: Response; value: any } | null = null;
  try {
    primary = await postIpc(BROWSER_IPC_BASE_URL, channel, payload);
  } catch {
    primary = null;
  }
  if (primary?.response.ok) return primary.value;

  // 同源没命中（例如直接用 Vite dev/preview 打开页面，/api/ipc 会 404）：
  // 自动回退直连本机 daemon，避免整页 IPC 全 404。
  const status = primary?.response.status ?? 0;
  if (status === 404 || status === 0) {
    const fallback = loopbackDaemonBaseUrl();
    if (fallback !== BROWSER_IPC_BASE_URL) {
      const retry = await postIpc(fallback, channel, payload);
      if (retry.response.ok) return retry.value;
      throw new Error(ipcErrorMessage(retry.response, retry.value));
    }
  }
  if (primary) throw new Error(ipcErrorMessage(primary.response, primary.value));
  throw new Error('无法连接 Beav 服务（daemon 未启动？）');
}

export function browserPayloadForCommand(command: string, args?: unknown): unknown {
  if (
    command.startsWith('knowledge_')
    && args
    && typeof args === 'object'
    && Object.prototype.hasOwnProperty.call(args, 'payload')
  ) {
    return (args as { payload?: unknown }).payload ?? null;
  }
  return args;
}
