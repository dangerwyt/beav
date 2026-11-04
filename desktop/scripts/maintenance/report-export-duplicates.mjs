/**
 * 体检：看看导出库里有多少「同一篇帖子被导出成多个目录」的重复素材。
 *
 * 只读，不改任何文件。想看明细就加 --all，想拿到表格就加 --csv。
 *
 * 为什么会有重复：导出目录的日期是**导出当天**，而同一篇帖子会在多个时机自动导出
 * （转写重试成功、重新下载、重新抽帧、重新分析、图文详情抓取），跨天再导一次就落到
 * 另一个 `日期/作者` 分支。旧版去重只在同一日期目录里按 `.beav-post-id` 查找，
 * 于是又建一份。导出侧已改为横向复用，本脚本用于盘清历史遗留。
 *
 * 用法：
 *   node scripts/maintenance/report-export-duplicates.mjs
 *   node scripts/maintenance/report-export-duplicates.mjs --top 50
 *   node scripts/maintenance/report-export-duplicates.mjs --all
 *   node scripts/maintenance/report-export-duplicates.mjs --csv /tmp/dups.csv
 *
 * 参数：
 *   --root PATH   导出根（默认读 store 的 monitorExportRoot / BEAV_EXPORT_ROOT）
 *   --store PATH  store 路径（默认自动探测）
 *   --top N       明细显示前 N 组（默认 20）
 *   --all         明细全量输出
 *   --csv PATH    把全量清单写成 CSV
 *
 * 清理请用同目录的 dedupe-export-dirs.mjs（默认 dry-run，--apply 才移动到备份目录）。
 */
import fs from 'node:fs';
import path from 'node:path';
import {
  readStore,
  resolveScanRoots,
  groupByPost,
  scoreDir,
  dirSize,
  platformOf,
  formatBytes,
} from './_export-dirs.mjs';

function arg(name, fallback = '') {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
const hasFlag = (name) => process.argv.includes(`--${name}`);

const TOP = hasFlag('all') ? 0 : Number(arg('top', '20')) || 20;
const CSV_PATH = arg('csv', '');

let ctx;
try {
  ctx = readStore(arg('store'));
} catch (error) {
  console.error(String(error.message ?? error));
  process.exit(1);
}
console.log(`store: ${ctx.label}`);

let scan;
try {
  scan = resolveScanRoots(ctx.store, arg('root'));
} catch (error) {
  console.error(String(error.message ?? error));
  process.exit(1);
}
if (scan.roots.length > 1) {
  console.log(`导出根: ${scan.exportRoot}`);
  console.log(`附带扫描: ${scan.roots[1]}（Obsidian 卡片镜像树）`);
} else {
  console.log(`导出根: ${scan.exportRoot}`);
}

const csvRows = [];
let totalDirs = 0;
let totalPosts = 0;
let totalDupGroups = 0;
let totalRedundantDirs = 0;
let totalRedundantBytes = 0;

for (const root of scan.roots) {
  const { all, byPost, groups } = groupByPost(root);
  const redundantDirs = groups.reduce((n, [, dirs]) => n + dirs.length - 1, 0);
  let redundantBytes = 0;
  const platforms = new Map();
  const detail = [];

  for (const [postId, dirs] of groups) {
    const ranked = dirs
      .map((dir) => ({ dir, ...scoreDir(dir), bytes: dirSize(dir) }))
      .sort((a, b) => b.total - a.total || b.mtime - a.mtime);
    const keeper = ranked[0];
    const losers = ranked.slice(1);
    const groupBytes = losers.reduce((n, item) => n + item.bytes, 0);
    redundantBytes += groupBytes;

    const platform = platformOf(root, keeper.dir);
    platforms.set(platform, (platforms.get(platform) ?? 0) + 1);

    detail.push({ postId, platform, keeper, losers, groupBytes });
    for (const item of ranked) {
      csvRows.push({
        root,
        platform,
        postId,
        role: item.dir === keeper.dir ? 'keep' : 'redundant',
        files: item.files.length,
        bytes: item.bytes,
        dir: item.dir,
      });
    }
  }

  console.log(`\n=== ${root}`);
  console.log(
    `产物目录 ${all.length} 个 · 帖子 ${byPost.size} 篇 · 重复 ${groups.length} 篇` +
      `（冗余目录 ${redundantDirs} 个，约 ${formatBytes(redundantBytes)}）`,
  );
  console.log(`平台分布: ${[...platforms.entries()].map(([k, v]) => `${k} ${v} 篇`).join(' / ') || '—'}`);

  if (groups.length) {
    detail.sort((a, b) => b.groupBytes - a.groupBytes);
    const shown = TOP > 0 ? detail.slice(0, TOP) : detail;
    console.log(`\n重复明细（按冗余体积降序${TOP > 0 && detail.length > TOP ? `，仅显示前 ${TOP} 组` : ''}）`);
    shown.forEach((item, index) => {
      console.log(`${index + 1}. ${item.postId}  ${item.platform}`);
      console.log(
        `   保留 ${path.relative(root, item.keeper.dir)}（${item.keeper.files.length} 文件, ${formatBytes(item.keeper.bytes)}）`,
      );
      for (const loser of item.losers) {
        console.log(
          `   冗余 ${path.relative(root, loser.dir)}（${loser.files.length} 文件, ${formatBytes(loser.bytes)}）`,
        );
      }
    });
    if (TOP > 0 && detail.length > TOP) {
      console.log(`   …… 还有 ${detail.length - TOP} 组，加 --all 看全量`);
    }
  }

  totalDirs += all.length;
  totalPosts += byPost.size;
  totalDupGroups += groups.length;
  totalRedundantDirs += redundantDirs;
  totalRedundantBytes += redundantBytes;
}

if (scan.roots.length > 1) {
  console.log('\n—— 汇总 ——');
  console.log(
    `产物目录 ${totalDirs} 个 · 帖子 ${totalPosts} 篇 · 重复 ${totalDupGroups} 篇 · ` +
      `冗余 ${totalRedundantDirs} 个目录（约 ${formatBytes(totalRedundantBytes)}）`,
  );
}

if (CSV_PATH) {
  const header = 'root,platform,postId,role,files,bytes,dir';
  const escape = (value) => `"${String(value).replace(/"/g, '""')}"`;
  const lines = [header, ...csvRows.map((row) => [
    row.root,
    row.platform,
    row.postId,
    row.role,
    row.files,
    row.bytes,
    row.dir,
  ].map(escape).join(','))];
  fs.writeFileSync(CSV_PATH, `${lines.join('\n')}\n`, 'utf8');
  console.log(`\nCSV 已写入: ${CSV_PATH}（${csvRows.length} 行）`);
}

console.log(
  totalDupGroups > 0
    ? '\n清理：node scripts/maintenance/dedupe-export-dirs.mjs            # 先预演\n      node scripts/maintenance/dedupe-export-dirs.mjs --apply    # 移到 _dedupe-backup-*'
    : '\n没有发现重复，导出库是干净的。',
);
