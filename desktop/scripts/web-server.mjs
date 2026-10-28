#!/usr/bin/env node
/**
 * Beav Web 桥接服务（非侵入式，不改 Rust）
 *
 * 作用：
 *  1) 静态托管 `dist/`（SPA fallback 到 index.html）
 *  2) 把同源 `/api/ipc/*` 反代到本机 assistant daemon（默认 127.0.0.1:31937），
 *     并把 Origin 改写成 loopback，从而通过 daemon 的「仅本机 localhost」校验
 *  3) `/api/file?path=` 读取服务器本地文件（目录白名单），解决浏览器读不了 file:// 的问题
 *  4) 可选 Basic Auth（WEB_BASIC_USER / WEB_BASIC_PASS）
 *
 * 用法：
 *  pnpm web:build           # 先构建 dist（beav 品牌）
 *  pnpm web:serve           # 启动桥接服务，浏览器打开 http://<本机IP>:8080
 *
 * 环境变量：
 *  WEB_HOST           监听地址，默认 0.0.0.0
 *  WEB_PORT           监听端口，默认 8080
 *  BEAV_DAEMON        daemon 地址，默认 http://127.0.0.1:31937
 *  WEB_DIST           dist 目录，默认 ../dist
 *  BEAV_FILE_ROOTS    额外允许读取的目录，分号分隔
 *  WEB_BASIC_USER / WEB_BASIC_PASS  可选 Basic Auth
 */
import http from 'node:http';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const DIST_DIR = path.resolve(process.env.WEB_DIST || path.join(__dirname, '..', 'dist'));
const WEB_HOST = process.env.WEB_HOST || '0.0.0.0';
const WEB_PORT = Number(process.env.WEB_PORT || 8080);
const DAEMON = (process.env.BEAV_DAEMON || 'http://127.0.0.1:31937').replace(/\/+$/, '');
const DAEMON_ORIGIN = new URL(DAEMON).origin;
const BASIC_USER = process.env.WEB_BASIC_USER || '';
const BASIC_PASS = process.env.WEB_BASIC_PASS || '';

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.map': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.webp': 'image/webp',
  '.gif': 'image/gif',
  '.bmp': 'image/bmp',
  '.ico': 'image/x-icon',
  '.avif': 'image/avif',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
  '.ttf': 'font/ttf',
  '.otf': 'font/otf',
  '.mp4': 'video/mp4',
  '.webm': 'video/webm',
  '.mov': 'video/quicktime',
  '.mp3': 'audio/mpeg',
  '.wav': 'audio/wav',
  '.m4a': 'audio/mp4',
  '.srt': 'text/plain; charset=utf-8',
  '.txt': 'text/plain; charset=utf-8',
  '.md': 'text/markdown; charset=utf-8',
  '.wasm': 'application/wasm',
};

const fileRoots = [];
let rootsPromise = null;

function log(...args) {
  console.log('[web]', ...args);
}

function lanUrls(port) {
  const urls = [];
  for (const list of Object.values(os.networkInterfaces())) {
    for (const net of list || []) {
      if (net.family === 'IPv4' && !net.internal) {
        urls.push(`http://${net.address}:${port}`);
      }
    }
  }
  return urls;
}

function isWithin(child, parent) {
  const rel = path.relative(parent, child);
  return rel === '' || (!rel.startsWith('..') && !path.isAbsolute(rel));
}

function addFileRoot(dir, label) {
  if (!dir) return;
  try {
    const resolved = fs.realpathSync(path.resolve(dir));
    if (!fs.statSync(resolved).isDirectory()) return;
    if (fileRoots.some((root) => root === resolved)) return;
    fileRoots.push(resolved);
    log(`file root (${label}): ${resolved}`);
  } catch {
    log(`file root skip (${label}): ${dir}`);
  }
}

function mimeFor(filePath) {
  return MIME[path.extname(filePath).toLowerCase()] || 'application/octet-stream';
}

function basicAuthOk(req) {
  if (!BASIC_USER && !BASIC_PASS) return true;
  const header = String(req.headers.authorization || '');
  if (!header.startsWith('Basic ')) return false;
  const decoded = Buffer.from(header.slice(6), 'base64').toString('utf8');
  const index = decoded.indexOf(':');
  if (index < 0) return false;
  return decoded.slice(0, index) === BASIC_USER && decoded.slice(index + 1) === BASIC_PASS;
}

function sendJson(res, status, body) {
  const payload = JSON.stringify(body);
  res.writeHead(status, {
    'Content-Type': 'application/json; charset=utf-8',
    'Content-Length': Buffer.byteLength(payload),
    'Cache-Control': 'no-store',
  });
  res.end(payload);
}

