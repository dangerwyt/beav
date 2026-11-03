/**
 * 修复被详情页误抓污染的帖子作者名，并可合并因此裂开的导出目录。
 *
 *   node scripts/maintenance/fix-post-authors.mjs --dry-run
 *   node scripts/maintenance/fix-post-authors.mjs --apply
 *   node scripts/maintenance/fix-post-authors.mjs --apply --merge-dirs
 *   node scripts/maintenance/fix-post-authors.mjs --apply --export-root "D:/beav_workspace/导出"
 *
 * ⚠️ 必须先退出应用，否则内存里的旧快照会覆盖本脚本的写入。
 *
 * 背景：`post_detail` 曾经无条件用详情页的作者覆盖记录里的作者，而详情页的兜底链会把
 * 正文里的 `@提及`（如「@抖音科技」「@抖音精选 App官方账号」）或推荐位的其他作者当成作者，
 * 于是同一个博主在导出目录里裂成多个作者文件夹。
 * 代码侧已在 `detail_author_may_override()` 里堵住新增；本脚本负责修存量。
 *
 * 判定规则：帖子有 watch_id 且该 watch 仍存在 → 作者应为 watch.name（列表抓取阶段写入的权威值）。
 */
import fs from 'node:fs';
import path from 'node:path';
import { resolveStorePath } from './_store-path.mjs';

const MARKER = '.beav-post-id';

function parseArgs(argv) {
  const out = { apply: false, dryRun: false, mergeDirs: false, store: '', exportRoot: '' };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === '--apply') out.apply = true;
    else if (a === '--dry-run') out.dryRun = true;
    else if (a === '--merge-dirs') out.mergeDirs = true;
    else if (a === '--store') out.store = argv[++i];
    else if (a === '--export-root') out.exportRoot = argv[++i];
  }
  return out;
}

const args = parseArgs(process.argv.slice(2));
if (!args.apply && !args.dryRun) {
  console.error('用法: node scripts/maintenance/fix-post-authors.mjs --dry-run | --apply [--merge-dirs]');
  process.exit(1);
}

const resolved = resolveStorePath(args.store);
args.store = resolved.file;
if (!fs.existsSync(args.store)) {
  console.error(`store 不存在: ${args.store}（来源: ${resolved.from}）`);
  console.error('若数据在别处，用 --store <路径> 指定，或设置 BEAV_STORE_DIR。');
  process.exit(1);
}
const stat = fs.statSync(args.store);
console.log(`store: ${args.store}`);
console.log(`      来源=${resolved.from} · ${(stat.size / 1024 / 1024).toFixed(2)} MB · 修改于 ${stat.mtime.toISOString()}`);
console.log('');

const store = JSON.parse(fs.readFileSync(args.store, 'utf8'));
const posts = Array.isArray(store.monitoredPosts) ? store.monitoredPosts : [];
const watches = Array.isArray(store.competitorWatchlist) ? store.competitorWatchlist : [];
const watchById = new Map(watches.map((w) => [w.id, w]));

console.log(`帖子 ${posts.length} 条 · 监控项 ${watches.length} 个`);

/* ---------- 1. 找出作者被污染的帖子 ---------- */
const candidates = [];
const skipped = { noWatchField: 0, watchGone: 0, emptyWatchName: 0 };

for (const post of posts) {
  const watchId = String(post.watchId ?? '').trim();
  if (!watchId) { skipped.noWatchField += 1; continue; }
  const watch = watchById.get(watchId);
  if (!watch) { skipped.watchGone += 1; continue; }
  const target = String(watch.name ?? '').trim();
  if (!target) { skipped.emptyWatchName += 1; continue; }
  const current = String(post.author ?? '').trim();
  if (current === target) continue;
  candidates.push({
    postId: post.id,
    postUrl: post.postUrl,
    platform: post.platform,
    from: current || '(空)',
    to: target,
    watchId,
  });
}

console.log(`\n=== 待修作者 ${candidates.length} 条 ===`);
console.log(`跳过：无 watch_id ${skipped.noWatchField} · watch 已删 ${skipped.watchGone} · watch 名为空 ${skipped.emptyWatchName}`);

if (candidates.length) {
  const groups = new Map();
  for (const c of candidates) {
    const key = `${c.from}  →  ${c.to}`;
    groups.set(key, (groups.get(key) ?? 0) + 1);
  }
  console.log('\n按 (旧 → 新) 汇总：');
  for (const [key, n] of [...groups.entries()].sort((a, b) => b[1] - a[1])) {
    console.log(`  ${String(n).padStart(4)} 条   ${key}`);
  }
  console.log('\n明细（最多 30 条）：');
  for (const c of candidates.slice(0, 30)) {
    console.log(`  ${String(c.platform).padEnd(12)} ${JSON.stringify(c.from)} → ${JSON.stringify(c.to)}`);
    console.log(`  ${''.padEnd(12)} ${c.postUrl}`);
  }
  if (candidates.length > 30) console.log(`  … 其余 ${candidates.length - 30} 条省略`);
}

/* ---------- 2. 合并裂开的导出目录 ---------- */
let moves = [];

/**
 * 导出根解析，顺序对齐 Rust 的 `resolve_export_target`：
 * `--export-root` → `BEAV_EXPORT_ROOT` → `settings.monitor_export_root` → `settings.workspace_dir`。
 * 最后那条是兜底工作区（未单独设置导出根时，产物就落在工作区里）。
 */
