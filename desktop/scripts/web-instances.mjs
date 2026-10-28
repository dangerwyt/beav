#!/usr/bin/env node
/**
 * Beav 多用户实例管理器（方案 B：每用户一个隐藏实例 + 独立数据目录）
 *
 * 职责：
 *  1) 账号：本地 users.json（scrypt 口令）+ 登录 cookie
 *  2) 路由：按登录用户把 /api/* 反代到该用户实例（127.0.0.1:<port>）
 *  3) 生命周期：首次访问拉起隐藏实例，闲置 IDLE_MINUTES 分钟回收
 *  4) 隔离：每用户独立 APPDATA / store / workspace / exports / Chrome profile / WebView2
 *
 * 用法：
 *  node scripts/web-instances.mjs --create alice 密码 [端口]
 *  node scripts/web-instances.mjs --set-password alice 新密码
 *  node scripts/web-instances.mjs --list
 *  node scripts/web-instances.mjs                # 启动服务（默认 8090）
 *
 * 环境变量：
 *  WEB_HOST / WEB_PORT              监听地址/端口（默认 0.0.0.0:8090）
 *  WEB_DIST                         前端 dist 目录
 *  BEAV_EXE                         实例可执行文件（默认自动找 release / tauri-dev / debug）
 *  BEAV_USERS_FILE                  用户表路径（默认 desktop/users.json）
 *  BEAV_USERS_ROOT                  用户数据根（默认 D:\beav_workspace\users）
 *  BEAV_INSTANCE_BASE_PORT          实例端口起点（默认 32001）
 *  BEAV_IDLE_MINUTES                闲置回收分钟（默认 30）
 *  BEAV_SESSION_HOURS               登录有效期小时（默认 72）
 *  BEAV_SESSION_SECRET              会话签名密钥（不设则落盘 .beav-session-secret）
 */
import http from 'node:http';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFile, spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const DESKTOP_ROOT = path.resolve(__dirname, '..');
const DIST_DIR = path.resolve(process.env.WEB_DIST || path.join(DESKTOP_ROOT, 'dist'));
const USERS_FILE = path.resolve(
  process.env.BEAV_USERS_FILE || path.join(DESKTOP_ROOT, 'users.json'),
);
const DEFAULT_USERS_ROOT =
  process.platform === 'win32'
    ? path.join('D:', 'beav_workspace', 'users')
    : path.join(os.homedir(), '.beav-workspace', 'users');
const USERS_ROOT = path.resolve(process.env.BEAV_USERS_ROOT || DEFAULT_USERS_ROOT);
const WEB_HOST = process.env.WEB_HOST || '0.0.0.0';
const WEB_PORT = Number(process.env.WEB_PORT || 8090);
const BASE_PORT = Number(process.env.BEAV_INSTANCE_BASE_PORT || 32001);
const IDLE_MINUTES = Number(process.env.BEAV_IDLE_MINUTES ?? 30);
// 夜间预热：默认 23:55 拉起所有用户实例，保活到 03:00，保证 00:00 的每日增量能跑。
// 置空（BEAV_PREWARM_AT=）可关闭。
const PREWARM_AT = String(process.env.BEAV_PREWARM_AT ?? '23:55').trim();
const KEEP_UNTIL = String(process.env.BEAV_KEEP_UNTIL ?? '03:00').trim();
// 集中触发每日任务：到点对每个用户实例调用 monitor:run-daily（置空关闭）。
const DAILY_AT = String(process.env.BEAV_DAILY_AT ?? '00:00').trim();
// 单实例多租户：所有 web 用户共用桌面端 daemon，只靠 X-Beav-User 区分数据。
const SINGLE_DAEMON = process.env.BEAV_SINGLE_DAEMON === '1';
const DAEMON_URL = new URL(process.env.BEAV_DAEMON || 'http://127.0.0.1:31937');
const DAEMON_PORT = Number(DAEMON_URL.port || 31937);
const SESSION_HOURS = Number(process.env.BEAV_SESSION_HOURS || 72);
const SECRET_FILE = path.resolve(
  process.env.BEAV_SESSION_SECRET_FILE || path.join(DESKTOP_ROOT, '.beav-session-secret'),
);

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.webp': 'image/webp',
  '.gif': 'image/gif',
  '.ico': 'image/x-icon',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
  '.ttf': 'font/ttf',
  '.map': 'application/json; charset=utf-8',
};

function log(...args) {
  console.log('[multi]', ...args);
}

// ---------------- 账号 ----------------

function hashPassword(password) {
  const salt = crypto.randomBytes(16).toString('hex');
  const hash = crypto.scryptSync(String(password), salt, 64).toString('hex');
  return `scrypt$${salt}$${hash}`;
}

function verifyPassword(password, stored) {
  const [algo, salt, hash] = String(stored || '').split('$');
  if (algo !== 'scrypt' || !salt || !hash) return false;
  const actual = crypto.scryptSync(String(password), salt, 64);
  const expected = Buffer.from(hash, 'hex');
  return actual.length === expected.length && crypto.timingSafeEqual(actual, expected);
}

function loadUsers() {
  if (!fs.existsSync(USERS_FILE)) return { users: [] };
  const raw = JSON.parse(fs.readFileSync(USERS_FILE, 'utf8'));
  if (!Array.isArray(raw.users)) raw.users = [];
  return raw;
}

function saveUsers(data) {
  fs.writeFileSync(USERS_FILE, JSON.stringify(data, null, 2), 'utf8');
}

function ensureUsersFile() {
  if (fs.existsSync(USERS_FILE)) return;
  const password = crypto.randomBytes(9).toString('base64url');
  saveUsers({
    users: [
      {
        username: 'admin',
        uid: 'admin',
        role: 'admin',
        port: BASE_PORT,
        passwordHash: hashPassword(password),
        createdAt: new Date().toISOString(),
      },
    ],
  });
  log(`已创建用户表 ${USERS_FILE}`);
  log(`默认账号 admin / ${password}  ← 请尽快用 --set-password 修改`);
}

function nextPort(users) {
  const used = new Set(users.map((u) => Number(u.port)).filter(Boolean));
  let port = BASE_PORT;
  while (used.has(port)) port += 1;
  return port;
}

// ---------------- 会话 ----------------

function sessionSecret() {
  if (process.env.BEAV_SESSION_SECRET) return process.env.BEAV_SESSION_SECRET;
  if (fs.existsSync(SECRET_FILE)) return fs.readFileSync(SECRET_FILE, 'utf8').trim();
  const secret = crypto.randomBytes(32).toString('hex');
  fs.writeFileSync(SECRET_FILE, secret, { mode: 0o600 });
  return secret;
}

function signSession(username) {
  const payload = Buffer.from(
    JSON.stringify({ u: username, exp: Date.now() + SESSION_HOURS * 3_600_000 }),
  ).toString('base64url');
  const sig = crypto.createHmac('sha256', sessionSecret()).update(payload).digest('base64url');
  return `${payload}.${sig}`;
}

function verifySession(token) {
  const [payload, sig] = String(token || '').split('.');
  if (!payload || !sig) return null;
  const expected = crypto
    .createHmac('sha256', sessionSecret())
    .update(payload)
    .digest('base64url');
  const a = Buffer.from(sig);
  const b = Buffer.from(expected);
  if (a.length !== b.length || !crypto.timingSafeEqual(a, b)) return null;
  try {
    const data = JSON.parse(Buffer.from(payload, 'base64url').toString('utf8'));
    if (!data.u || Number(data.exp) < Date.now()) return null;
    return data.u;
  } catch {
    return null;
  }
}

function parseCookies(header) {
  const out = {};
  for (const part of String(header || '').split(';')) {
    const index = part.indexOf('=');
    if (index > 0) out[part.slice(0, index).trim()] = decodeURIComponent(part.slice(index + 1).trim());
  }
  return out;
}

// ---------------- 实例 ----------------

