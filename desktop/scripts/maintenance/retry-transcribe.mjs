/**
 * 批量重跑失败的语音转写。
 *
 * 逐条打 daemon 的 `monitor:transcribe-video`（只补转写，不重抽帧 —— 抽帧在
 * 「视频没变就跳过」之后已经很快，但仍会重新读一遍帧目录）。
 *
 * 按 `postVideoAnalyses[].transcriptStatus` 分类处理：
 *   · `failed`         → 直接重试转写（配额/网络/模型问题，通常已自愈）
 *   · `no-audio`       → 源视频无音轨，**先走 `monitor:download-video {patchAudio:true}`
 *                        补音轨**，成功后再转写；补音轨失败则跳过
 *   · `not-configured` → 没配 ASR 服务，重试无意义 → 跳过
 *   · `pending`        → 抽过帧但没转写（被 skipTranscript 打断）→ 直接重试
 *
 * 用法：
 *   node scripts/maintenance/retry-transcribe.mjs --dry-run
 *   node scripts/maintenance/retry-transcribe.mjs --limit 20
 *   node scripts/maintenance/retry-transcribe.mjs --limit 50 --concurrency 2
 *
 * 参数：
 *   --status S     只处理指定状态（默认 failed,pending；可给 no-audio；逗号分隔）
 *   --limit N      本次最多处理多少条（默认 20）
 *   --concurrency N 并发数（默认 2；ASR 服务普遍限流）
 *   --platform P   平台（默认 all）
 *   --watch-id W   只处理某个监控项下的帖子
 *   --daemon URL   daemon 地址（默认 http://127.0.0.1:31937）
 *   --store PATH   store 路径（默认自动探测）
 *   --dry-run      只列清单
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
const CONCURRENCY = Math.max(1, Math.min(4, Number(arg('concurrency', '2')) || 2));
const PLATFORM = arg('platform', 'all');
const WATCH_ID = arg('watch-id', '');
const DAEMON = String(arg('daemon', 'http://127.0.0.1:31937')).replace(/\/+$/, '');
const DRY = hasFlag('dry-run');
const WANT = new Set(
  String(arg('status', 'failed,pending'))
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean),
);

const storeResolved = resolveStorePath(arg('store'));
const storeFile = storeResolved.file;
if (!fs.existsSync(storeFile)) {
  console.error(`store 不存在: ${storeFile}（来源: ${storeResolved.from}）`);
  process.exit(1);
}

const store = JSON.parse(fs.readFileSync(storeFile, 'utf8'));
const posts = Array.isArray(store.monitoredPosts) ? store.monitoredPosts : [];
const analyses = Array.isArray(store.postVideoAnalyses) ? store.postVideoAnalyses : [];

/** postId → 分析记录 */
const analysisByPost = new Map(analyses.map((a) => [a.postId, a]));

const candidates = [];
for (const post of posts) {
  if (PLATFORM !== 'all' && String(post.platform) !== PLATFORM) continue;
  if (WATCH_ID && String(post.watchId ?? '') !== WATCH_ID) continue;
  const analysis = analysisByPost.get(post.id);
  const status = String(analysis?.transcriptStatus ?? '');
  if (!WANT.has(status)) continue;
  // 必须有本地视频才能转写（transcribe-video 依赖 video_local_path）
  if (!post.videoLocalPath) continue;
  candidates.push({
    id: post.id,
    platform: post.platform,
    status: status || '(空)',
    hasAudio: post.videoHasAudio,
    title: String(post.title || '').slice(0, 40),
    error: String(analysis?.transcriptError ?? '').slice(0, 120),
  });
}

console.log(`store: ${storeFile}（来源: ${storeResolved.from}）`);
console.log(`筛选状态: ${[...WANT].join(', ')}`);
console.log(`待重跑 ${candidates.length} 条 · 本次处理 ${Math.min(LIMIT, candidates.length)} 条 · 并发 ${CONCURRENCY}`);

const byStatus = new Map();
for (const c of candidates) byStatus.set(c.status, (byStatus.get(c.status) || 0) + 1);
console.log('状态分布: ' + [...byStatus.entries()].map(([k, v]) => `${k}=${v}`).join(' '));

if (candidates.length === 0) {
  console.log('\n没有需要重跑的转写');
  process.exit(0);
}

