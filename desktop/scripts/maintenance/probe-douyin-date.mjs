/**
 * 抖音发布时间探针（只读，不改 store、不改仓库）。
 *
 * 用途：查清「抖音帖抓不到发布时间」到底是哪一层断了。逐条打印三条提取来源：
 *   1. DOM 选择器 [data-e2e=video-publish-time] / .video-info-detail …
 *   2. SSR __INITIAL_STATE__.itemModule[vid].createTime
 *   3. SSR _ROUTER_DATA.loaderData[*].videoInfoRes.item_list[*].create_time
 *   4. 兜底：详情区可见文本里的 YYYY-MM-DD
 *
 * 复用应用已经拉起的那个 Chrome：Chrome 会把实际调试端口写进
 * `<profile>/DevToolsActivePort`（因为应用用的是 --remote-debugging-port=0，
 * 命令行里看不到真实端口）。这样不用另起一个浏览器，也就不会撞 profile 锁。
 *
 * 用法：
 *   node scripts/maintenance/probe-douyin-date.mjs                  # 自动从 store 挑一条抖音帖
 *   node scripts/maintenance/probe-douyin-date.mjs --url <视频URL>
 *   node scripts/maintenance/probe-douyin-date.mjs --store <路径>
 *
 * 前提：应用（或它拉起的 Chrome）正在运行，且抖音已登录。
 */
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { resolveStorePath } from './_store-path.mjs';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function arg(name, fallback = '') {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const BASE_DIR = process.platform === 'win32'
  ? (process.env.APPDATA ?? path.join(process.env.USERPROFILE ?? '', 'AppData', 'Roaming'))
  : path.join(process.env.HOME ?? '', 'Library', 'Application Support');

const CHROME_BIN = process.platform === 'darwin'
  ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'
  : process.platform === 'win32'
    ? 'C:/Program Files/Google/Chrome/Application/chrome.exe'
    : '/usr/bin/google-chrome';

/** 应用的 Chrome profile 候选目录（app_data_dir 与 store 目录不是同一个位置）。 */
function profileCandidates() {
  const list = [
    process.env.BEAV_CHROME_PROFILE_DIR && path.join(process.env.BEAV_CHROME_PROFILE_DIR, 'douyin'),
    process.env.BEAV_STORE_DIR && path.join(process.env.BEAV_STORE_DIR, 'browser-profile', 'douyin'),
    path.join(BASE_DIR, 'com.redconvert.beav', 'browser-profile', 'douyin'),
    path.join(BASE_DIR, 'Beav', 'browser-profile', 'douyin'),
    path.join(BASE_DIR, 'RedBox', 'browser-profile', 'douyin'),
  ];
  return [...new Set(list.filter(Boolean))];
}

function findProfile() {
  for (const dir of profileCandidates()) {
    if (fs.existsSync(dir)) return dir;
  }
  return '';
}

/** Chrome 把实际调试端口写在 profile 根的 DevToolsActivePort 第一行。 */
function readDebugPort(profileDir) {
  // DevToolsActivePort 在 user-data-dir 根目录（我们传的是 douyin 子目录，再往上一层也要试）
  const tried = [profileDir, path.dirname(profileDir)];
  for (const dir of tried) {
    const file = path.join(dir, 'DevToolsActivePort');
    if (!fs.existsSync(file)) continue;
    const first = fs.readFileSync(file, 'utf8').split('\n')[0].trim();
    const port = Number(first);
    if (Number.isFinite(port) && port > 0) return port;
  }
  return 0;
}

async function portAlive(port) {
  try {
    const r = await fetch(`http://127.0.0.1:${port}/json/version`, { signal: AbortSignal.timeout(1500) });
    return r.ok;
  } catch {
    return false;
  }
}

/** 拿到 browser 级 WebSocket 地址：优先复用已开的 Chrome，没有就用同一 profile 起一个。 */
async function acquireBrowser(profileDir) {
  const port = readDebugPort(profileDir);
  if (port && (await portAlive(port))) {
    const r = await fetch(`http://127.0.0.1:${port}/json/version`);
    const info = await r.json();
    return { wsUrl: info.webSocketDebuggerUrl, port, started: false };
  }
  if (!fs.existsSync(CHROME_BIN)) {
    throw new Error(`找不到 Chrome：${CHROME_BIN}`);
  }
  console.log('未发现运行中的调试端口，用同一 profile 启动一个 Chrome …');
  spawn(CHROME_BIN, [
    '--remote-debugging-port=0',
    // 必须用 profileDir 本身：应用是 `--user-data-dir=<browser-profile>/<platform>`，
    // 传父目录会变成一个全新的空 profile，抖音登录态全丢。
    `--user-data-dir=${profileDir}`,
    '--no-first-run',
    '--no-default-browser-check',
    '--window-size=1280,900',
  ], { detached: true, stdio: 'ignore' }).unref();

  for (let i = 0; i < 40; i += 1) {
    await sleep(500);
    const p = readDebugPort(profileDir);
    if (p && (await portAlive(p))) {
      const r = await fetch(`http://127.0.0.1:${p}/json/version`);
      const info = await r.json();
      return { wsUrl: info.webSocketDebuggerUrl, port: p, started: true };
    }
  }
  throw new Error('等待 Chrome 调试端口超时（20s）');
}

class Cdp {
  constructor(ws) { this.ws = ws; this.id = 0; this.pending = new Map(); }
  static async connect(url) {
    const ws = new WebSocket(url);
    await new Promise((res, rej) => { ws.onopen = res; ws.onerror = () => rej(new Error('ws 连接失败')); });
    const c = new Cdp(ws);
    ws.onmessage = (ev) => {
      const msg = JSON.parse(ev.data);
      if (msg.id && c.pending.has(msg.id)) {
        const { res, rej } = c.pending.get(msg.id);
        c.pending.delete(msg.id);
        msg.error ? rej(new Error(JSON.stringify(msg.error))) : res(msg.result);
      }
    };
    return c;
  }
  send(method, params = {}, sessionId) {
    const id = ++this.id;
    return new Promise((res, rej) => {
      this.pending.set(id, { res, rej });
      this.ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
      setTimeout(() => {
        if (this.pending.has(id)) { this.pending.delete(id); rej(new Error(`timeout ${method}`)); }
      }, 60000);
    });
  }
}

const PROBE = String.raw`(() => {
  const txt = (sel) => { const e = document.querySelector(sel); return e ? (e.textContent || '').trim() : ''; };
  const vid = (location.pathname.match(/\/video\/(\d+)/) || [])[1] || '';

  const state = window.__INITIAL_STATE__ || null;
  const im = state && state.itemModule ? state.itemModule : null;
  const imKeys = im ? Object.keys(im) : null;
  const it = (vid && im) ? (im[vid] || null) : null;

  let router = null;
  try {
    const rd = window._ROUTER_DATA;
    const ld = rd && (rd.loaderData || rd.loader_data);
    if (ld) {
      const keys = Object.keys(ld);
      for (const k of keys) {
        const arr = ld[k] && ld[k].videoInfoRes && ld[k].videoInfoRes.item_list;
        if (Array.isArray(arr) && arr.length) {
          router = {
            loaderKeys: keys,
            hitKey: k,
            count: arr.length,
            firstIds: arr.slice(0, 3).map((x) => String((x && (x.aweme_id || x.awemeId)) || '')),
            firstCreates: arr.slice(0, 3).map((x) => (x && (x.create_time || x.createTime)) || 0),
          };
          break;
        }
      }
      if (!router) router = { loaderKeys: keys, hitKey: null };
    }
  } catch (e) { router = { error: String(e) }; }

  const area = document.querySelector('[data-e2e="video-detail"], .video-info-detail, [data-e2e="video-info"]');
  const areaText = area ? area.textContent.replace(/\s+/g, ' ').slice(0, 300) : '';

  // 关键：抖音不再注入 __INITIAL_STATE__ / _ROUTER_DATA，旧的 data-e2e 选择器也全废，
  // 日期只以纯文本存在。这里把「含日期的文本节点」连同它的祖先链一起抓出来，
  // 好据此写出稳定选择器（而不是靠 body.innerText 瞎猜）。
  const dateHits = [];
  try {
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    let node;
    let budget = 30000;
    while ((node = walker.nextNode()) && budget-- > 0) {
      const t = (node.nodeValue || '').trim();
      if (!t) continue;
      if (!/(20\d{2}-\d{2}-\d{2})/.test(t) && !/^\d{1,2}-\d{1,2}$/.test(t)) continue;
      const el = node.parentElement;
      if (!el) continue;
      const chain = [];
      let cur = el;
      for (let i = 0; i < 4 && cur; i += 1) {
        chain.push(cur.tagName.toLowerCase() + (cur.className ? '.' + String(cur.className).split(/\s+/).slice(0, 2).join('.') : ''));
        cur = cur.parentElement;
      }
      dateHits.push({
        text: t.slice(0, 40),
        tag: el.tagName.toLowerCase(),
        cls: String(el.className || '').slice(0, 100),
        dataE2e: el.getAttribute && (el.getAttribute('data-e2e') || ''),
        chain,
        context: (el.parentElement ? el.parentElement.textContent : '').replace(/\s+/g, ' ').slice(0, 100),
      });
      if (dateHits.length >= 10) break;
    }
  } catch (e) { dateHits.push({ error: String(e) }); }

  const bodyYmd = (document.body.innerText.match(/(?:^|[^\d])(20\d{2}-\d{2}-\d{2})(?![\d])/) || [])[1] || '';
  const bodyMd = (document.body.innerText.match(/(?:^|[^\d])(\d{1,2}-\d{1,2})(?![\d])/) || [])[1] || '';

  return JSON.stringify({
    url: location.href,
    vid,
    hasInitialState: !!state,
    stateTopKeys: state ? Object.keys(state).slice(0, 12) : null,
    itemModuleKeys: imKeys ? imKeys.slice(0, 5) : null,
    itemModuleHit: !!it,
    itemCreateTime: it ? (it.createTime || it.create_time || 0) : 0,
    routerData: router,
    dom: {
      // 直接列出应用实际会用的选择器，逐一取值 —— 这样能判断「选择器对不对」
      // 与「应用求值时机对不对」是两件事。
      appDetailVideoPublishTime: txt('[data-e2e="detail-video-publish-time"]'),
      appOldPublishTime: txt('[data-e2e="video-publish-time"]'),
      appVideoDesc: txt('[data-e2e="video-desc"]'),
      appVideoInfoDetail: txt('.video-info-detail'),
      appAuthorName: txt('[data-e2e="author-name"]'),
    },
    areaTextSample: areaText,
    dateHits,
    bodyHasYmd: bodyYmd,
    bodyHasMd: bodyMd,
    looksLikeLoginWall: /扫码登录|登录后查看|验证|安全验证/.test(document.body.innerText.slice(0, 800)),
    pageTitle: document.title.slice(0, 80),
  }, null, 2);
})()`;

(async () => {
  const storeResolved = resolveStorePath(arg('store'));
  const storeFile = storeResolved.file;
  if (!fs.existsSync(storeFile)) {
    console.error(`store 不存在: ${storeFile}（来源: ${storeResolved.from}）`);
    process.exit(1);
  }
  const store = JSON.parse(fs.readFileSync(storeFile, 'utf8'));
  const posts = Array.isArray(store.monitoredPosts) ? store.monitoredPosts : [];
  const douyin = posts.filter((p) => String(p.platform) === 'douyin' && /\/video\/\d+/.test(String(p.postUrl || '')));

  let targetUrl = arg('url');
  if (!targetUrl) {
    // 优先挑「有正文但没发布时间」的，最贴近问题现场
    const pick = douyin.find((p) => !String(p.publishedAt || '').trim() && String(p.content || '').trim())
      || douyin[0];
    if (!pick) { console.error('store 里没有抖音视频帖，请用 --url 指定'); process.exit(1); }
    targetUrl = pick.postUrl;
    console.log(`从 store 挑了一条：${pick.id}  publishedAt=${JSON.stringify(pick.publishedAt)}`);
  } else {
    const hit = douyin.find((p) => String(p.postUrl || '').split('?')[0] === targetUrl.split('?')[0]);
    if (hit) {
      console.log(`store 记录：publishedAt=${JSON.stringify(hit.publishedAt)}  detailFetched=${hit.detailFetched}  updatedAt=${hit.updatedAt}`);
    } else {
      console.log('store 里没有这条 URL（可能是临时抓取的，或 URL 带参数不一致）');
    }
  }
  console.log(`目标 URL: ${targetUrl}`);

  const profileDir = findProfile();
  if (!profileDir) {
    console.error('找不到 Chrome profile 目录，候选：');
    profileCandidates().forEach((d) => console.error('  ' + d));
    console.error('可设置 BEAV_CHROME_PROFILE_DIR 指定。');
    process.exit(1);
  }
  console.log(`profile: ${profileDir}`);

  const { wsUrl, port, started } = await acquireBrowser(profileDir);
  console.log(`CDP: ${wsUrl}（端口 ${port}${started ? '，本次新启动' : '，复用运行中的 Chrome'}）`);

  const cdp = await Cdp.connect(wsUrl);
  const { targetId } = await cdp.send('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: true });
  await cdp.send('Page.enable', {}, sessionId);
  await cdp.send('Runtime.enable', {}, sessionId);

  console.log('打开页面并等待渲染（约 12s）…');
  await cdp.send('Page.navigate', { url: targetUrl }, sessionId);
  await sleep(12000);

  const out = await cdp.send('Runtime.evaluate', { expression: PROBE, returnByValue: true }, sessionId);
  console.log('\n===== 探针结果 =====');
  console.log(out?.result?.value ?? JSON.stringify(out, null, 2));

  await cdp.send('Target.closeTarget', { targetId }).catch(() => {});
  process.exit(0);
})().catch((e) => {
  console.error('探针失败:', e.message);
  process.exit(1);
});