/** 跨平台可执行文件解析：Windows 是 beav.exe，macOS/Linux 是 beav（无扩展名）。 */
function resolveExe() {
  if (process.env.BEAV_EXE) return path.resolve(process.env.BEAV_EXE);
  const base = path.join(DESKTOP_ROOT, 'src-tauri', 'target');
  const names =
    process.platform === 'win32'
      ? ['beav.exe', 'beaw.exe', 'redbox.exe', 'thrive.exe']
      : ['beav', 'beaw', 'redbox', 'thrive'];
  const dirs = [
    path.join(base, 'release'),
    path.join(base, 'tauri-dev', 'debug'),
    path.join(base, 'debug'),
  ];

  for (const dir of dirs) {
    for (const name of names) {
      const candidate = path.join(dir, name);
      if (fs.existsSync(candidate)) return candidate;
    }
    // macOS .app 包
    if (process.platform === 'darwin') {
      const bundleDir = path.join(dir, 'bundle', 'macos');
      if (fs.existsSync(bundleDir)) {
        for (const entry of fs.readdirSync(bundleDir)) {
          if (!entry.endsWith('.app')) continue;
          const macosDir = path.join(bundleDir, entry, 'Contents', 'MacOS');
          if (!fs.existsSync(macosDir)) continue;
          for (const bin of fs.readdirSync(macosDir)) {
            const candidate = path.join(macosDir, bin);
            try {
              if (fs.statSync(candidate).isFile()) return candidate;
            } catch {}
          }
        }
      }
    }
    // 兜底：目录下任意可执行文件
    if (fs.existsSync(dir)) {
      for (const entry of fs.readdirSync(dir)) {
        if (entry.includes('.')) continue;
        const candidate = path.join(dir, entry);
        try {
          const stat = fs.statSync(candidate);
          if (stat.isFile() && (process.platform === 'win32' || (stat.mode & 0o111))) {
            return candidate;
          }
        } catch {}
      }
    }
  }
  return null;
}

function userDir(user) {
  return path.join(USERS_ROOT, user.uid || user.username);
}

function ensureUserDirs(user) {
  const dir = userDir(user);
  for (const sub of [
    'appdata',
    'home',
    'state',
    'workspace',
    'exports',
    'chrome-profile',
    'webview',
  ]) {
    fs.mkdirSync(path.join(dir, sub), { recursive: true });
  }
  return dir;
}

function instanceEnv(user) {
  const dir = ensureUserDirs(user);
  const env = {
    ...process.env,
    BEAV_STORE_DIR: path.join(dir, 'state'),
    BEAV_WORKSPACE_ROOT: path.join(dir, 'workspace'),
    BEAV_EXPORT_ROOT: path.join(dir, 'exports'),
    BEAV_CHROME_PROFILE_DIR: path.join(dir, 'chrome-profile'),
    BEAV_WEBVIEW_DATA_DIR: path.join(dir, 'webview'),
    BEAV_DAEMON_PORT: String(user.port),
    BEAV_HEADLESS: '1',
    BEAV_MANAGED_DIRS: '1',
    // 不需要官方功能：跳过登录/扫码闸门，直接进工作台。
    BEAV_SKIP_OFFICIAL_LOGIN: '1',
    // 每日任务由管理器集中触发，实例内定时器关闭，避免重复跑。
    BEAV_DISABLE_DAILY_SCHEDULER: '1',
  };
  // 隔离 Tauri 的 app_data（official auth / acp 配置 / 日志队列等）。
  if (process.platform === 'win32') {
    env.APPDATA = path.join(dir, 'appdata');
  } else {
    env.HOME = path.join(dir, 'home');
    if (process.platform !== 'darwin') {
      env.XDG_CONFIG_HOME = path.join(dir, 'home', '.config');
    }
  }
  return env;
}

const instances = new Map(); // uid -> { child, port, lastActive }

async function healthOk(port) {
  try {
    const response = await fetch(`http://127.0.0.1:${port}/api/ipc/health`, {
      signal: AbortSignal.timeout(1500),
    });
    return response.ok;
  } catch {
    return false;
  }
}

function execFileText(file, args, timeout = 5000) {
  return new Promise((resolve) => {
    execFile(
      file,
      args,
      { windowsHide: true, timeout, maxBuffer: 8 * 1024 * 1024 },
      (error, stdout) => resolve(error ? '' : String(stdout || '')),
    );
  });
}

/** 进程常驻内存（MB）。Windows 用 tasklist，其余用 ps。 */
async function getMemoryMB(pid) {
  if (!pid) return null;
  if (process.platform === 'win32') {
    const stdout = await execFileText('tasklist', [
      '/FI',
      `PID eq ${pid}`,
      '/FO',
      'CSV',
      '/NH',
    ]);
    // "beav.exe","12345","Console","1","123,456 K"
    const match = stdout.trim().match(/^"[^"]*","(\d+)","[^"]*","\d+","([\d.,]+)\s*K"/m);
    if (!match) return null;
    return Math.round(Number(match[2].replace(/[.,]/g, '')) / 1024);
  }
  const stdout = await execFileText('ps', ['-o', 'rss=', '-p', String(pid)]);
  const kb = Number(stdout.trim());
  return Number.isFinite(kb) ? Math.round(kb / 1024) : null;
}

/** 按监听端口反查 PID（管理器重启后也能识别既有实例）。 */
async function findPidByPort(port) {
  if (process.platform === 'win32') {
    const stdout = await execFileText('netstat', ['-ano', '-p', 'TCP'], 8000);
    for (const line of stdout.split(/\r?\n/)) {
      if (!line.includes(`:${port}`) || !line.includes('LISTENING')) continue;
      const parts = line.trim().split(/\s+/);
      const pid = Number(parts[parts.length - 1]);
      if (pid) return pid;
    }
    return null;
  }
  const stdout = await execFileText('sh', ['-c', `lsof -nP -iTCP:${port} -sTCP:LISTEN -t 2>/dev/null`]);
  const pid = Number(stdout.trim().split(/\s+/)[0]);
  return Number.isFinite(pid) && pid > 0 ? pid : null;
}

async function instanceStatus(user) {
  const key = user.uid || user.username;
  const record = instances.get(key);
  const running = await healthOk(user.port);
  let pid = record?.child?.pid ?? null;
  if (!pid && running) pid = await findPidByPort(user.port);
  const rssMB = pid ? await getMemoryMB(pid) : null;
  return {
    username: user.username,
    uid: key,
    role: user.role || 'user',
    port: user.port,
    running,
    pid,
    rssMB,
    startedAt: record?.startedAt ?? null,
    lastActive: record?.lastActive ?? null,
    createdAt: user.createdAt ?? null,
  };
}

// 同一用户的并发请求必须共用一次 spawn：页面首屏会同时发几十个 /api/*，
// 没有去重就会瞬间拉起多个实例（只有一个抢到端口，其余变僵尸进程）。
const spawning = new Map();

/** 直接调该用户实例的 IPC（管理器内部用，带 Origin/CORS 与 Content-Length）。 */
/** 单实例模式：所有用户共用 daemon 端口，靠 owner 头区分；多实例模式：各自的 port。 */
function targetOf(user, asAdmin = false) {
  if (SINGLE_DAEMON) {
    // role=admin 的 web 账号也按管理员处理（不加 owner 头 → 看全部），
    // 否则它只会看到 owner=admin 的那部分数据。
    const isAdminUser = user?.role === 'admin';
    const owner = asAdmin || isAdminUser ? null : user?.username || null;
    return { port: DAEMON_PORT, owner };
  }
  return { port: user.port, owner: null };
}