const batch = candidates.slice(0, LIMIT);
if (DRY) {
  console.log('\n[dry-run] 前 10 条：');
  batch.slice(0, 10).forEach((c) => {
    console.log(`  [${c.status}] hasAudio=${c.hasAudio}  ${c.title}`);
    if (c.error) console.log(`      原因: ${c.error}`);
  });
  if (batch.length > 10) console.log(`  … 其余 ${batch.length - 10} 条省略`);
  process.exit(0);
}

try {
  const health = await fetch(`${DAEMON}/api/ipc/health`, { signal: AbortSignal.timeout(5000) });
  if (!health.ok) throw new Error(`HTTP ${health.status}`);
} catch (e) {
  console.error(`daemon 不可用（${DAEMON}）：${e.message}`);
  console.error('请确认桌面端在运行（转写必须由它执行：CDP + 写盘都在它那边）。');
  process.exit(1);
}

async function invoke(channel, payload, timeoutMs) {
  const res = await fetch(`${DAEMON}/api/ipc/invoke`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ channel, payload }),
    signal: AbortSignal.timeout(timeoutMs),
  });
  const text = await res.text();
  try { return JSON.parse(text); } catch { return { success: false, note: text.slice(0, 160) }; }
}

/** 无音轨的先补音轨（patchAudio：不重下视频轨，只补音频轨再合并）。 */
async function patchAudio(id) {
  const r = await invoke('monitor:download-video', { id, patchAudio: true }, 600_000);
  return Boolean(r?.success);
}

async function retryOne(c) {
  // 无音轨：先补音轨再转写
  if (c.status === 'no-audio' || c.hasAudio === false) {
    const patched = await patchAudio(c.id);
    if (!patched) return { ok: false, value: '', reason: '补音轨失败' };
  }
  const r = await invoke('monitor:transcribe-video', { id: c.id }, 300_000);
  const status = String(r?.analysis?.transcriptStatus ?? r?.status ?? '');
  if (r?.success && status === 'ok') {
    const len = String(r?.analysis?.transcript ?? '').length;
    return { ok: true, value: `ok ${len} 字` };
  }
  const reason = String(r?.analysis?.transcriptError ?? r?.note ?? '');
  return { ok: false, value: '', reason: reason || `状态=${status || '未知'}` };
}

const queue = batch.slice();
let done = 0;
let ok = 0;
let failed = 0;
const failures = new Map();

async function worker() {
  while (queue.length) {
    const c = queue.shift();
    if (!c) break;
    try {
      const r = await retryOne(c);
      done += 1;
      if (r.ok) {
        ok += 1;
        console.log(`[${done}/${batch.length}] ✔ ${r.value}  ${c.id}`);
      } else {
        failed += 1;
        failures.set(r.reason, (failures.get(r.reason) || 0) + 1);
        console.log(`[${done}/${batch.length}] ✘ ${c.id}  ${r.reason}`);
      }
    } catch (e) {
      done += 1;
      failed += 1;
      failures.set(e.message, (failures.get(e.message) || 0) + 1);
      console.log(`[${done}/${batch.length}] ✘ ${c.id}  ${e.message}`);
    }
    await sleep(500); // 轻微间隔，别把 ASR 服务打爆
  }
}

await Promise.all(Array.from({ length: Math.min(CONCURRENCY, batch.length) }, () => worker()));

console.log(`\n本次结果：成功 ${ok} · 失败 ${failed}`);
if (failures.size) {
  console.log('失败原因汇总：');
  for (const [reason, n] of [...failures.entries()].sort((a, b) => b[1] - a[1])) {
    console.log(`  ${String(n).padStart(3)} 条  ${reason}`);
  }
}

// 重新读 store 报剩余
const fresh = JSON.parse(fs.readFileSync(storeFile, 'utf8'));
const freshAnalyses = fresh.postVideoAnalyses ?? [];
const freshByPost = new Map(freshAnalyses.map((a) => [a.postId, a]));
const remain = (fresh.monitoredPosts ?? []).filter((p) => {
  if (PLATFORM !== 'all' && String(p.platform) !== PLATFORM) return false;
  if (WATCH_ID && String(p.watchId ?? '') !== WATCH_ID) return false;
  const st = String(freshByPost.get(p.id)?.transcriptStatus ?? '');
  return WANT.has(st);
}).length;
console.log(`\n仍待重跑：${remain} 条（再跑一次本脚本可继续）`);
