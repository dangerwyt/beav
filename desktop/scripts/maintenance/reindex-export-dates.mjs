/**
 * 按「发布时间」重新归位导出目录的日期段。
 *
 * 背景：归档日期以前取「导出当天」，同一篇内容每天凌晨重抓就会漂出一个新日期目录。
 * 现在导出侧已改成取发布时间（commit 0f1d02e），但历史目录还挂在旧的导出日下。
 *
 * 范围：只动**发布时间 >= 20261001** 的帖子；更早的一律不碰（用户要求，
 * 避免大批历史素材换父目录）。没有 publishedAt 的也不动。
 *
 * 只改路径里的「日期」段，作者/标题目录名原样保留 —— 不做重命名，避免和
 * rename-export-dirs 的命名规则纠缠。
 *
 * 用法：
 *   node scripts/maintenance/reindex-export-dates.mjs              # 预演（默认）
 *   node scripts/maintenance/reindex-export-dates.mjs --csv a.csv   # 预演 + 导出映射表
 *   node scripts/maintenance/reindex-export-dates.mjs --apply       # 真正移动
 *
 * 冲突处理：目标目录已存在时不合并、不覆盖，只报告（留给人判断）。
 */
import fs from 'node:fs';
import path from 'node:path';
import { resolveStorePath } from './_store-path.mjs';

const CUTOFF = '20261001'; // 早于这个发布日的目录一律不动

function arg(name, fallback = '') {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
const APPLY = process.argv.includes('--apply');
const CSV_PATH = arg('csv');

const storeResolved = resolveStorePath(arg('store'));
if (!fs.existsSync(storeResolved.file)) {
  console.error(`store 不存在: ${storeResolved.file}`);
  process.exit(1);
}
const store = JSON.parse(fs.readFileSync(storeResolved.file, 'utf8'));
const root = path.resolve(String(arg('root') || store.monitorExportRoot || '').trim());
if (!root || !fs.existsSync(root)) {
  console.error(`导出根不存在: ${root || '(未配置 monitorExportRoot)'}`);
  process.exit(1);
}
console.log(`导出根: ${root}`);
console.log(APPLY ? '模式: APPLY（会移动目录）' : '模式: dry-run（只看不动）');

/** 从 publishedAt 抠出 YYYYMMDD（兼容 2026-10-01 / 2026/10/01 / 20261001） */
function publishedDate(text) {
  const raw = String(text ?? '');
  const match = raw.match(/(20\d{2})[-/年]?(\d{1,2})[-/月]?(\d{1,2})/);
  if (!match) return '';
  const [, y, m, d] = match;
  return `${y}${m.padStart(2, '0')}${d.padStart(2, '0')}`;
}

/** 递归找产物目录（带 .beav-post-id 的目录），跳过备份/素材包 */
function collect(dir, depth = 0, out = []) {
  if (depth > 6) return out;
  let entries;
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  if (entries.some((e) => e.name === '.beav-post-id')) {
    out.push(dir);
    return out;
  }
  for (const entry of entries) {
    if (!entry.isDirectory() || entry.name.startsWith('_') || entry.name.startsWith('.')) continue;
    collect(path.join(dir, entry.name), depth + 1, out);
  }
  return out;
}

const plan = [];
let skipped = 0;
let noDate = 0;
let conflict = 0;

for (const dir of collect(root)) {
  const relative = path.relative(root, dir).split(path.sep); // 平台/日期/作者/标题
  if (relative.length < 4) {
    skipped += 1;
    continue;
  }
  const [platform, currentDate, author, title] = relative;
  let meta = {};
  try {
    meta = JSON.parse(fs.readFileSync(path.join(dir, '帖子信息.json'), 'utf8'));
  } catch {
    meta = {};
  }
  const date = publishedDate(meta.publishedAt ?? meta.published_at);
  if (!date) {
    noDate += 1;
    continue;
  }
  if (date < CUTOFF) {
    skipped += 1; // 20261001 之前的发布日：按用户要求不动
    continue;
  }
  if (date === currentDate) continue;

  const target = path.join(root, platform, date, author, title);
  if (fs.existsSync(target)) {
    conflict += 1;
    console.log(`! 目标已存在，跳过: ${path.relative(root, dir)}  ->  ${path.relative(root, target)}`);
    continue;
  }
  plan.push({ from: dir, to: target, date });
}

for (const item of plan) {
  console.log(`${path.relative(root, item.from)}\n  -> ${path.relative(root, item.to)}`);
}

let moved = 0;
if (APPLY) {
  for (const item of plan) {
    try {
      fs.mkdirSync(path.dirname(item.to), { recursive: true });
      fs.renameSync(item.from, item.to); // 同盘 rename，瞬间完成
      moved += 1;
    } catch (error) {
      console.log(`! 移动失败 ${path.relative(root, item.from)}: ${error.message}`);
    }
  }
}

if (CSV_PATH) {
  const lines = ['from,to', ...plan.map((item) => `"${item.from}","${item.to}"`)];
  fs.writeFileSync(CSV_PATH, `${lines.join('\n')}\n`, 'utf8');
  console.log(`\n映射表已写入: ${CSV_PATH}`);
}

console.log('');
console.log(
  `待移动 ${plan.length} 个 · 目标已存在跳过 ${conflict} 个 · 20261001 前/无变化 ${skipped} 个 · 无发布时间 ${noDate} 个`,
);
console.log(
  APPLY ? `已移动 ${moved} 个（同盘 rename，未复制、未删除）` : '加 --apply 才会真正移动。',
);