function callIpc(user, channel, payload, timeoutMs = 0, asAdmin = false) {
  const body = Buffer.from(JSON.stringify({ channel, payload: payload ?? {} }));
  const target = targetOf(user, asAdmin);
  const headers = {
    'Content-Type': 'application/json',
    'Content-Length': body.length,
    Origin: `http://127.0.0.1:${target.port}`,
  };
  if (target.owner) headers['x-beav-user'] = target.owner;
  return new Promise((resolve, reject) => {
    const request = http.request(
      {
        hostname: '127.0.0.1',
        port: target.port,
        method: 'POST',
        path: '/api/ipc/invoke',
        headers,
      },
      (response) => {
        let text = '';
        response.on('data', (chunk) => (text += chunk));
        response.on('end', () => {
          try {
            resolve(JSON.parse(text));
          } catch (error) {
            reject(new Error(`invalid JSON: ${text.slice(0, 120)}`));
          }
        });
      },
    );
    request.on('error', reject);
    if (timeoutMs > 0) {
      request.setTimeout(timeoutMs, () => request.destroy(new Error('timeout')));
    }
    request.end(body);
  });
}

const PLATFORM_HOME = {
  xiaohongshu: 'https://www.xiaohongshu.com',
  douyin: 'https://www.douyin.com',
  bilibili: 'https://www.bilibili.com',
};

const STUCK_MS = 30 * 60_000;

/** 视频下载异常：failed / 卡在 downloading 超过 30 分钟；另统计从未下载的。 */
function computeVideoIssues(posts) {
  const now = Date.now();
  const issues = [];
  let pending = 0;
  for (const post of posts) {
    if (post.mediaType !== 'video') continue;
    const status = String(post.videoStatus || '');
    const updatedAt = Number(post.updatedAt || 0);
    if (status === 'failed') {
      issues.push({
        id: post.id,
        title: post.title || '',
        platform: post.platform,
        status: 'failed',
        error: post.videoError || '',
        updatedAt,
      });
    } else if (status === 'downloading' && now - updatedAt > STUCK_MS) {
      issues.push({
        id: post.id,
        title: post.title || '',
        platform: post.platform,
        status: 'stuck',
        error: post.videoError || '',
        updatedAt,
      });
    } else if (status === 'none' || !status) {
      pending += 1;
    }
  }
  issues.sort((a, b) => b.updatedAt - a.updatedAt);
  return { issues, pending };
}

/** 夜间任务：拉起实例 → 确保各平台 Chrome → 跑每日预取。 */
async function runDailyFor(user) {
  if (SINGLE_DAEMON) {
    try {
      const result = await callIpc(user, 'monitor:run-daily', {}, 0, true);
      log(`daily (shared) done: ${JSON.stringify(result).slice(0, 180)}`);
    } catch (error) {
      log(`daily (shared) failed: ${error.message}`);
    }
    return;
  }
  await ensureInstance(user);
  const record = instances.get(user.uid || user.username);
  if (record) {
    record.lastActive = Date.now();
    record.keepAliveUntil = Date.now() + 4 * 3_600_000;
  }

  let watches = [];
  try {
    const list = await callIpc(user, 'monitor:watch-list', {}, 30_000);
    watches = Array.isArray(list) ? list : Array.isArray(list?.items) ? list.items : [];
  } catch (error) {
    log(`daily ${user.username}: watch-list failed: ${error.message}`);
  }

  const platforms = [...new Set(watches.map((item) => item?.platform).filter(Boolean))];
  for (const platform of platforms) {
    try {
      await callIpc(
        user,
        'monitor:chrome-launch',
        { platform, url: PLATFORM_HOME[platform] || PLATFORM_HOME.xiaohongshu },
        60_000,
      );
      log(`daily ${user.username}: chrome ${platform} ready`);
    } catch (error) {
      log(`daily ${user.username}: chrome ${platform} failed: ${error.message}`);
    }
  }
  if (platforms.length) await new Promise((resolve) => setTimeout(resolve, 4_000));

  try {
    const result = await callIpc(user, 'monitor:run-daily', {});
    log(`daily ${user.username} done: ${JSON.stringify(result).slice(0, 180)}`);
  } catch (error) {
    log(`daily ${user.username} failed: ${error.message}`);
  }

  // 跑完汇总视频异常，便于管理员第二天直接重试。
  try {
    const posts = await callIpc(user, 'monitor:post-list', { limit: 300 }, 60_000);
    const { issues, pending } = computeVideoIssues(Array.isArray(posts) ? posts : []);
    const failed = issues.filter((item) => item.status === 'failed').length;
    const stuck = issues.length - failed;
    log(
      `daily ${user.username} video issues: ${failed} failed, ${stuck} stuck, ${pending} pending`,
    );
  } catch (error) {
    log(`daily ${user.username}: issue scan failed: ${error.message}`);
  }
}

async function ensureInstance(user) {
  if (SINGLE_DAEMON) {
    if (await healthOk(DAEMON_PORT)) {
      return { child: null, port: DAEMON_PORT, lastActive: Date.now(), startedAt: null };
    }
    throw new Error(`桌面端未运行或 daemon 未监听 ${DAEMON_PORT}`);
  }
  const key = user.uid || user.username;

  const existing = instances.get(key);
  if (existing) {
    existing.lastActive = Date.now();
    if (await healthOk(user.port)) return existing;
    try {
      existing.child?.kill();
    } catch {}
    instances.delete(key);
  }

  if (await healthOk(user.port)) {
    const record = { child: null, port: user.port, lastActive: Date.now(), startedAt: null };
    instances.set(key, record);
    return record;
  }

  const pending = spawning.get(key);
  if (pending) return pending;

  const task = (async () => {
    // 拿到"启动权"后再确认一次，避免上一个刚起好又被重复 spawn。
    if (await healthOk(user.port)) {
      const record = { child: null, port: user.port, lastActive: Date.now(), startedAt: null };
      instances.set(key, record);
      return record;
    }

    const exe = resolveExe();
    if (!exe || !fs.existsSync(exe)) {
      throw new Error(
        '找不到实例可执行文件（已尝试 target/release、target/debug、.app 包）。' +
          '请先在服务器上构建（pnpm tauri:build:beav），或用 BEAV_EXE 指向可执行文件。',
      );
    }
    const child = spawn(exe, [], {
      env: instanceEnv(user),
      stdio: 'ignore',
      windowsHide: true,
    });
    log(`spawn ${user.username} -> :${user.port} (pid ${child.pid})`);

    const deadline = Date.now() + 45_000;
    while (Date.now() < deadline) {
      if (await healthOk(user.port)) {
        const record = {
          child,
          port: user.port,
          lastActive: Date.now(),
          startedAt: Date.now(),
        };
        instances.set(key, record);
        log(`instance ready ${user.username} :${user.port}`);
        return record;
      }
      if (child.exitCode !== null) break;
      await new Promise((resolve) => setTimeout(resolve, 700));
    }
    try {
      child.kill();
    } catch {}
    throw new Error(`实例启动超时（${user.username} :${user.port}）`);
  })().finally(() => spawning.delete(key));

  spawning.set(key, task);
  return task;
}

function hhmm(date) {
  return `${String(date.getHours()).padStart(2, '0')}:${String(date.getMinutes()).padStart(2, '0')}`;
}

/** 下一个 KEEP_UNTIL 时间戳（今天的还没到就今天，否则明天）。 */
function nextKeepUntil(now) {
  const [hours, minutes] = KEEP_UNTIL.split(':').map((value) => Number(value) || 0);
  const target = new Date(now);
  target.setHours(hours, minutes, 0, 0);
  if (target.getTime() <= now.getTime()) target.setDate(target.getDate() + 1);
  return target.getTime();
}

let lastPrewarmDay = '';

