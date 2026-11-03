/**
 * 批量补「发布时间」或「正文」：逐条触发 `post_detail(refresh=true)`。
 *
 * 走 daemon 的 `/api/ipc/invoke`，等价于界面上对每条帖子点一次「重新抓正文」，
 * 但可以批量、有进度、可中断续跑（已补上的下次自动跳过）。
 *
 * `--reason content` 用于洗「正文退化成标题」的帖子（平台改版、提取器只拿到
 * h1 时会出现）。因为 `post_detail` 有「新正文不短于旧正文才覆盖」的保护，
 * 重抓只会把短的升级成完整的，不会反过来把好数据冲掉，所以跑一遍是无害的。
 *
 * 前置：
 *   - 桌面端必须正在运行（daemon 监听 31937），且平台 Chrome 已登录；
 *   - 正文提取目前对抖音是「仅标题级」的版本必须已修复，否则洗了也没用。
 *
 * 用法：
 *   node scripts/maintenance/backfill-publish-time.mjs --dry-run
 *   node scripts/maintenance/backfill-publish-time.mjs --limit 10
 *   node scripts/maintenance/backfill-publish-time.mjs --reason content --limit 100
 *
 * 参数：
 *   --reason R       `publish-time`（默认，挑 publishedAt 为空的）
 *                    或 `content`（挑正文等于标题、或比标题还短的）
 *   --limit N        本次最多处理多少条（默认 20，先小批量验证）
 *   --concurrency N  并发数（默认 2；每条都会开一个 CDP 标签页，太高容易触发风控）
 *   --platform P     平台（默认 douyin，传 all 表示不限）
 *   --daemon URL     daemon 地址（默认 http://127.0.0.1:31937）
 *   --store PATH     store 路径（默认自动探测）
 *   --dry-run        只列出待处理清单
 */