function proxyToDaemon(req, res, rawPath) {
  let target;
  try {
    target = new URL(rawPath, DAEMON);
  } catch {
    return sendJson(res, 400, { success: false, error: 'bad path' });
  }

  // 先把请求体收全：daemon 是手写 HTTP 解析器，**不支持 Transfer-Encoding: chunked**。
  // 直接 pipe 且删掉 Content-Length 会退化成 chunked，请求体变成「分块长度+JSON」，
  // daemon 解析报 `trailing characters at line 1 column 2`（把 7b 当 JSON 数字）。
  const chunks = [];
  req.on('data', (chunk) => chunks.push(chunk));
  req.on('error', () => {
    if (!res.headersSent) sendJson(res, 400, { success: false, error: 'request body error' });
  });
  req.on('end', () => {
    const body = Buffer.concat(chunks);
    const headers = { ...req.headers };
    // daemon 只信任 loopback 来源：改写 Origin/Referer，Host 交给 daemon
    headers.origin = DAEMON_ORIGIN;
    headers.referer = `${DAEMON_ORIGIN}/`;
    headers.host = target.host;
    delete headers['transfer-encoding'];
    if (body.length > 0) {
      headers['content-length'] = String(body.length);
    } else {
      delete headers['content-length'];
    }

    const upstream = http.request(
      {
        protocol: target.protocol,
        hostname: target.hostname,
        port: target.port || 80,
        method: req.method,
        path: `${target.pathname}${target.search}`,
        headers,
      },
      (upstreamRes) => {
        const outHeaders = { ...upstreamRes.headers };
        // 同源部署无需 CORS，去掉上游的 * 以免误导
        delete outHeaders['access-control-allow-origin'];
        res.writeHead(upstreamRes.statusCode || 502, outHeaders);
        upstreamRes.pipe(res);
      },
    );
    upstream.on('error', (error) => {
      log('daemon proxy error:', error.message);
      if (!res.headersSent) {
        sendJson(res, 502, {
          success: false,
          error: `无法连接 Beav daemon（${DAEMON}）：${error.message}。请确认桌面端 App 正在运行。`,
        });
      } else {
        res.destroy();
      }
    });
    upstream.end(body);
  });
}

function serveFile(req, res, absolutePath, { allowSpaFallback = false } = {}) {
  let stat;
  try {
    stat = fs.statSync(absolutePath);
  } catch {
    if (allowSpaFallback) {
      return serveFile(req, res, path.join(DIST_DIR, 'index.html'), { allowSpaFallback: false });
    }
    return sendJson(res, 404, { success: false, error: 'not found' });
  }
  if (stat.isDirectory()) {
    return serveFile(req, res, path.join(absolutePath, 'index.html'), { allowSpaFallback });
  }

  const headers = {
    'Content-Type': mimeFor(absolutePath),
    'Cache-Control': absolutePath.includes(`${path.sep}assets${path.sep}`)
      ? 'public, max-age=31536000, immutable'
      : 'no-cache',
    'Accept-Ranges': 'bytes',
  };

  const range = String(req.headers.range || '');
  const match = /^bytes=(\d*)-(\d*)$/.exec(range);
  if (match && stat.size > 0) {
    let start = match[1] ? Number(match[1]) : 0;
    let end = match[2] ? Number(match[2]) : stat.size - 1;
    if (!match[1] && match[2]) {
      start = Math.max(0, stat.size - Number(match[2]));
      end = stat.size - 1;
    }
    if (start > end || start >= stat.size) {
      res.writeHead(416, { ...headers, 'Content-Range': `bytes */${stat.size}` });
      return res.end();
    }
    end = Math.min(end, stat.size - 1);
    headers['Content-Range'] = `bytes ${start}-${end}/${stat.size}`;
    headers['Content-Length'] = end - start + 1;
    res.writeHead(206, headers);
    if (req.method === 'HEAD') return res.end();
    return fs.createReadStream(absolutePath, { start, end }).pipe(res);
  }

  headers['Content-Length'] = stat.size;
  res.writeHead(200, headers);
  if (req.method === 'HEAD') return res.end();
  return fs.createReadStream(absolutePath).pipe(res);
}

async function ensureFileRoots() {
  if (fileRoots.length) return;
  if (!rootsPromise) {
    rootsPromise = discoverDaemonState().finally(() => {
      rootsPromise = null;
    });
  }
  await rootsPromise;
}