setInterval(() => {
  if (SINGLE_DAEMON) return;
  const now = Date.now();

  // 夜间预热：到点把全部用户实例拉起，并保活到 KEEP_UNTIL。
  if (PREWARM_AT) {
    const current = new Date(now);
    if (hhmm(current) === PREWARM_AT) {
      const dayKey = current.toDateString();
      if (lastPrewarmDay !== dayKey) {
        lastPrewarmDay = dayKey;
        const keepUntil = nextKeepUntil(current);
        for (const user of loadUsers().users) {
          ensureInstance(user)
            .then((record) => {
              record.keepAliveUntil = keepUntil;
              log(`prewarm ${user.username} (keep until ${KEEP_UNTIL})`);
            })
            .catch((error) => log(`prewarm ${user.username} failed: ${error.message}`));
        }
      }
    }
  }

  // 闲置回收：IDLE_MINUTES<=0 表示不回收；保活中的实例跳过。
  if (IDLE_MINUTES <= 0) return;
  for (const [key, record] of instances) {
    if (record.keepAliveUntil && now < record.keepAliveUntil) continue;
    if (now - record.lastActive > IDLE_MINUTES * 60_000) {
      try {
        record.child?.kill();
      } catch {}
      instances.delete(key);
      log(`reap idle instance ${key}`);
    }
  }
}, 60_000).unref?.();

// 集中触发每日任务（不依赖各实例内部定时器）。
let lastDailyDay = '';
setInterval(() => {
  if (SINGLE_DAEMON) return;
  if (!DAILY_AT) return;
  const now = new Date();
  if (hhmm(now) !== DAILY_AT) return;
  const dayKey = now.toDateString();
  if (lastDailyDay === dayKey) return;
  lastDailyDay = dayKey;
  (async () => {
    const users = loadUsers().users;
    log(`daily start (${users.length} users)`);
    for (const user of users) {
      try {
        await runDailyFor(user);
      } catch (error) {
        log(`daily ${user.username} failed: ${error.message}`);
      }
    }
    log('daily finished');
  })();
}, 60_000).unref?.();

// ---------------- HTTP ----------------

function sendJson(res, status, body) {
  const payload = JSON.stringify(body);
  res.writeHead(status, {
    'Content-Type': 'application/json; charset=utf-8',
    'Content-Length': Buffer.byteLength(payload),
    'Cache-Control': 'no-store',
  });
  res.end(payload);
}

function sendHtml(res, status, html) {
  res.writeHead(status, {
    'Content-Type': 'text/html; charset=utf-8',
    'Content-Length': Buffer.byteLength(html),
    'Cache-Control': 'no-store',
  });
  res.end(html);
}

const LOGIN_HTML = `<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8" />
<meta name="viewport" content="width=device-width,initial-scale=1" />
<title>Beav 登录</title>
<style>
  :root{color-scheme:dark}
  body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;
    background:#1b1b1f;color:#e8e8ea;font:14px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif}
  form{width:320px;padding:28px;border:1px solid #33333a;border-radius:14px;background:#232329}
  h1{margin:0 0 4px;font-size:18px}
  p{margin:0 0 18px;color:#8a8a93;font-size:12px}
  input{width:100%;box-sizing:border-box;margin-bottom:12px;padding:10px 12px;border-radius:8px;
    border:1px solid #3a3a42;background:#1b1b1f;color:#e8e8ea;font-size:14px}
  button{width:100%;padding:10px;border:0;border-radius:8px;background:#c9a227;color:#1b1b1f;
    font-weight:700;font-size:14px;cursor:pointer}
  .err{color:#f87171;font-size:12px;margin-bottom:12px}
  .alt{margin-top:14px;font-size:12px;color:#8a8a93;text-align:center}
  a{color:#c9a227}
</style></head>
<body><form method="post" action="/login">
  <h1>Beav</h1><p>多用户工作区，请输入账号</p>
  __ERROR__
  <input name="username" placeholder="用户名" autocomplete="username" autofocus />
  <input name="password" type="password" placeholder="密码" autocomplete="current-password" />
  <button type="submit">登录</button>
  __REGISTER__
</form></body></html>`;

function loginPage(error) {
  return LOGIN_HTML
    .replace('__ERROR__', error ? `<div class="err">${escapeHtml(error)}</div>` : '')
    .replace('__REGISTER__', process.env.BEAV_ALLOW_REGISTER === '0'
      ? ''
      : '<div class="alt">还没有账号？<a href="/register">注册</a></div>');
}

function escapeHtml(value) {
  return String(value ?? '').replace(/[&<>"']/g, (ch) => ({
    '&': '&amp;',
    '<': '&lt;',
    '>': '&gt;',
    '"': '&quot;',
    "'": '&#39;',
  }[ch]));
}

const REGISTER_HTML = `<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8" />
<meta name="viewport" content="width=device-width,initial-scale=1" />
<title>Beav 注册</title>
<style>
  :root{color-scheme:dark}
  body{margin:0;min-height:100vh;display:flex;align-items:center;justify-content:center;
    background:#1b1b1f;color:#e8e8ea;font:14px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif}
  form{width:320px;padding:28px;border:1px solid #33333a;border-radius:14px;background:#232329}
  h1{margin:0 0 4px;font-size:18px}
  p{margin:0 0 18px;color:#8a8a93;font-size:12px}
  input{width:100%;box-sizing:border-box;margin-bottom:12px;padding:10px 12px;border-radius:8px;
    border:1px solid #3a3a42;background:#1b1b1f;color:#e8e8ea;font-size:14px}
  button{width:100%;padding:10px;border:0;border-radius:8px;background:#c9a227;color:#1b1b1f;
    font-weight:700;font-size:14px;cursor:pointer}
  .err{color:#f87171;font-size:12px;margin-bottom:12px}
  .alt{margin-top:14px;font-size:12px;color:#8a8a93;text-align:center}
  a{color:#c9a227}
</style></head>
<body><form method="post" action="/register">
  <h1>注册 Beav</h1><p>每个账号拥有独立的数据、工作区与导出目录</p>
  __ERROR__
  <input name="username" placeholder="用户名（2-24 位字母/数字/_-）" autocomplete="username" autofocus />
  <input name="password" type="password" placeholder="密码（至少 6 位）" autocomplete="new-password" />
  <input name="confirm" type="password" placeholder="确认密码" autocomplete="new-password" />
  <button type="submit">注册并登录</button>
  <div class="alt">已有账号？<a href="/login">登录</a></div>
</form></body></html>`;

function registerPage(error) {
  return REGISTER_HTML.replace(
    '__ERROR__',
    error ? `<div class="err">${escapeHtml(error)}</div>` : '',
  );
}

