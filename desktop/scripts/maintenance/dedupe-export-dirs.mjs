/**
 * 合并「同一篇帖子被导出成多个目录」的重复素材。
 *
 * 背景：导出目录里的日期是**导出当天**（`export_date_yyyymmdd` 恒返回今天），
 * 而同一篇帖子会在多个时机被自动导出：转写重试成功、重新下载、重新抽帧、
 * 重新分析、图文帖详情抓取，都会调 `export_post_product`。跨天再导一次就会落到
 * `日期/作者` 的另一条分支，而旧版去重只在**同一个日期目录**里按 `.beav-post-id`
 * 查找，于是又建一份 —— 同一篇帖子出现两个目录，各自只有一半文件。
 *
 * 导出侧已改为横向复用（`find_export_dir_for_post`），本脚本清理**历史遗留**。
 * 想看清单不动手，用 report-export-duplicates.mjs。
 *
 * 规则：
 *   · 按 `.beav-post-id` 分组，保留「文件更全」的那份（视频 +1000 / 转写稿 +500 /
 *     正文 +100 / 抽帧 +50），同分时取修改时间更新的；
 *   · 保留目录缺的文件先从多余目录复制过去（不覆盖已有文件）；
 *   · `--apply` 时把多余目录整体移到 `<root>/_dedupe-backup-<时间戳>/`（保留原相对
 *     路径，可原样搬回），**不删除**。
 *
 * 用法：
 *   node scripts/maintenance/dedupe-export-dirs.mjs              # 预演
 *   node scripts/maintenance/dedupe-export-dirs.mjs --limit 20    # 只处理前 20 组
 *   node scripts/maintenance/dedupe-export-dirs.mjs --apply       # 真正执行
 *
 * 参数：
 *   --root PATH   导出根（默认读 store 的 monitorExportRoot / BEAV_EXPORT_ROOT）
 *   --store PATH  store 路径（默认自动探测）
 *   --apply       真正执行（默认 dry-run）
 *   --limit N     每棵树最多处理多少组重复（默认不限）
 */
import fs from 'node:fs';
import path from 'node:path';
import {
  readStore,
  resolveScanRoots,
  groupByPost,
  scoreDir,
  listFiles,
  BACKUP_PREFIX,
  POST_ID_MARKER,
} from './_export-dirs.mjs';

function arg(name, fallback = '') {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
const hasFlag = (name) => process.argv.includes(`--${name}`);

const APPLY = hasFlag('apply');
const LIMIT = Number(arg('limit', '0')) || 0;

let ctx;
let scan;
try {
  ctx = readStore(arg('store'));
  scan = resolveScanRoots(ctx.store, arg('root'));
} catch (error) {
  console.error(String(error.message ?? error));
  process.exit(1);
}

console.log(`store: ${ctx.label}`);
console.log(`导出根: ${scan.exportRoot}`);
if (scan.roots.length > 1) console.log(`附带扫描: ${scan.roots[1]}（Obsidian 卡片镜像树）`);
console.log(APPLY ? '模式: APPLY（会移动重复目录到备份）' : '模式: dry-run（只看不动）');

const stamp = new Date().toISOString().replace(/[-:T]/g, '').slice(0, 14);
let grandCopied = 0;
let grandMoved = 0;
let grandDuplicates = 0;

for (const root of scan.roots) {
  const { all, byPost, groups } = groupByPost(root);
  console.log(
    `\n=== ${root}\n产物目录 ${all.length} 个 · 帖子 ${byPost.size} 篇 · 重复 ${groups.length} 篇`,
  );
  if (!groups.length) continue;

  const selected = LIMIT > 0 ? groups.slice(0, LIMIT) : groups;
  const backupRoot = path.join(root, `${BACKUP_PREFIX}${stamp}`);
  let copiedFiles = 0;
  let movedDirs = 0;

  for (const [postId, dirs] of selected) {
    const ranked = dirs
      .map((dir) => ({ dir, ...scoreDir(dir) }))
      .sort((a, b) => b.total - a.total || b.mtime - a.mtime);
    const keeper = ranked[0];
    console.log(`\n· ${postId}`);
    console.log(`  保留: ${path.relative(root, keeper.dir)}（${keeper.total} 分）`);

    for (const loser of ranked.slice(1)) {
      console.log(`  重复: ${path.relative(root, loser.dir)}（${loser.total} 分）`);
      // 先把保留目录缺的文件补进去，避免搬走就永久丢内容
      for (const name of listFiles(loser.dir)) {
        if (name === POST_ID_MARKER) continue;
        const dst = path.join(keeper.dir, name);
        if (fs.existsSync(dst)) continue;
        console.log(`    + 补文件 ${name}`);
        copiedFiles += 1;
        if (APPLY) {
          try {
            fs.copyFileSync(path.join(loser.dir, name), dst);
          } catch (error) {
            console.log(`    ! 复制失败 ${name}: ${error.message}`);
          }
        }
      }
      if (APPLY) {
        const target = path.join(backupRoot, path.relative(root, loser.dir));
        fs.mkdirSync(path.dirname(target), { recursive: true });
        try {
          fs.renameSync(loser.dir, target);
          movedDirs += 1;
        } catch (error) {
          console.log(`    ! 移动失败: ${error.message}`);
        }
      }
    }
  }

  grandCopied += copiedFiles;
  grandMoved += movedDirs;
  grandDuplicates += selected.reduce((n, [, dirs]) => n + dirs.length - 1, 0);
  if (APPLY) console.log(`\n备份位置: ${backupRoot}`);
}

console.log('');
if (APPLY) {
  console.log(`完成：补文件 ${grandCopied} 个，移动重复目录 ${grandMoved} 个`);
  console.log('确认无误后可自行删除 _dedupe-backup-*；需要恢复就把里面的目录原样搬回去。');
} else {
  console.log(`预演：拟补文件 ${grandCopied} 个，拟移动重复目录 ${grandDuplicates} 个`);
  console.log('加 --apply 才会真正执行（移动到 _dedupe-backup-* 备份目录，不删除）。');
}