function resolveExportRoot() {
  const explicit = String(args.exportRoot ?? '').trim();
  if (explicit) return { dir: explicit, from: '--export-root' };
  const envRoot = String(process.env.BEAV_EXPORT_ROOT ?? '').trim();
  if (envRoot) return { dir: envRoot, from: 'BEAV_EXPORT_ROOT' };
  const settings = store.settings ?? {};
  const configured = String(store.monitorExportRoot ?? '').trim();
  if (configured) return { dir: configured, from: 'store.monitorExportRoot' };
  const workspace = String(settings.workspace_dir ?? '').trim();
  if (workspace) return { dir: workspace, from: 'settings.workspace_dir' };
  return { dir: '', from: '' };
}

const exportResolved = resolveExportRoot();
let exportRoot = exportResolved.dir;

if (args.mergeDirs) {
  if (!exportRoot) {
    console.log('\n[merge-dirs] 未找到导出根，跳过目录合并');
    console.log('  候选都为空：--export-root / BEAV_EXPORT_ROOT / store.monitorExportRoot / settings.workspace_dir');
    console.log('  可到应用「设置 → 通用」看当前工作目录，再用 --export-root 指定。');
  } else if (!fs.existsSync(exportRoot)) {
    console.log(`\n[merge-dirs] 导出根不存在（来源: ${exportResolved.from}），跳过: ${exportRoot}`);
  } else {
    console.log(`\n=== 扫描导出目录 ===\n根: ${exportRoot}（来源: ${exportResolved.from}）`);
    // 用 postId → 目标作者 建立映射，避免依赖平台中文名 / 日期格式
    const todoByPostId = new Map(candidates.map((c) => [c.postId, c]));
    const markers = [];
    const walk = (dir, depth) => {
      if (depth > 6) return;
      let entries;
      try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
      for (const ent of entries) {
        const full = path.join(dir, ent.name);
        if (ent.isFile() && ent.name === MARKER) markers.push({ markerPath: full, dir });
        else if (ent.isDirectory()) walk(full, depth + 1);
      }
    };
    walk(exportRoot, 0);
    console.log(`发现 ${markers.length} 个 ${MARKER} 标记`);

    for (const m of markers) {
      let postId = '';
      try { postId = fs.readFileSync(m.markerPath, 'utf8').trim(); } catch { continue; }
      const todo = todoByPostId.get(postId);
      if (!todo) continue;
      const titleDir = m.dir;
      const authorDir = path.dirname(titleDir);
      const dateDir = path.dirname(authorDir);
      if (path.basename(authorDir) === todo.to) continue;

      const targetDir = path.join(dateDir, todo.to);
      const destDir = path.join(targetDir, path.basename(titleDir));
      moves.push({ postId, titleDir, destDir, targetDir, from: path.basename(authorDir), to: todo.to, conflict: fs.existsSync(destDir) });
    }

    console.log(`\n需要移动 ${moves.length} 个帖子目录`);
    for (const mv of moves) {
      console.log(`  ${mv.conflict ? '[冲突跳过] ' : ''}${mv.from}/… → ${mv.to}/${path.basename(mv.titleDir)}`);
    }
  }
}

/* ---------- 3. 写入 ---------- */
if (args.dryRun) {
  console.log('\n[dry-run] 未写入任何文件');
  process.exit(0);
}

if (!candidates.length && !moves.length) {
  console.log('\n无需改动');
  process.exit(0);
}

const stamp = new Date().toISOString().replace(/[-:T]/g, '').slice(0, 15);
const backup = `${args.store}.bak-${stamp}`;
fs.copyFileSync(args.store, backup);
console.log(`\n已备份 store -> ${backup}`);

const byId = new Map(posts.map((p) => [p.id, p]));
let fixed = 0;
for (const c of candidates) {
  const post = byId.get(c.postId);
  if (!post) continue;
  post.author = c.to;
  fixed += 1;
}
fs.writeFileSync(args.store, `${JSON.stringify(store, null, 2)}\n`, 'utf8');
console.log(`已修正 ${fixed} 条帖子的 author`);

/* ---------- 4. 真正移动目录 ---------- */
if (args.mergeDirs && moves.length) {
  let moved = 0;
  const touchedAuthorDirs = new Set();
  for (const mv of moves) {
    if (mv.conflict) {
      console.log(`  跳过（目标已存在）: ${mv.destDir}`);
      continue;
    }
    try {
      fs.mkdirSync(mv.targetDir, { recursive: true });
      fs.renameSync(mv.titleDir, mv.destDir);
      touchedAuthorDirs.add(path.dirname(mv.titleDir));
      moved += 1;
    } catch (e) {
      console.log(`  移动失败 ${mv.titleDir}: ${e.message}`);
    }
  }
  console.log(`已移动 ${moved} 个帖子目录`);

  // 清掉搬空的旧作者目录（只删空目录，且不碰导出根本身）
  let removed = 0;
  for (const dir of touchedAuthorDirs) {
    try {
      if (fs.readdirSync(dir).length === 0 && dir !== exportRoot) {
        fs.rmdirSync(dir);
        removed += 1;
      }
    } catch { /* 忽略 */ }
  }
  console.log(`已清理 ${removed} 个空作者目录`);
}

/* ---------- 5. 回读校验 ---------- */
const check = JSON.parse(fs.readFileSync(args.store, 'utf8'));
const stillBad = (check.monitored_posts ?? []).filter((p) => {
  const wid = String(p.watch_id ?? '').trim();
  if (!wid) return false;
  const w = watchById.get(wid);
  return w && String(w.name ?? '').trim() && String(p.author ?? '').trim() !== String(w.name).trim();
});
console.log(`\n=== 回读校验 ===\n仍有不一致: ${stillBad.length} 条`);