const ADMIN_HTML = `<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8" />
<meta name="viewport" content="width=device-width,initial-scale=1" />
<title>Beav 实例状态</title>
<style>
  :root{color-scheme:dark}
  body{margin:0;padding:28px;background:#1b1b1f;color:#e8e8ea;
    font:14px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif}
  h1{margin:0 0 4px;font-size:20px}
  .sub{color:#8a8a93;font-size:12px;margin-bottom:20px}
  .summary{display:flex;gap:18px;margin-bottom:14px;font-size:12px;color:#a0a0aa}
  .summary b{color:#e8e8ea}
  table{width:100%;border-collapse:collapse;font-size:13px}
  th,td{padding:8px 10px;text-align:left;border-bottom:1px solid #2c2c33;white-space:nowrap}
  th{color:#8a8a93;font-weight:600;font-size:12px}
  .run{color:#4ade80}.stop{color:#8a8a93}
  button{padding:4px 10px;border:1px solid #3a3a42;border-radius:6px;background:#232329;
    color:#e8e8ea;font-size:12px;cursor:pointer}
  button:hover{border-color:#c9a227;color:#c9a227}
  .right{margin-left:auto}
  a{color:#c9a227;text-decoration:none}
</style></head><body>
<h1>实例状态</h1>
<div class="sub">每 5 秒自动刷新 · 数据目录 <code>__ROOT__</code></div>
<div class="summary">
  <span>用户 <b id="total">-</b></span>
  <span>运行中 <b id="running">-</b></span>
  <span>总内存 <b id="mem">-</b></span>
  <span class="right"><a href="/admin/qr">扫码登录小红书</a> · <a href="/account">我的账号</a> · <a href="/logout">退出</a></span>
</div>
<table><thead><tr>
  <th>用户</th><th>端口</th><th>状态</th><th>PID</th><th>内存</th><th>启动时间</th><th>最近活跃</th><th></th>
</tr></thead><tbody id="rows"><tr><td colspan="8">加载中…</td></tr></tbody></table>
<script>
function fmtTime(ts){ return ts ? new Date(ts).toLocaleTimeString() : '-'; }
async function stopUser(username){
  if(!confirm('停止 ' + username + ' 的实例？下次访问会自动重启')) return;
  await fetch('/admin/api/stop',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({username})});
  refresh();
}
async function refresh(){
  let list;
  try { const res = await fetch('/admin/api/instances'); if(!res.ok) throw new Error('HTTP '+res.status); list = await res.json(); }
  catch(e){ document.getElementById('rows').innerHTML = '<tr><td colspan="8">读取失败：'+e.message+'</td></tr>'; return; }
  const tbody = document.getElementById('rows'); tbody.innerHTML='';
  let mem=0, running=0;
  for(const it of list){
    if(it.running) running++;
    if(it.rssMB) mem += it.rssMB;
    const tr=document.createElement('tr');
    const cells=[it.username + (it.role==='admin'?' (admin)':''), it.port,
      it.running?'运行中':'未启动', it.pid ?? '-', it.rssMB!=null?it.rssMB+' MB':'-',
      fmtTime(it.startedAt), fmtTime(it.lastActive)];
    for(const c of cells){ const td=document.createElement('td'); td.textContent=String(c); if(c==='运行中')td.className='run'; if(c==='未启动')td.className='stop'; tr.appendChild(td); }
    const td=document.createElement('td');
    const link=document.createElement('a');
    link.href='/admin/user/'+encodeURIComponent(it.username);
    link.textContent='数据';
    link.style.marginRight='10px';
    td.appendChild(link);
    if(it.running){ const b=document.createElement('button'); b.textContent='停止'; b.onclick=()=>stopUser(it.username); td.appendChild(b); }
    tr.appendChild(td); tbody.appendChild(tr);
  }
  document.getElementById('total').textContent=String(list.length);
  document.getElementById('running').textContent=String(running);
  document.getElementById('mem').textContent=mem?mem+' MB':'-';
}
refresh(); setInterval(refresh, 5000);
</script></body></html>`;

const QR_PAGE_HTML = `<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8" />
<meta name="viewport" content="width=device-width,initial-scale=1" />
<title>扫码登录 · 小红书</title>
<style>
  :root{color-scheme:dark}
  body{margin:0;padding:32px;background:#1b1b1f;color:#e8e8ea;
    font:14px/1.6 system-ui,-apple-system,"Segoe UI",sans-serif}
  h1{font-size:20px;margin:0 0 6px}
  .sub{color:#8a8a93;font-size:12px;margin-bottom:18px}
  button{padding:8px 14px;border:1px solid #3a3a42;border-radius:8px;background:#232329;
    color:#e8e8ea;font-size:13px;cursor:pointer}
  button:hover{border-color:#c9a227;color:#c9a227}
  #msg{color:#a0a0aa;font-size:12px;margin-left:10px}
  #wrap{margin-top:20px;display:flex;justify-content:center}
  #qr{max-width:320px;border-radius:12px;background:#fff;padding:10px;display:none}
  a{color:#c9a227;text-decoration:none}
</style></head><body>
<h1>扫码登录 · 小红书</h1>
<div class="sub">用**小红书 App** 扫描下方二维码；登录成功后自动保存 cookie（约 2 分钟过期）· <a href="/admin">返回</a></div>
<button id="start">获取二维码 / 刷新</button><span id="msg"></span>
<div id="wrap"><img id="qr" alt="小红书登录二维码" /></div>
<script>
let timer = null;
async function refreshQr(){
  document.getElementById('msg').textContent = '获取中…（首次启动 Chrome 约 10-30s）';
  try{
    const res = await fetch('/admin/api/qr-start',{method:'POST',headers:{'Content-Type':'application/json'},
      body:JSON.stringify({platform:'xiaohongshu'})});
    const data = await res.json();
    if(!data.success){ document.getElementById('msg').textContent = '失败：' + (data.error||''); return; }
    const img = document.getElementById('qr');
    img.src = data.image;
    img.style.display = 'block';
    document.getElementById('msg').textContent = '请用小红书 App 扫码';
    if(!timer) timer = setInterval(poll, 5000);
  }catch(e){ document.getElementById('msg').textContent = '失败：' + e.message; }
}
async function poll(){
  try{
    const res = await fetch('/admin/api/qr-status?platform=xiaohongshu');
    const data = await res.json();
    if(data.loggedIn){
      document.getElementById('msg').textContent = '已登录 ✅ 可以关闭本页';
      if(timer){ clearInterval(timer); timer = null; }
    }
  }catch(e){}
}
document.getElementById('start').onclick = refreshQr;
refreshQr();
</script></body></html>`;