async function handleFileProxy(req, res, url) {
  const requested = url.searchParams.get('path') || '';
  if (!requested) return sendJson(res, 400, { success: false, error: 'missing path' });
  if (!fileRoots.length) await ensureFileRoots();
  if (!fileRoots.length) {
    return sendJson(res, 403, {
      success: false,
      error: '未配置可读目录（BEAV_FILE_ROOTS / daemon 导出根）；请确认桌面端已运行，或设置 BEAV_FILE_ROOTS',
    });
  }
  let resolved;
  try {
    resolved = fs.realpathSync(path.resolve(decodeURIComponent(requested)));
  } catch {
    return sendJson(res, 404, { success: false, error: 'file not found' });
  }
  if (!fileRoots.some((root) => isWithin(resolved, root))) {
    return sendJson(res, 403, { success: false, error: 'path outside allowed roots' });
  }
  return serveFile(req, res, resolved);
}

const server = http.createServer((req, res) => {
  if (!basicAuthOk(req)) {
    res.writeHead(401, { 'WWW-Authenticate': 'Basic realm="Beav", charset="UTF-8"' });
    return res.end('auth required');
  }
  const url = new URL(req.url || '/', `http://${req.headers.host || 'localhost'}`);
  const pathname = decodeURIComponent(url.pathname);

  if (req.method === 'OPTIONS') {
    res.writeHead(204);
    return res.end();
  }

  // 1) IPC 反代
  if (pathname === '/api/ipc' || pathname.startsWith('/api/ipc/')) {
    if (req.method !== 'GET' && req.method !== 'POST' && req.method !== 'HEAD') {
      return sendJson(res, 405, { success: false, error: 'method not allowed' });
    }
    return proxyToDaemon(req, res, pathname + url.search);
  }

  // 2) /api/file 也交给 daemon：目录白名单（工作区 + 导出根）只在 daemon 侧维护一份，
  //    避免 Node 与 Rust 两套白名单不一致（例如 monitor-images 在工作区下）。

  // 其他 /api/*（含 /api/file）交给 daemon（accounts / knowledge 等）
  if (pathname.startsWith('/api/')) {
    return proxyToDaemon(req, res, pathname + url.search);
  }

  // 3) 静态资源
  if (req.method !== 'GET' && req.method !== 'HEAD') {
    return sendJson(res, 405, { success: false, error: 'method not allowed' });
  }
  const relative = pathname.replace(/^\/+/, '');
  const target = path.resolve(DIST_DIR, relative || 'index.html');
  if (!isWithin(target, DIST_DIR)) {
    return sendJson(res, 403, { success: false, error: 'forbidden' });
  }
  const hasExtension = path.extname(target) !== '';
  return serveFile(req, res, target, { allowSpaFallback: !hasExtension });
});

async function discoverDaemonState() {
  const health = await fetch(`${DAEMON}/api/ipc/health`).catch(() => null);
  log(`daemon health: ${health ? health.status : 'unreachable'}`);
  try {
    const response = await fetch(`${DAEMON}/api/ipc/invoke`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', Origin: DAEMON_ORIGIN },
      body: JSON.stringify({ channel: 'monitor:get-export-root', payload: {} }),
    });
    const value = await response.json().catch(() => null);
    const root = value && typeof value === 'object' ? value.root : '';
    if (typeof root === 'string' && root.trim()) {
      addFileRoot(root, 'daemon export root');
    }
  } catch (error) {
    log('无法从 daemon 读取导出根目录：', error.message);
  }
}

async function main() {
  if (!fs.existsSync(path.join(DIST_DIR, 'index.html'))) {
    log(`警告：${DIST_DIR}/index.html 不存在，请先执行 \`pnpm web:build\``);
  }
  for (const dir of String(process.env.BEAV_FILE_ROOTS || '').split(/[;,]/)) {
    addFileRoot(dir.trim(), 'env');
  }
  await discoverDaemonState();
  if (path.isAbsolute(process.env.BEAV_FILE_ROOTS || '')) {
    // 单路径便捷写法
    addFileRoot(process.env.BEAV_FILE_ROOTS, 'env');
  }
  server.listen(WEB_PORT, WEB_HOST, () => {
    log(`listening on http://${WEB_HOST}:${WEB_PORT}`);
    for (const url of lanUrls(WEB_PORT)) log(`LAN: ${url}`);
    log(`daemon  -> ${DAEMON}`);
    log(`dist    -> ${DIST_DIR}`);
    log(`auth    -> ${BASIC_USER || BASIC_PASS ? 'basic' : 'off'}`);
  });
}

main();
