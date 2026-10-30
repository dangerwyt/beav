#!/usr/bin/env node
/**
 * 一次性维护脚本：检测 / 合并「同一用户下重复的监控项」。
 *
 * 背景：同一个博主被建了多个监控项时，两个 watch 会轮流抓同一批帖子，
 * 而落库按 (owner, 裸URL) 去重 → 帖子只会归属其中**一个** watch，
 * 另一个 watch 的卡片永远显示「已收录 0 篇」。
 * 用户看到的就是「抓取提示抓到 47 篇，卡片却只有 5 篇」这种自相矛盾的现象。
 *
 * 抓取侧已修（`watch_add` 判重 + 落库不抢归属），本脚本处理**存量**重复项：
 * - `--dry-run`（默认）：只报告哪些是重复项，不动数据；
 * - `--merge`：把重复项的帖子迁移到保留的主项，然后删除重复项。
 *
 * ⚠️ 必须先退出 Beav，否则应用内存里的旧快照会覆盖本脚本的写入。
 *
 * 用法：
 *   node scripts/maintenance/dedupe-watch-list.mjs
 *   node scripts/maintenance/dedupe-watch-list.mjs --merge
 *   node scripts/maintenance/dedupe-watch-list.mjs --store "D:/path/beav-state.json" --dry-run
 */
import { readFileSync, writeFileSync, copyFileSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { homedir } from 'node:os';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');

const DEFAULT_STORE =
  process.platform === 'win32'
    ? join(process.env.APPDATA ?? join(homedir(), 'AppData', 'Roaming'), 'Beav', 'beav-state.json')
    : join(homedir(), 'Library', 'Application Support', 'Beav', 'beav-state.json');

function arg(name, fallback = null) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const storePath = arg('store', DEFAULT_STORE);
const dryRun = !process.argv.includes('--merge');

/** 与 Rust 侧 `watch_dedupe_key` 保持一致：owner + 平台 + 归一化主页 URL。 */
function dedupeKey(watch) {
  const owner = String(watch.owner ?? '').trim();
  const platform = String(watch.platform ?? '').trim().toLowerCase();
  const url = String(watch.profileUrl ?? watch.profile_url ?? '').trim();
  if (!url) return null;
  const bare = url.split('?')[0].replace(/\/+$/, '');
  if (!bare) return null;
  return `${owner}|${platform}|${bare.toLowerCase()}`;
}

/** 帖子的裸 URL（去掉 query）——抓取侧就是按这个去重的。 */
function barePostUrl(url) {
  const raw = String(url ?? '');
  return raw.split('?')[0];
}

function main() {
  if (!existsSync(storePath)) {
    console.error(`找不到 store 文件：${storePath}`);
    process.exit(1);
  }

  const store = JSON.parse(readFileSync(storePath, 'utf8'));
  // store 用 camelCase（serde rename_all），但老文件可能是 snake_case —— 两种都认。
  const watchKey = store.competitorWatchlist ? 'competitorWatchlist' : 'competitor_watchlist';
  const postKey = store.monitoredPosts ? 'monitoredPosts' : 'monitored_posts';
  const watches = store[watchKey] ?? [];
  const posts = store[postKey] ?? [];

  // 按判重键分组，保留每组第一个（postCount 最多的优先，留下的那条归属信息最全）
  const groups = new Map();
  for (const watch of watches) {
    const key = dedupeKey(watch);
    if (!key) continue;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(watch);
  }

  const duplicates = [...groups.entries()].filter(([, items]) => items.length > 1);
  if (duplicates.length === 0) {
    console.log(`store : ${storePath}`);
    console.log(`监控项 : ${watches.length}，未发现重复（同一用户 + 同一主页）`);
    return;
  }

  const postCountOf = (watchId) =>
    posts.filter((post) => (post.watchId ?? post.watch_id) === watchId).length;

  console.log(`store : ${storePath}`);
  console.log(`监控项 : ${watches.length}，发现 ${duplicates.length} 组重复\n`);

  const removals = new Set();
  const postMoves = [];

  for (const [key, items] of duplicates) {
    const sorted = [...items].sort((a, b) => postCountOf(b.id) - postCountOf(a.id));
    const keep = sorted[0];
    const drop = sorted.slice(1);
    console.log(`重复：${key}`);
    console.log(`  保留 ${keep.id}  ${keep.name}  (${postCountOf(keep.id)} 篇)`);
    for (const item of drop) {
      console.log(`  待删 ${item.id}  ${item.name}  (${postCountOf(item.id)} 篇)`);
      removals.add(item.id);
      // 帖子迁移到保留的主项（按裸 URL 去重，重复的帖子不迁，避免一条帖子出现两行）
      const keepUrls = new Set(
        posts
          .filter((post) => (post.watchId ?? post.watch_id) === keep.id)
          .map((post) => barePostUrl(post.postUrl ?? post.post_url)),
      );
      for (const post of posts) {
        const postWatchId = post.watchId ?? post.watch_id;
        if (postWatchId !== item.id) continue;
        const url = barePostUrl(post.postUrl ?? post.post_url);
        if (keepUrls.has(url)) continue; // 主项已有同一条帖子 → 丢弃这份重复
        keepUrls.add(url);
        postMoves.push({ post, from: item.id, to: keep.id });
      }
    }
    console.log('');
  }

  const movedPosts = postMoves.length;
  console.log(`将删除 ${removals.size} 个重复监控项，迁移 ${movedPosts} 条帖子`);

  if (dryRun) {
    console.log('\n--dry-run：未写入（加 --merge 执行实际合并）');
    return;
  }

  if (removals.size === 0) {
    console.log('\n没有需要合并的重复项');
    return;
  }

  const backup = storePath.replace(/\.json$/, `.backup-dedupe-${Date.now()}.json`);
  copyFileSync(storePath, backup);

  for (const move of postMoves) {
    if ('watchId' in move.post) move.post.watchId = move.to;
    else move.post.watch_id = move.to;
  }
  store[watchKey] = watches.filter((watch) => !removals.has(watch.id));

  writeFileSync(storePath, JSON.stringify(store, null, 2), 'utf8');
  console.log(`\n已写入；备份: ${backup}`);
}

main();