const USER_PAGE_HTML = `<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8" />
<meta name="viewport" content="width=device-width,initial-scale=1" />
<title>用户数据 · Beav</title>
<style>
  :root{color-scheme:dark}
  body{margin:0;padding:28px;background:#1b1b1f;color:#e8e8ea;
    font:14px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif}
  h1{font-size:20px;margin:0 0 6px}h2{font-size:14px;margin:22px 0 8px;color:#a0a0aa}
  .sub{color:#8a8a93;font-size:12px;margin-bottom:14px}
  .bar{display:flex;gap:10px;align-items:center;margin-bottom:8px}
  table{width:100%;border-collapse:collapse;font-size:13px}
  th,td{padding:7px 10px;text-align:left;border-bottom:1px solid #2c2c33;white-space:nowrap}
  th{color:#8a8a93;font-weight:600;font-size:12px}
  button{padding:6px 12px;border:1px solid #3a3a42;border-radius:7px;background:#232329;
    color:#e8e8ea;font-size:12px;cursor:pointer}
  button:hover{border-color:#c9a227;color:#c9a227}
  a{color:#c9a227;text-decoration:none}
  .muted{color:#8a8a93}
</style></head><body>
<h1 id="title">用户数据</h1>
<div class="sub">只读视图 · 数据来自该用户自己的实例 · <a href="/admin">返回实例列表</a></div>
<div class="bar">
  <button id="run">立即跑每日任务</button>
  <span id="msg" class="muted"></span>
</div>
<h2>监控博主</h2>
<table><thead><tr><th>名称</th><th>平台</th><th>已收录</th><th>上次抓取</th><th>已补历史</th></tr></thead>
<tbody id="watches"><tr><td colspan="5" class="muted">加载中…</td></tr></tbody></table>
<h2>视频下载异常 <span id="issueCount" class="muted"></span></h2>
<div class="bar">
  <button id="retryAll">全部重试</button>
  <span id="issueMsg" class="muted"></span>
</div>
<table><thead><tr><th>标题</th><th>平台</th><th>状态</th><th>错误</th><th>更新时间</th><th></th></tr></thead>
<tbody id="issues"><tr><td colspan="6" class="muted">加载中…</td></tr></tbody></table>
<p id="pending" class="muted"></p>

<h2>最近动态</h2>
<table><thead><tr><th>标题</th><th>平台</th><th>视频</th><th>更新时间</th></tr></thead>
<tbody id="posts"><tr><td colspan="4" class="muted">加载中…</td></tr></tbody></table>
<script>
const UID = __UID__;
const NAMES = __NAME__;
let current = null;
function td(text){const el=document.createElement('td');el.textContent=String(text??'');return el;}
function row(){return document.createElement('tr');}
async function retryOne(item){
  document.getElementById('issueMsg').textContent='重试中…';
  try{
    await fetch('/admin/api/retry-video',{method:'POST',headers:{'Content-Type':'application/json'},
      body:JSON.stringify({username:UID,postId:item.id,platform:item.platform})});
    document.getElementById('issueMsg').textContent='已触发重试';
  }catch(e){document.getElementById('issueMsg').textContent='重试失败：'+e.message;}
}
async function refresh(){
  let data;
  try{
    const res=await fetch('/admin/api/user/'+encodeURIComponent(UID));
    if(!res.ok) throw new Error('HTTP '+res.status);
    data=await res.json();
  }catch(e){
    document.getElementById('msg').textContent='读取失败：'+e.message;return;
  }
  current = data;
  document.title=NAMES+' · 用户数据';
  document.getElementById('title').textContent=NAMES+' 的数据';
  const wt=document.getElementById('watches');wt.innerHTML='';
  for(const w of data.watches){
    const tr=row();
    tr.append(td(w.name),td(w.platform),td(w.postCount??'-'),
      td(w.lastFetchAt?new Date(Number(w.lastFetchAt)).toLocaleString():'-'),
      td(w.historyBackfilledAt?'是':'否'));
    wt.appendChild(tr);
  }
  if(!data.watches.length) wt.innerHTML='<tr><td colspan="5" class="muted">还没有监控博主</td></tr>';
  const it=document.getElementById('issues');it.innerHTML='';
  document.getElementById('issueCount').textContent = data.issues.length ? '('+data.issues.length+')' : '';
  for(const x of data.issues){
    const tr=row();
    const statusText = x.status==='failed' ? '失败' : x.status==='stuck' ? '卡住' : x.status;
    tr.append(td(x.title||'(无标题)'),td(x.platform),td(statusText),td(x.error||'-'),
      td(x.updatedAt?new Date(x.updatedAt).toLocaleString():'-'));
    const action=document.createElement('td');
    const btn=document.createElement('button');btn.textContent='重试';btn.onclick=()=>retryOne(x);
    action.appendChild(btn);tr.appendChild(action);it.appendChild(tr);
  }
  if(!data.issues.length) it.innerHTML='<tr><td colspan="6" class="muted">没有异常</td></tr>';
  document.getElementById('pending').textContent = data.pendingVideos
    ? ('另有 '+data.pendingVideos+' 个视频从未下载（不在异常列表）') : '';
  const pt=document.getElementById('posts');pt.innerHTML='';
  for(const p of data.posts){
    const tr=row();
    tr.append(td(p.title||'(无标题)'),td(p.platform),td(p.videoStatus||'-'),
      td(p.updatedAt?new Date(Number(p.updatedAt)).toLocaleString():'-'));
    pt.appendChild(tr);
  }
  if(!data.posts.length) pt.innerHTML='<tr><td colspan="4" class="muted">没有动态</td></tr>';
}
document.getElementById('retryAll').onclick=async()=>{
  if(!current || !current.issues.length) return;
  const ids = current.issues.map((x)=>x.id);
  document.getElementById('issueMsg').textContent='批量重试中…';
  try{
    await fetch('/admin/api/retry-videos',{method:'POST',headers:{'Content-Type':'application/json'},
      body:JSON.stringify({username:UID,ids})});
    document.getElementById('issueMsg').textContent='已触发 '+ids.length+' 个';
  }catch(e){document.getElementById('issueMsg').textContent='批量重试失败：'+e.message;}
};
document.getElementById('run').onclick=async()=>{
  document.getElementById('msg').textContent='已触发，后台执行中…';
  try{
    await fetch('/admin/api/run-daily',{method:'POST',headers:{'Content-Type':'application/json'},
      body:JSON.stringify({username:UID})});
    document.getElementById('msg').textContent='已触发（看管理器日志）';
  }catch(e){document.getElementById('msg').textContent='触发失败：'+e.message;}
};
refresh();
</script></body></html>`;

function userPage(uid, username) {
  return USER_PAGE_HTML
    .replace('__UID__', JSON.stringify(uid))
    .replace('__NAME__', JSON.stringify(username || uid));
}

function accountPage(user, status) {
  const rows = [
    ['用户名', user.username],
    ['角色', user.role || 'user'],
    ['实例端口', status.port],
    ['状态', status.running ? '运行中' : '未启动'],
    ['PID', status.pid ?? '-'],
    ['内存', status.rssMB != null ? `${status.rssMB} MB` : '-'],
    ['数据目录', path.join(USERS_ROOT, user.uid || user.username)],
  ];
  const trs = rows
    .map(([key, value]) => `<tr><th>${escapeHtml(key)}</th><td>${escapeHtml(value)}</td></tr>`)
    .join('');
  return `<!doctype html><html lang="zh-CN"><head><meta charset="utf-8" />
<title>我的账号 · Beav</title><style>
:root{color-scheme:dark}body{margin:0;padding:28px;background:#1b1b1f;color:#e8e8ea;
font:14px/1.5 system-ui,-apple-system,"Segoe UI",sans-serif}
h1{font-size:20px;margin:0 0 16px}table{border-collapse:collapse;min-width:420px}
th,td{padding:8px 12px;border-bottom:1px solid #2c2c33;text-align:left;font-size:13px}
th{color:#8a8a93;font-weight:600;width:120px}a{color:#c9a227;text-decoration:none}
.bar{margin-top:18px;font-size:12px}
</style></head><body>
<h1>我的账号</h1>
<table>${trs}</table>
<div class="bar"><a href="/">进入工作台</a>${user.role === 'admin' ? ' · <a href="/admin">实例状态</a>' : ''} · <a href="/logout">退出</a></div>
</body></html>`;
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on('data', (chunk) => chunks.push(chunk));
    req.on('end', () => resolve(Buffer.concat(chunks)));
    req.on('error', reject);
  });
}

function proxyToInstance(req, res, user, url, body) {
  const target = targetOf(user);
  const targetOrigin = `http://127.0.0.1:${target.port}`;
  const headers = { ...req.headers };
  headers.origin = targetOrigin;
  headers.referer = `${targetOrigin}/`;
  headers.host = `127.0.0.1:${target.port}`;
  if (target.owner) headers['x-beav-user'] = target.owner;
  else delete headers['x-beav-user'];
  delete headers['transfer-encoding'];
  if (body.length > 0) headers['content-length'] = String(body.length);
  else delete headers['content-length'];

  const upstream = http.request(
    {
      hostname: '127.0.0.1',
      port: target.port,
      method: req.method,
      path: `${url.pathname}${url.search}`,
      headers,
    },
    (upstreamRes) => {
      const outHeaders = { ...upstreamRes.headers };
      delete outHeaders['access-control-allow-origin'];
      res.writeHead(upstreamRes.statusCode || 502, outHeaders);
      upstreamRes.pipe(res);
    },
  );
  upstream.on('error', (error) => {
    log(`proxy error ${user.username}:`, error.message);
    if (!res.headersSent) {
      sendJson(res, 502, { success: false, error: `实例不可用：${error.message}` });
    } else {
      res.destroy();
    }
  });
  upstream.end(body);
}

function serveFile(req, res, filePath, allowSpaFallback) {
  let stat;
  try {
    stat = fs.statSync(filePath);
  } catch {
    if (allowSpaFallback) return serveFile(req, res, path.join(DIST_DIR, 'index.html'), false);
    return sendJson(res, 404, { success: false, error: 'not found' });
  }
  if (stat.isDirectory()) {
    return serveFile(req, res, path.join(filePath, 'index.html'), allowSpaFallback);
  }
  const headers = {
    'Content-Type': MIME[path.extname(filePath).toLowerCase()] || 'application/octet-stream',
    'Content-Length': stat.size,
    'Cache-Control': filePath.includes(`${path.sep}assets${path.sep}`)
      ? 'public, max-age=31536000, immutable'
      : 'no-cache',
  };
  res.writeHead(200, headers);
  if (req.method === 'HEAD') return res.end();
  fs.createReadStream(filePath).pipe(res);
}

