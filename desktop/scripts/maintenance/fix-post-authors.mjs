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

/**
 * 复刻 Rust `sanitize_fs_name()` + 导出时的 32 字符截断，用来和磁盘上的作者目录名比对。
 * 不这么做的话，作者名里含非法字符或被截断时永远对不上，会反复计划移动同一个目录。
 */
function sanitizeAuthorDir(name) {
  const ILLEGAL = new Set(['\\', '/', ':', '*', '?', '"', '<', '>', '|']);
  let out = Array.from(String(name ?? ''))
    .map((c) => {
      const code = c.codePointAt(0) ?? 0;
      return ILLEGAL.has(c) || code < 0x20 || code === 0x7f ? '_' : c;
    })
    .join('');
  while (out.includes('__')) out = out.split('__').join('_');
  out = out.trim().replace(/^_+|_+$/g, '');
  return Array.from(out).slice(0, 32).join('');
}

/** 在 dir 的直接子目录里找「`.beav-post-id` 等于 postId」的那个目录。 */
function findDirOwnedBy(dir, postId) {
  let entries = [];
  try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return null; }
  for (const ent of entries) {
    if (!ent.isDirectory()) continue;
    const candidate = path.join(dir, ent.name);
    try {
      if (fs.readFileSync(path.join(candidate, MARKER), 'utf8').trim() === postId) return candidate;
    } catch { /* 该子目录没有标记文件 */ }
  }
  return null;
}

/**
 * 目标目录下的唯一落点，规则对齐 Rust 导出（`try_export_post_product`）：
 * 先试原标题目录名，被占就用 `{标题}_{postId}`；仍被占再追加序号。
 *
 * 为什么必须这样：同一作者下「标题前 10 字相同」的帖子很常见
 * （比如一堆 `【抖音独家】Open…`），导出侧本来就是靠 `_{postId}` 后缀区分的。
 * 早期脚本「目标存在就跳过」，结果这批帖子永远搬不过去。
 */
