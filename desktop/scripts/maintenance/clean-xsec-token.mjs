#!/usr/bin/env node
/**
 * 一次性维护脚本：清理存量数据里的会话绑定 token（xsec_token / xsec_source）。
 *
 * 背景：小红书的 `xsec_token` 是**与浏览器会话绑定**的防盗链令牌。
 * 用户在自己电脑上复制的主页链接带的是本机 token，存进库后：
 * - 抓主页时用服务器的登录态访问，带着别人的 token 可能被判无效；
 * - 点「原文」时，用户本地浏览器也未必认这个 token（除非登录的是同一账号）。
 *
 * 抓取侧已修（`strip_modal_id_param` 剥离 + watch_add 存库归一化），
 * 本脚本用于把**存量**数据清干净：
 * - `competitorWatchlist[].profileUrl` → 剥掉 xsec_token / xsec_source / modal_id
 *
 * ⚠️ **不动 `monitoredPosts[].postUrl`**：笔记详情抓取是直接拿库里存的 URL 导航的
 *    （`scrape_post_detail_once` 不补 token），那里的 token 是服务器自己登录态下
 *    抓到的有效 token，剥掉会导致老帖正文抓取直接 404。
 *
 * ⚠️ 必须先退出 Beav，否则应用内存里的旧快照会覆盖本脚本的写入。
 *
 * 用法：
 *   node scripts/maintenance/clean-xsec-token.mjs
 *   node scripts/maintenance/clean-xsec-token.mjs --store "D:/path/beav-state.json" --dry-run
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

const STRIP_PARAMS = new Set(['xsec_token', 'xsec_source', 'modal_id']);

function arg(name, fallback = null) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

/** 与 Rust 侧 `strip_modal_id_param` 保持一致。 */
function stripSessionParams(url) {
  if (typeof url !== 'string' || !url) return url;
  const at = url.indexOf('?');
  if (at < 0) return url;
  const base = url.slice(0, at);
  const kept = url
    .slice(at + 1)
    .split('&')
    .filter(Boolean)
    .filter((part) => !STRIP_PARAMS.has(part.split('=')[0]));
  return kept.length ? `${base}?${kept.join('&')}` : base;
}

const storePath = arg('store', DEFAULT_STORE);
const dryRun = process.argv.includes('--dry-run');

function main() {
  if (!existsSync(storePath)) {
    console.error(`找不到 store 文件：${storePath}`);
    process.exit(1);
  }

  const store = JSON.parse(readFileSync(storePath, 'utf8'));
  const watches = store.competitorWatchlist ?? store.competitor_watchlist ?? [];

  let watchChanged = 0;

  for (const watch of watches) {
    const before = watch.profileUrl ?? watch.profile_url;
    if (!before) continue;
    const after = stripSessionParams(before);
    if (after !== before) {
      if ('profileUrl' in watch) watch.profileUrl = after;
      else watch.profile_url = after;
      watchChanged += 1;
    }
  }

  console.log(`store        : ${storePath}`);
  console.log(`博主主页 URL : 清理 ${watchChanged} / ${watches.length}`);
  console.log(`帖子 URL     : 不处理（详情抓取依赖其中的服务器有效 token）`);

  if (dryRun) {
    console.log('\n--dry-run：未写入');
    return;
  }
  if (watchChanged === 0) {
    console.log('\n没有需要清理的链接');
    return;
  }
  const backup = storePath.replace(/\.json$/, `.backup-xsec-${Date.now()}.json`);
  copyFileSync(storePath, backup);
  writeFileSync(storePath, JSON.stringify(store, null, 2), 'utf8');
  console.log(`\n已写入；备份: ${backup}`);
}

main();