import fs from 'node:fs';
import { resolveStorePath } from './_store-path.mjs';

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function arg(name, fallback = '') {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
const hasFlag = (name) => process.argv.includes(`--${name}`);

const LIMIT = Number(arg('limit', '20')) || 20;
const CONCURRENCY = Math.max(1, Math.min(6, Number(arg('concurrency', '2')) || 2));
const PLATFORM = arg('platform', 'douyin');
const REASON = arg('reason', 'publish-time');
const DAEMON = String(arg('daemon', 'http://127.0.0.1:31937')).replace(/\/+$/, '');
const DRY = hasFlag('dry-run');

if (!['publish-time', 'content'].includes(REASON)) {
  console.error(`--reason 只支持 publish-time | content，收到: ${REASON}`);
  process.exit(1);
}

const storeResolved = resolveStorePath(arg('store'));
const storeFile = storeResolved.file;
if (!fs.existsSync(storeFile)) {
  console.error(`store 不存在: ${storeFile}（来源: ${storeResolved.from}）`);
  process.exit(1);
}

const isHttpUrl = (v) => /^https?:\/\//.test(String(v || ''));

function loadPosts() {
  const store = JSON.parse(fs.readFileSync(storeFile, 'utf8'));
  const all = Array.isArray(store.monitoredPosts) ? store.monitoredPosts : [];
  return all.filter((p) => {
    if (PLATFORM !== 'all' && String(p.platform) !== PLATFORM) return false;
    if (!isHttpUrl(p.postUrl)) return false;
    if (REASON === 'content') {
      // 正文等于标题、或比标题还短 → 疑似被提取成了标题级
      const c = String(p.content || '').trim();
      const t = String(p.title || '').trim();
      return c === t || c.length < t.length;
    }
    return !String(p.publishedAt || '').trim();
  });
}

const pending = loadPosts();
console.log(`store: ${storeFile}（来源: ${storeResolved.from}）`);
const REASON_LABEL = REASON === 'content' ? '正文疑似标题级' : '缺少发布时间';
console.log(`平台=${PLATFORM} · ${REASON_LABEL} ${pending.length} 条 · 本次处理 ${Math.min(LIMIT, pending.length)} 条 · 并发 ${CONCURRENCY}`);

if (pending.length === 0) {
  console.log('没有需要补的帖子');
  process.exit(0);
}

const batch = pending.slice(0, LIMIT);
if (DRY) {
  console.log('\n[dry-run] 前 10 条：');
  batch.slice(0, 10).forEach((p) => console.log(`  ${p.id}  ${String(p.postUrl).slice(0, 70)}`));
  if (batch.length > 10) console.log(`  … 其余 ${batch.length - 10} 条`);
  process.exit(0);
}

// 先确认 daemon 活着，避免白跑一批全失败
try {
  const health = await fetch(`${DAEMON}/api/ipc/health`, { signal: AbortSignal.timeout(5000) });
  if (!health.ok) throw new Error(`HTTP ${health.status}`);
} catch (e) {
  console.error(`daemon 不可用（${DAEMON}）：${e.message}`);
  console.error('请确认桌面端在运行。');
  process.exit(1);
}

async function refreshOne(post) {
  const body = JSON.stringify({
    channel: 'monitor:post-detail',
    payload: { id: post.id, refresh: true },
  });
  const res = await fetch(`${DAEMON}/api/ipc/invoke`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body,
    signal: AbortSignal.timeout(120_000),
  });
  const text = await res.text();
  let json;
  try { json = JSON.parse(text); } catch { throw new Error(`返回非 JSON: ${text.slice(0, 120)}`); }
  const post2 = json?.post || {};
  if (REASON === 'content') {
    const nowLen = String(post2.content || '').trim().length;
    const beforeLen = String(post.content || '').trim().length;
    // 看的是「有没有变长」而不是绝对长度：短文案本来就可能等于标题
    if (nowLen > beforeLen) return { ok: true, value: `${beforeLen}→${nowLen} 字` };
    return { ok: false, value: '', reason: json?.note || (json?.fetched ? `没变长（${beforeLen}→${nowLen}）` : '抓取失败') };
  }
  const got = String(post2.publishedAt || '').trim();
  if (got) return { ok: true, value: got };
  return { ok: false, value: '', reason: json?.note || (json?.fetched ? '抓到了但没解析出发布时间' : '抓取失败') };
}

const queue = batch.slice();
let done = 0;
let ok = 0;
let failed = 0;
const failures = [];

async function worker(idx) {
  while (queue.length) {
    const post = queue.shift();
    if (!post) break;
    try {
      const r = await refreshOne(post);
      done += 1;
      if (r.ok) {
        ok += 1;
        console.log(`[${done}/${batch.length}] ✔ ${r.value}  ${post.id}`);
      } else {
        failed += 1;
        failures.push({ id: post.id, reason: r.reason });
        console.log(`[${done}/${batch.length}] ✘ ${post.id}  ${r.reason}`);
      }
    } catch (e) {
      done += 1;
      failed += 1;
      failures.push({ id: post.id, reason: e.message });
      console.log(`[${done}/${batch.length}] ✘ ${post.id}  ${e.message}`);
    }
    await sleep(300); // 轻微间隔，降低触发风控的概率
  }
}

await Promise.all(Array.from({ length: Math.min(CONCURRENCY, batch.length) }, (_, i) => worker(i)));

console.log(`\n本次结果：成功 ${ok} · 失败 ${failed}`);
if (failures.length) {
  console.log('失败原因汇总：');
  const byReason = new Map();
  for (const f of failures) byReason.set(f.reason, (byReason.get(f.reason) || 0) + 1);
  for (const [reason, n] of [...byReason.entries()].sort((a, b) => b[1] - a[1])) {
    console.log(`  ${String(n).padStart(3)} 条  ${reason}`);
  }
}

const remain = loadPosts().length;
console.log(`\n仍待补：${remain} 条（再跑一次本脚本可继续，已补上的会自动跳过）`);