function uniqueDestDir(targetDir, baseName, postId) {
  const samePost = findDirOwnedBy(targetDir, postId);
  if (samePost) return samePost;
  const first = path.join(targetDir, baseName);
  if (!fs.existsSync(first)) return first;
  const withId = path.join(targetDir, `${baseName}_${postId}`);
  if (!fs.existsSync(withId)) return withId;
  for (let n = 2; n < 1000; n += 1) {
    const next = path.join(targetDir, `${baseName}_${postId}_${n}`);
    if (!fs.existsSync(next)) return next;
  }
  return withId;
}

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
    // **直接按「帖子 → 应有作者目录名」判定，不依赖 candidates**。
    // 早期版本从 candidates 推导待移动集合，于是出现：上一次 --apply 已经把 author 修好、
    // candidates 归零，再跑 --merge-dirs 就一个目录都不移 —— 目录永远合不上。
    const postById = new Map(posts.map((p) => [String(p.id), p]));
    const expectedAuthorDir = (post) => {
      const wid = String(post.watchId ?? '').trim();
      const watch = wid ? watchById.get(wid) : null;
      const name = String(watch?.name ?? '').trim() || String(post.author ?? '').trim();
      return sanitizeAuthorDir(name) || '未知作者';
    };
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

    let orphan = 0;
    for (const m of markers) {
      let postId = '';
      try { postId = fs.readFileSync(m.markerPath, 'utf8').trim(); } catch { continue; }
      const post = postById.get(postId);
      if (!post) { orphan += 1; continue; }
      const to = expectedAuthorDir(post);
      const titleDir = m.dir;
      const authorDir = path.dirname(titleDir);
      const dateDir = path.dirname(authorDir);
      const from = path.basename(authorDir);
      if (from === to) continue;

      const targetDir = path.join(dateDir, to);
      const titleName = path.basename(titleDir);

      // 目标作者下已经存在「同一个帖子」的目录 → 源是重复副本，不用搬。
      const own = findDirOwnedBy(targetDir, postId);
      if (own) {
        moves.push({ postId, titleDir, destDir: own, targetDir, from, to, titleName, kind: 'duplicate' });
        continue;
      }

      // 目标下同名目录被别人占了（同一作者标题前 10 字相同很常见）→
      // 复刻 Rust 导出的兜底：`{标题}_{postId}`，仍撞就再追加序号。
      const destDir = uniqueDestDir(targetDir, titleName, postId);
      moves.push({ postId, titleDir, destDir, targetDir, from, to, titleName, kind: 'move' });
    }
    if (orphan) console.log(`其中 ${orphan} 个标记对应的帖子已不在库中（跳过）`);

    const dupCount = moves.filter((m) => m.kind === 'duplicate').length;
    const renameCount = moves.filter((m) => m.kind === 'move' && path.basename(m.destDir) !== m.titleName).length;
    console.log(`\n需要处理 ${moves.length} 个帖子目录（重命名避让 ${renameCount} · 重复副本 ${dupCount}）`);
    for (const mv of moves) {
      if (mv.kind === 'duplicate') {
        console.log(`  [重复] ${mv.from}/${mv.titleName} —— 目标已有同一帖子，源目录留待手动清理`);
      } else {
        const renamed = path.basename(mv.destDir) !== mv.titleName ? '  (重命名避让)' : '';
        console.log(`  ${mv.from}/… → ${mv.to}/${path.basename(mv.destDir)}${renamed}`);
      }
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
let movedOk = 0;
let movedSkip = 0;
let movedFail = 0;
if (args.mergeDirs && moves.length) {
  const touchedAuthorDirs = new Set();
  for (const mv of moves) {
    if (mv.kind === 'duplicate') {
      movedSkip += 1;
      console.log(`  — 重复副本，未移动: ${mv.from}/${mv.titleName}`);
      continue;
    }
    try {
      fs.mkdirSync(mv.targetDir, { recursive: true });
      fs.renameSync(mv.titleDir, mv.destDir);
      touchedAuthorDirs.add(path.dirname(mv.titleDir));
      movedOk += 1;
      const renamed = path.basename(mv.destDir) !== mv.titleName ? '  (重命名避让)' : '';
      console.log(`  ✔ ${mv.from}/${mv.titleName} → ${mv.to}/${path.basename(mv.destDir)}${renamed}`);
    } catch (e) {
      movedFail += 1;
      console.log(`  ✘ 失败 ${mv.titleDir}`);
      console.log(`     原因: ${e.message}`);
    }
  }

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
  console.log(`\n移动结果：成功 ${movedOk} · 重复副本 ${movedSkip} · 失败 ${movedFail} · 清理空目录 ${removed}`);
}

/* ---------- 5. 移动后重新扫一遍，确认真的归位 ---------- */
if (args.mergeDirs && exportRoot && fs.existsSync(exportRoot)) {
  const byId2 = new Map(posts.map((p) => [String(p.id), p]));
  const expect2 = (post) => {
    const wid = String(post.watchId ?? '').trim();
    const w = wid ? watchById.get(wid) : null;
    const name = String(w?.name ?? '').trim() || String(post.author ?? '').trim();
    return sanitizeAuthorDir(name) || '未知作者';
  };
  const stillMisplaced = [];
  const walk2 = (dir, depth) => {
    if (depth > 6) return;
    let entries;
    try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
    for (const ent of entries) {
      const full = path.join(dir, ent.name);
      if (ent.isFile() && ent.name === MARKER) {
        let pid = '';
        try { pid = fs.readFileSync(full, 'utf8').trim(); } catch { continue; }
        const post = byId2.get(pid);
        if (!post) continue;
        const want = expect2(post);
        const got = path.basename(path.dirname(dir));
        if (got !== want) stillMisplaced.push(`${got}/ → ${want}/${path.basename(dir)}`);
      } else if (ent.isDirectory()) walk2(full, depth + 1);
    }
  };
  walk2(exportRoot, 0);
  console.log(`
=== 复核 === 仍错位 ${stillMisplaced.length} 个`);
  stillMisplaced.slice(0, 5).forEach((s) => console.log(`  ${s}`));
  if (stillMisplaced.length > 5) console.log(`  … 其余 ${stillMisplaced.length - 5} 个省略`);
}

/* ---------- 6. 回读校验（store 内 author 是否与 watch.name 一致）---------- */
const check = JSON.parse(fs.readFileSync(args.store, 'utf8'));
const stillBad = (check.monitoredPosts ?? []).filter((p) => {
  const wid = String(p.watchId ?? '').trim();
  if (!wid) return false;
  const w = watchById.get(wid);
  return w && String(w.name ?? '').trim() && String(p.author ?? '').trim() !== String(w.name).trim();
});
console.log(`
=== 回读校验 === 仍有不一致: ${stillBad.length} 条`);
