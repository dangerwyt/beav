#!/usr/bin/env node
/**
 * 一次性维护脚本：清洗存量抖音标题。
 *
 * 背景：抖音卡片取 `textContent` 会拼成「置顶3.7万<正文><正文>」——
 * 开头是「置顶」徽标 + 点赞数（点赞数是独立子元素），末尾把同一段文案重复一遍。
 * 抓取侧已在 `headless_browser.rs` 的 `cleanTitle()` 修掉，本脚本用于回洗历史数据。
 *
 * 清洗规则（与 `headless_browser.rs` / `scrape_profile_posts` 内的实现保持一致）：
 *   1. 去掉前导「置顶」
 *   2. 去掉前导点赞数（强量词后不剥；弱量词仅在数字 ≤2 位时护住）
 *   3. 去掉尾部重复文案（容错公共前缀：≥12 字且 ≥75%，容忍 DOM 截断）
 *
 * ⚠️ 必须先退出 Beav，否则应用内存里的旧快照会覆盖本脚本的写入。
 *
 * 用法：
 *   node scripts/maintenance/clean-douyin-titles.mjs
 *   node scripts/maintenance/clean-douyin-titles.mjs --store "D:/path/beav-state.json" --dry-run
 */
import { readFileSync, writeFileSync, copyFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { homedir } from 'node:os';

const DEFAULT_STORE =
  process.platform === 'win32'
    ? join(process.env.APPDATA ?? join(homedir(), 'AppData', 'Roaming'), 'Beav', 'beav-state.json')
    : join(homedir(), 'Library', 'Application Support', 'Beav', 'beav-state.json');

function arg(name, fallback = null) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const storePath = arg('store', DEFAULT_STORE);
const dryRun = process.argv.includes('--dry-run');

const STRONG_UNIT = /^[个种条步招篇次位项]/;
const WEAK_UNIT = /^[月日年天分秒岁]/;
const LEAD_PIN = /^(?:置顶|置頂)\s*/;
const LEAD_NUM = /^[\d.]+\s*(?:万|w|W|k|K)?\s*/;

function commonPrefixLen(a, b) {
  const n = Math.min(a.length, b.length);
  let i = 0;
  while (i < n && a[i] === b[i]) i += 1;
  return i;
}

function dedupeTail(s) {
  for (let i = Math.ceil(s.length / 2); i < s.length; i += 1) {
    const tail = s.slice(i);
    if (tail.length < 12) break;
    const l = commonPrefixLen(s, tail);
    if (l >= 12 && l >= tail.length * 0.75) return s.slice(0, i).trim();
  }
  return s;
}

export function cleanTitle(raw) {
  let t = String(raw ?? '').replace(/\s+/g, ' ').trim();
  if (!t) return '';
  t = t.replace(LEAD_PIN, '');
  const m = t.match(LEAD_NUM);
  if (m) {
    const rest = t.slice(m[0].length).trim();
    const digitLen = m[0].replace(/[^\d]/g, '').length;
    const blocked = STRONG_UNIT.test(rest) || (WEAK_UNIT.test(rest) && digitLen <= 2);
    if (rest.length >= 6 && !blocked) t = rest;
  }
  return dedupeTail(t).trim();
}

/** 回归用例：真实脏数据 + 容易误伤的正文数字。`--self-test` 触发。 */
function selfTest() {
  const cases = [
    [
      '重复+数字前缀(完整)',
      '667大家能对出下联吗？ @赵镭老师 @哈佛JoJo #语文 #语文课 #对联 #趣味课堂 #老师大家能对出下联吗？ @赵镭老师 @哈佛JoJo #语文 #语文',
      '大家能对出下联吗？ @赵镭老师 @哈佛JoJo #语文 #语文课 #对联 #趣味课堂 #老师',
    ],
    [
      '重复+数字前缀(尾部截断)',
      '1522这题还真难不倒我 @赵镭老师 @哈佛JoJo #教师节 #语文 #文化常识 #老师 #小学生这题还真难不倒我 @赵镭老师 @哈佛JoJo #语',
      '这题还真难不倒我 @赵镭老师 @哈佛JoJo #教师节 #语文 #文化常识 #老师 #小学生',
    ],
    ['弱量词长数字(月)', '543月圆人团圆，阖家欢乐 @赵镭老师 @哈佛JoJo #中秋快乐', '月圆人团圆，阖家欢乐 @赵镭老师 @哈佛JoJo #中秋快乐'],
    ['弱量词长数字(天)', '519天赋和努力哪个更重要？ @赵镭老师', '天赋和努力哪个更重要？ @赵镭老师'],
    ['置顶+万级点赞', '置顶3.7万今天的课堂实录 #语文 #课堂', '今天的课堂实录 #语文 #课堂'],
    ['强量词保护(2位)', '10个写作技巧让你秒变高手', '10个写作技巧让你秒变高手'],
    ['强量词保护(1位)', '3个方法提升记忆力', '3个方法提升记忆力'],
    ['弱量词保护(短数字)', '3天速成语文阅读理解', '3天速成语文阅读理解'],
    ['相似但不重复', '今天讲语文课的重点 今天讲数学课的难点', '今天讲语文课的重点 今天讲数学课的难点'],
    ['空白归一', '  多余的   空白   ', '多余的 空白'],
  ];

  let failed = 0;
  for (const [name, input, expected] of cases) {
    const actual = cleanTitle(input);
    if (actual !== expected) {
      failed += 1;
      console.error(`✗ ${name}\n  期望: ${expected}\n  实际: ${actual}`);
    }
  }
  if (failed > 0) {
    console.error(`\ncleanTitle 回归失败：${failed}/${cases.length}`);
    process.exit(1);
  }
  console.log(`✓ cleanTitle 回归通过（${cases.length} 例）`);
}

function main() {
  if (process.argv.includes('--self-test')) {
    selfTest();
    return;
  }
  if (!existsSync(storePath)) {
    console.error(`找不到 store 文件：${storePath}`);
    process.exit(1);
  }
  const store = JSON.parse(readFileSync(storePath, 'utf8'));
  const posts = store.monitoredPosts ?? store.monitored_posts ?? [];

  let changed = 0;
  const samples = [];
  for (const p of posts) {
    if ((p.platform ?? '') !== 'douyin') continue;
    const oldTitle = p.title ?? '';
    const next = cleanTitle(oldTitle);
    if (next && next !== oldTitle) {
      p.title = next;
      changed += 1;
      if (samples.length < 5) samples.push([oldTitle, next]);
    }
  }

  // 分析记录里的 postTitle 同步一份，避免详情页显示旧标题
  const titleById = new Map(posts.map((p) => [p.id, p.title]));
  const analyses = store.postVideoAnalyses ?? store.post_video_analyses ?? [];
  let synced = 0;
  for (const a of analyses) {
    const t = titleById.get(a.postId ?? a.post_id);
    if (t && a.postTitle !== t) {
      a.postTitle = t;
      synced += 1;
    }
  }

  console.log(`store: ${storePath}`);
  console.log(`抖音帖: ${posts.filter((p) => (p.platform ?? '') === 'douyin').length}`);
  console.log(`标题清洗: ${changed}`);
  console.log(`分析记录同步: ${synced}`);
  for (const [oldTitle, next] of samples) {
    console.log(`  - ${oldTitle.slice(0, 46)}…`);
    console.log(`  → ${next.slice(0, 46)}…`);
  }

  if (dryRun) {
    console.log('\n--dry-run：未写入');
    return;
  }
  if (changed === 0) {
    console.log('\n无需改动');
    return;
  }
  const backup = storePath.replace(/\.json$/, `.backup-title-${Date.now()}.json`);
  copyFileSync(storePath, backup);
  writeFileSync(storePath, JSON.stringify(store, null, 2), 'utf8');
  console.log(`\n已写入；备份: ${backup}`);
}

main();