function serveApp(req, res, url) {
  const relative = decodeURIComponent(url.pathname).replace(/^\/+/, '');
  const target = path.resolve(DIST_DIR, relative || 'index.html');
  if (!target.startsWith(DIST_DIR)) return sendJson(res, 403, { success: false, error: 'forbidden' });
  return serveFile(req, res, target, path.extname(target) === '');
}

// ---------------- 启动 ----------------

function findUser(username) {
  const target = String(username || '').trim().toLowerCase();
  return loadUsers().users.find((user) => String(user.username).toLowerCase() === target);
}

async function main() {
  const command = process.argv[2];
  if (command === '--list') {
    const { users } = loadUsers();
    if (!users.length) log('（空）');
    for (const user of users) log(`${user.username}  uid=${user.uid}  port=${user.port}`);
    return;
  }
  if (command === '--create' || command === '--create-admin') {
    const username = process.argv[3];
    const password = process.argv[4];
    if (!username || !password) {
      throw new Error(`用法：${command} <用户名> <密码> [端口]`);
    }
    const data = loadUsers();
    if (data.users.some((user) => user.username === username)) {
      throw new Error(`用户已存在：${username}`);
    }
    const port = Number(process.argv[5] || nextPort(data.users));
    data.users.push({
      username,
      uid: username,
      role: command === '--create-admin' ? 'admin' : 'user',
      port,
      passwordHash: hashPassword(password),
      createdAt: new Date().toISOString(),
    });
    saveUsers(data);
    log(`已创建 ${username}（role=${command === '--create-admin' ? 'admin' : 'user'}, port=${port}）`);
    return;
  }
  if (command === '--set-password') {
    const username = process.argv[3];
    const password = process.argv[4];
    if (!username || !password) throw new Error('用法：--set-password <用户名> <新密码>');
    const data = loadUsers();
    const user = data.users.find((item) => item.username === username);
    if (!user) throw new Error(`用户不存在：${username}`);
    user.passwordHash = hashPassword(password);
    saveUsers(data);
    log(`已更新 ${username} 的密码`);
    return;
  }
  if (command === '--delete') {
    const username = process.argv[3];
    const data = loadUsers();
    const before = data.users.length;
    data.users = data.users.filter((item) => item.username !== username);
    saveUsers(data);
    log(`已删除 ${before - data.users.length} 个用户`);
    return;
  }

  ensureUsersFile();
  if (!fs.existsSync(path.join(DIST_DIR, 'index.html'))) {
    log(`警告：${DIST_DIR}/index.html 不存在，请先 pnpm web:build`);
  }

  const server = http.createServer(async (req, res) => {
    try {
      const url = new URL(req.url || '/', `http://${req.headers.host || 'localhost'}`);

      if (url.pathname === '/login') {
        if (req.method === 'GET') return sendHtml(res, 200, loginPage(''));
        if (req.method === 'POST') {
          const body = (await readBody(req)).toString('utf8');
          const params = new URLSearchParams(body);
          const username = String(params.get('username') || '').trim();
          const password = String(params.get('password') || '');
          const user = findUser(username);
          if (!user || !verifyPassword(password, user.passwordHash)) {
            return sendHtml(res, 401, loginPage('用户名或密码不正确'));
          }
          const token = signSession(user.username);
          res.writeHead(302, {
            'Set-Cookie': `beav_session=${encodeURIComponent(token)}; HttpOnly; SameSite=Lax; Path=/; Max-Age=${SESSION_HOURS * 3600}`,
            Location: '/',
          });
          return res.end();
        }
        return sendJson(res, 405, { success: false, error: 'method not allowed' });
      }

      if (url.pathname === '/logout') {
        res.writeHead(302, {
          'Set-Cookie': 'beav_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0',
          Location: '/login',
        });
        return res.end();
      }

      if (url.pathname === '/register') {
        if (process.env.BEAV_ALLOW_REGISTER === '0') {
          return sendHtml(res, 403, registerPage('注册已关闭，请联系管理员'));
        }
        if (req.method === 'GET') return sendHtml(res, 200, registerPage(''));
        if (req.method === 'POST') {
          const body = (await readBody(req)).toString('utf8');
          const params = new URLSearchParams(body);
          const username = String(params.get('username') || '').trim();
          const password = String(params.get('password') || '');
          const confirm = String(params.get('confirm') || '');
          if (!/^[A-Za-z0-9_-]{2,24}$/.test(username)) {
            return sendHtml(res, 400, registerPage('用户名需为 2-24 位字母/数字/_/-'));
          }
          if (password.length < 6) {
            return sendHtml(res, 400, registerPage('密码至少 6 位'));
          }
          if (password !== confirm) {
            return sendHtml(res, 400, registerPage('两次输入的密码不一致'));
          }
          const data = loadUsers();
          if (
            data.users.some(
              (user) => String(user.username).toLowerCase() === username.toLowerCase(),
            )
          ) {
            return sendHtml(res, 409, registerPage('用户名已被占用'));
          }
          const port = nextPort(data.users);
          data.users.push({
            username,
            uid: username,
            role: 'user',
            port,
            passwordHash: hashPassword(password),
            createdAt: new Date().toISOString(),
          });
          saveUsers(data);
          log(`registered ${username} :${port}`);
          const token = signSession(username);
          res.writeHead(302, {
            'Set-Cookie': `beav_session=${encodeURIComponent(token)}; HttpOnly; SameSite=Lax; Path=/; Max-Age=${SESSION_HOURS * 3600}`,
            Location: '/',
          });
          return res.end();
        }
        return sendJson(res, 405, { success: false, error: 'method not allowed' });
      }

      const cookies = parseCookies(req.headers.cookie);
      const sessionUser = verifySession(cookies.beav_session);
      const user = sessionUser ? findUser(sessionUser) : null;
      if (!user) {
        if (url.pathname.startsWith('/api/')) {
          return sendJson(res, 401, { success: false, error: '未登录' });
        }
        res.writeHead(302, { Location: '/login' });
        return res.end();
      }

      // 扫码页只显示平台二维码，不涉及用户数据 → 允许所有已登录用户使用；
      // 其余 /admin（用户数据、实例状态）仍仅管理员。
      const qrPath =
        process.env.BEAV_QR_FOR_ALL !== '0' &&
        (url.pathname === '/admin/qr' || url.pathname.startsWith('/admin/api/qr-'));

      if (url.pathname === '/admin' || url.pathname.startsWith('/admin/')) {
        if (!qrPath && (user.role || 'user') !== 'admin') {
          if (url.pathname.startsWith('/admin/api/')) {
            return sendJson(res, 403, { success: false, error: '需要管理员权限' });
          }
          return sendHtml(res, 403, '<h1>403 · 需要管理员权限</h1>');
        }
        if (url.pathname === '/admin') {
          return sendHtml(res, 200, ADMIN_HTML.replace('__ROOT__', escapeHtml(USERS_ROOT)));
        }
        if (url.pathname === '/admin/qr') {
          return sendHtml(res, 200, QR_PAGE_HTML);
        }
        if (url.pathname === '/admin/api/qr-start') {
          if (!SINGLE_DAEMON) {
            return sendJson(res, 400, {
              success: false,
              error: '扫码登录仅支持单实例模式（BEAV_SINGLE_DAEMON=1）',
            });
          }
          if (req.method !== 'POST') {
            return sendJson(res, 405, { success: false, error: 'method not allowed' });
          }
          const body = (await readBody(req)).toString('utf8');
          let parsed = {};
          try {
            parsed = JSON.parse(body || '{}');
          } catch {}
          try {
            const result = await callIpc(
              null,
              'monitor:chrome-qr-start',
              { platform: parsed.platform || 'xiaohongshu' },
              120_000,
              true,
            );
            return sendJson(res, 200, result);
          } catch (error) {
            return sendJson(res, 500, { success: false, error: error.message });
          }
        }
        if (url.pathname === '/admin/api/qr-status') {
          if (!SINGLE_DAEMON) {
            return sendJson(res, 400, { success: false, error: '仅单实例模式可用' });
          }
          const platform = url.searchParams.get('platform') || 'xiaohongshu';
          try {
            const result = await callIpc(
              null,
              'monitor:chrome-qr-status',
              { platform },
              20_000,
              true,
            );
            return sendJson(res, 200, result);
          } catch (error) {
            return sendJson(res, 500, { success: false, error: error.message });
          }
        }
        if (url.pathname.startsWith('/admin/user/')) {
          const uid = decodeURIComponent(url.pathname.slice('/admin/user/'.length));
          const target = findUser(uid);
          if (!target) return sendHtml(res, 404, '<h1>用户不存在</h1>');
          return sendHtml(res, 200, userPage(target.uid || target.username, target.username));
        }
        if (url.pathname.startsWith('/admin/api/user/')) {
          const uid = decodeURIComponent(url.pathname.slice('/admin/api/user/'.length));
          const target = findUser(uid);
          if (!target) return sendJson(res, 404, { success: false, error: '用户不存在' });
          await ensureInstance(target);
          const [watchesRaw, postsRaw] = await Promise.all([
            callIpc(target, 'monitor:watch-list', {}, 30_000).catch(() => []),
            callIpc(target, 'monitor:post-list', { limit: 200 }, 60_000).catch(() => []),
          ]);
          const watches = Array.isArray(watchesRaw) ? watchesRaw : [];
          const posts = Array.isArray(postsRaw) ? postsRaw : [];
          const { issues, pending } = computeVideoIssues(posts);
          return sendJson(res, 200, { watches, posts, issues, pendingVideos: pending });
        }
        if (url.pathname === '/admin/api/retry-video') {
          if (req.method !== 'POST') {
            return sendJson(res, 405, { success: false, error: 'method not allowed' });
          }
          const body = (await readBody(req)).toString('utf8');
          let parsed = {};
          try {
            parsed = JSON.parse(body || '{}');
          } catch {}
          const target = findUser(parsed.username);
          const postId = String(parsed.postId || '').trim();
          if (!target) return sendJson(res, 404, { success: false, error: '用户不存在' });
          if (!postId) return sendJson(res, 400, { success: false, error: '缺少 postId' });
          await ensureInstance(target);
          if (parsed.platform) {
            try {
              await callIpc(
                target,
                'monitor:chrome-launch',
                {
                  platform: parsed.platform,
                  url: PLATFORM_HOME[parsed.platform] || PLATFORM_HOME.xiaohongshu,
                },
                30_000,
              );
            } catch (error) {
              log(`retry chrome ${parsed.platform} failed: ${error.message}`);
            }
          }
          callIpc(target, 'monitor:download-video', { id: postId })
            .then((result) => log(`retry ${postId}: ${JSON.stringify(result).slice(0, 120)}`))
            .catch((error) => log(`retry ${postId} failed: ${error.message}`));
          return sendJson(res, 200, { success: true, started: true });
        }
        if (url.pathname === '/admin/api/retry-videos') {
          if (req.method !== 'POST') {
            return sendJson(res, 405, { success: false, error: 'method not allowed' });
          }
          const body = (await readBody(req)).toString('utf8');
          let parsed = {};
          try {
            parsed = JSON.parse(body || '{}');
          } catch {}
          const target = findUser(parsed.username);
          const ids = Array.isArray(parsed.ids) ? parsed.ids.filter(Boolean) : [];
          if (!target) return sendJson(res, 404, { success: false, error: '用户不存在' });
          if (!ids.length) return sendJson(res, 400, { success: false, error: '没有可重试的帖子' });
          (async () => {
            for (const id of ids) {
              try {
                await callIpc(target, 'monitor:download-video', { id });
              } catch (error) {
                log(`retry ${id} failed: ${error.message}`);
              }
            }
            log(`retry batch ${target.username} done (${ids.length})`);
          })();
          return sendJson(res, 200, { success: true, started: true, count: ids.length });
        }
        if (url.pathname === '/admin/api/run-daily') {
          if (req.method !== 'POST') {
            return sendJson(res, 405, { success: false, error: 'method not allowed' });
          }
          const body = (await readBody(req)).toString('utf8');
          let parsed = {};
          try {
            parsed = JSON.parse(body || '{}');
          } catch {}
          const target = findUser(parsed.username);
          if (!target) return sendJson(res, 404, { success: false, error: '用户不存在' });
          runDailyFor(target).catch((error) =>
            log(`manual daily ${target.username} failed: ${error.message}`),
          );
          return sendJson(res, 200, { success: true, started: true });
        }
        if (url.pathname === '/admin/api/instances') {
          if (SINGLE_DAEMON) {
            const running = await healthOk(DAEMON_PORT);
            const pid = running ? await findPidByPort(DAEMON_PORT) : null;
            const rssMB = pid ? await getMemoryMB(pid) : null;
            return sendJson(res, 200, [
              {
                username: '(桌面端共享实例)',
                uid: 'shared',
                role: 'shared',
                port: DAEMON_PORT,
                running,
                pid,
                rssMB,
                startedAt: null,
                lastActive: null,
                createdAt: null,
              },
            ]);
          }
          const list = [];
          for (const item of loadUsers().users) {
            list.push(await instanceStatus(item));
          }
          return sendJson(res, 200, list);
        }
        if (url.pathname === '/admin/api/stop') {
          if (SINGLE_DAEMON) {
            return sendJson(res, 400, {
              success: false,
              error: '单实例模式：请直接在桌面端退出',
            });
          }
          if (req.method !== 'POST') {
            return sendJson(res, 405, { success: false, error: 'method not allowed' });
          }
          const body = (await readBody(req)).toString('utf8');
          let parsed = {};
          try {
            parsed = JSON.parse(body || '{}');
          } catch {}
          const target = findUser(parsed.username);
          if (!target) return sendJson(res, 404, { success: false, error: '用户不存在' });
          const key = target.uid || target.username;
          const record = instances.get(key);
          let pid = record?.child?.pid ?? null;
          if (!pid) pid = await findPidByPort(target.port);
          if (pid) {
            try {
              process.kill(pid);
            } catch {}
          }
          instances.delete(key);
          log(`stopped instance ${target.username} (pid ${pid ?? '-'})`);
          return sendJson(res, 200, { success: true });
        }
        return sendJson(res, 404, { success: false, error: 'not found' });
      }

      if (url.pathname === '/account') {
        const status = await instanceStatus(user);
        return sendHtml(res, 200, accountPage(user, status));
      }

      if (url.pathname.startsWith('/api/')) {
        await ensureInstance(user);
        const record = instances.get(user.uid || user.username);
        if (record) record.lastActive = Date.now();
        const body = await readBody(req);
        return proxyToInstance(req, res, user, url, body);
      }

      if (req.method !== 'GET' && req.method !== 'HEAD') {
        return sendJson(res, 405, { success: false, error: 'method not allowed' });
      }
      return serveApp(req, res, url);
    } catch (error) {
      log('request failed:', error.message);
      if (!res.headersSent) sendJson(res, 500, { success: false, error: error.message });
    }
  });

  server.listen(WEB_PORT, WEB_HOST, () => {
    log(`listening on http://${WEB_HOST}:${WEB_PORT}`);
    log(`mode    -> ${SINGLE_DAEMON ? `single-daemon(:${DAEMON_PORT})` : 'multi-instance'}`);
    log(`users   -> ${USERS_FILE}`);
    log(`data    -> ${USERS_ROOT}`);
    log(`exe     -> ${resolveExe()}`);
    log(`idle    -> ${IDLE_MINUTES > 0 ? `${IDLE_MINUTES} min` : 'never'}`);
    log(`prewarm -> ${PREWARM_AT ? `${PREWARM_AT} (keep until ${KEEP_UNTIL})` : 'off'}`);
    log(`daily   -> ${DAILY_AT || 'off'}`);
  });
}

main().catch((error) => {
  console.error('[multi] fatal:', error.message);
  process.exit(1);
});
