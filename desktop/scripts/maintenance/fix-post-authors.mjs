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
/** 复刻 Rust `sanitize_fs_name()`：非法字符/控制字符换 `_`，压缩连续下划线，trim 首尾下划线。不截断。 */
function sanitizeFsName(name) {
  const ILLEGAL = new Set(['\\', '/', ':', '*', '?', '"', '<', '>', '|']);
  let out = Array.from(String(name ?? ''))
    .map((c) => {
      const code = c.codePointAt(0) ?? 0;
      return ILLEGAL.has(c) || code < 0x20 || code === 0x7f ? '_' : c;
    })
    .join('');
  while (out.includes('__')) out = out.split('__').join('_');
  return out.trim().replace(/^_+|_+$/g, '');
}

function sanitizeAuthorDir(name) {
  // 导出侧的作者目录名会截断到 32 字符（见 monitor.rs 的导出路径构造）
  return Array.from(sanitizeFsName(name)).slice(0, 32).join('');
}

/** 文件夹重命名取标题前多少个字符：中文 3 字节/字，40 字约 120 字节，远低于单段 255 字节上限。 */
const RENAME_TITLE_MAX = 40;

/** 从产物目录里的 `帖子信息.json` 读完整标题（导出时写的，含原始 title，不截断）。 */
function readExportedTitle(titleDir) {
  try {
    const meta = JSON.parse(fs.readFileSync(path.join(titleDir, '帖子信息.json'), 'utf8'));
    return String(meta?.title ?? '').trim();
  } catch {
    return '';
  }
}

/** 由标题算新的目录名；空标题回退「未命名」。 */
function dirNameFromTitle(title) {
  const safe = sanitizeFsName(title);
  if (!safe) return '未命名';
  return Array.from(safe).slice(0, RENAME_TITLE_MAX).join('');
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

/**
 * 同级目录内的唯一名字（用于重命名，**不按 postId 找已有目录**）。
 *
 * 不能复用 uniqueDestDir：它优先返回「已存在且属于同一 postId 的目录」，
 * 而重命名场景里那个目录正是源目录自己，会算成「改到原名」这种空操作。
 *
 * 大小写按小写比较：macOS/Windows 默认大小写不敏感，只差大小写也算重名。
 */
function uniqueNameInDir(parent, want, currentName) {
  if (want === currentName) return path.join(parent, currentName);
  let entries = [];
  try { entries = fs.readdirSync(parent); } catch { /* 目录不可读时按不冲突处理 */ }
  const taken = new Set(entries.map((n) => n.toLowerCase()));
  if (!taken.has(want.toLowerCase())) return path.join(parent, want);
  for (let n = 2; n < 1000; n += 1) {
    const candidate = `${want}_${n}`;
    if (!taken.has(candidate.toLowerCase())) return path.join(parent, candidate);
  }
  return path.join(parent, `${want}_${Date.now()}`);
}

/**
 * 强制加 `_{postId}` 后缀的唯一落点（忽略原标题名能否直接用）。
 * 用于 rename 报 ENOTEMPTY/EEXIST 后的重试。
 */
function forcedUniqueDir(targetDir, baseName, postId) {
  for (let n = 1; n < 1000; n += 1) {
    const name = n === 1 ? `${baseName}_${postId}` : `${baseName}_${postId}_${n}`;
    const candidate = path.join(targetDir, name);
    if (!fs.existsSync(candidate)) return candidate;
  }
  return path.join(targetDir, `${baseName}_${postId}_overflow`);
}

const MARKER = '.beav-post-id';

function parseArgs(argv) {
  const out = { apply: false, dryRun: false, mergeDirs: false, renameDirs: false, store: '', exportRoot: '' };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === '--apply') out.apply = true;
    else if (a === '--dry-run') out.dryRun = true;
    else if (a === '--merge-dirs') out.mergeDirs = true;
    else if (a === '--rename-dirs') out.renameDirs = true;
    else if (a === '--store') out.store = argv[++i];
    else if (a === '--export-root') out.exportRoot = argv[++i];
  }
  return out;
}

const args = parseArgs(process.argv.slice(2));
if (!args.apply && !args.dryRun) {
  console.error(
    '用法: node scripts/maintenance/fix-post-authors.mjs --dry-run|--apply [--merge-dirs] [--rename-dirs]',
  );
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

/* ---------- 2.5 按 帖子信息.json 的 title 重命名产物目录（计划）---------- */
let renames = [];
const vaultRoot = String(store.monitorObsidianVault ?? '').trim();
const vaultUsable = Boolean(vaultRoot) && fs.existsSync(vaultRoot);

if (args.renameDirs) {
  if (!exportRoot || !fs.existsSync(exportRoot)) {
    console.log('\n[rename-dirs] 未找到导出根，跳过重命名');
  } else {
    console.log(`\n=== 扫描待重命名目录 ===\n根: ${exportRoot}`);
    if (vaultUsable) {
      console.log(`Obsidian 库: ${vaultRoot} —— hub 笔记会同步改名并替换路径`);
    } else {
      console.log('未配置有效 Obsidian 库，只重命名产物目录（knowledge.md 用的是裸文件名，不受影响）');
    }

    const renameTargets = [];
    const walkRename = (dir, depth) => {
      if (depth > 6) return;
      let entries;
      try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
      for (const ent of entries) {
        const full = path.join(dir, ent.name);
        if (ent.isFile() && ent.name === MARKER) renameTargets.push(dir);
        else if (ent.isDirectory()) walkRename(full, depth + 1);
      }
    };
    walkRename(exportRoot, 0);

    let noMeta = 0;
    let alreadySame = 0;
    for (const dir of renameTargets) {
      const title = readExportedTitle(dir);
      if (!title) { noMeta += 1; continue; }
      const want = dirNameFromTitle(title);
      const cur = path.basename(dir);
      if (want === cur) { alreadySame += 1; continue; }
      let postId = '';
      try { postId = fs.readFileSync(path.join(dir, MARKER), 'utf8').trim(); } catch { /* 忽略 */ }
      const parent = path.dirname(dir);
      const dest = uniqueNameInDir(parent, want, cur);
      renames.push({ titleDir: dir, dest, parent, want, cur, postId });
    }

    if (noMeta) console.log(`  跳过 ${noMeta} 个没有 帖子信息.json 的目录`);
    if (alreadySame) console.log(`  已经是目标名 ${alreadySame} 个`);
    console.log(`\n需要重命名 ${renames.length} 个目录`);
    for (const r of renames.slice(0, 20)) {
      console.log(`  ${r.cur}`);
      console.log(`    → ${path.basename(r.dest)}`);
    }
    if (renames.length > 20) console.log(`  … 其余 ${renames.length - 20} 个省略`);
  }
}

/* ---------- 3. 写入 ---------- */
if (args.dryRun) {
  console.log('\n[dry-run] 未写入任何文件');
  process.exit(0);
}

if (!candidates.length && !moves.length && !renames.length) {
  console.log('\n无需改动');
  process.exit(0);
}

const byId = new Map(posts.map((p) => [p.id, p]));
let fixed = 0;
for (const c of candidates) {
  const post = byId.get(c.postId);
  if (!post) continue;
  post.author = c.to;
  fixed += 1;
}

// 只有真的改了 author 才写盘。`--merge-dirs` / `--rename-dirs` 只动磁盘时，
// 写 store 是纯风险：桌面端在跑的话，它的内存快照可能把这次写入覆盖回去，
// 徒增「改了但没生效」的困惑。
if (fixed > 0) {
  const stamp = new Date().toISOString().replace(/[-:T]/g, '').slice(0, 15);
  const backup = `${args.store}.bak-${stamp}`;
  fs.copyFileSync(args.store, backup);
  fs.writeFileSync(args.store, `${JSON.stringify(store, null, 2)}\n`, 'utf8');
  console.log(`\n已备份 store -> ${backup}`);
  console.log(`已修正 ${fixed} 条帖子的 author`);
} else {
  console.log('\nauthor 无需修正，跳过写 store（正在运行的应用不受影响）');
}

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
      // **执行时再算一次唯一落点**：规划是整批算的，而同一作者下「标题前 10 字相同」
      // 的帖子很多（`【抖音独家】Anth…`、`Clau…`、`Deep…`），规划时这些目标目录都还
      // 不存在，于是全算成同一个名字；执行到第二条就 ENOTEMPTY。
      const dest = uniqueDestDir(mv.targetDir, mv.titleName, mv.postId);
      if (fs.existsSync(dest)) {
        // uniqueDestDir 只有在「目标已有同一帖子」时才会返回已存在路径
        movedSkip += 1;
        console.log(`  — 重复副本，跳过: ${mv.from}/${mv.titleName}`);
        continue;
      }
      fs.renameSync(mv.titleDir, dest);
      touchedAuthorDirs.add(path.dirname(mv.titleDir));
      movedOk += 1;
      const renamed = path.basename(dest) !== mv.titleName ? '  (重命名避让)' : '';
      console.log(`  ✔ ${mv.from}/${mv.titleName} → ${mv.to}/${path.basename(dest)}${renamed}`);
    } catch (e) {
      // ENOTEMPTY/EEXIST：目标在检查之后才被占用（大小写/Unicode 归一化、并发都可能）
      // → 强制加 `_{postId}` 后缀重试一次，避免整条失败。
      try {
        const retryDest = forcedUniqueDir(mv.targetDir, mv.titleName, mv.postId);
        fs.renameSync(mv.titleDir, retryDest);
        touchedAuthorDirs.add(path.dirname(mv.titleDir));
        movedOk += 1;
        console.log(`  ✔ ${mv.from}/${mv.titleName} → ${mv.to}/${path.basename(retryDest)}  (重试避让)`);
      } catch (retryError) {
        movedFail += 1;
        console.log(`  ✘ 失败 ${mv.titleDir}`);
        console.log(`     原因: ${e.message}`);
        console.log(`     重试仍失败: ${retryError.message}`);
      }
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

/* ---------- 4.5 执行重命名 ---------- */
if (args.renameDirs && renames.length) {
  let renamedOk = 0;
  let renamedFail = 0;
  for (const r of renames) {
    try {
      // 同样在执行时重算落点：同批次可能有两条帖子的 title 完全一致
      const dest = uniqueNameInDir(r.parent, r.want, r.cur);
      fs.renameSync(r.titleDir, dest);
      renamedOk += 1;
      console.log(`  ✔ ${r.cur} → ${path.basename(dest)}`);

      // Obsidian hub 同步：hub 目录名含标题，笔记里的 ![[...]] 又是库内完整路径，两者都要跟着改
      if (vaultUsable) {
        const rel = path.relative(exportRoot, r.titleDir);
        const newRel = path.join(path.dirname(rel), path.basename(dest));
        const oldHub = path.join(vaultRoot, 'Beav导出', rel);
        const newHub = path.join(vaultRoot, 'Beav导出', newRel);
        if (fs.existsSync(oldHub)) {
          try {
            fs.mkdirSync(path.dirname(newHub), { recursive: true });
            if (!fs.existsSync(newHub)) fs.renameSync(oldHub, newHub);
            const note = path.join(newHub, 'knowledge.md');
            if (fs.existsSync(note)) {
              const before = fs.readFileSync(note, 'utf8');
              const after = before.split(`/${r.cur}/`).join(`/${path.basename(dest)}/`);
              if (after !== before) fs.writeFileSync(note, after, 'utf8');
            }
          } catch (hubError) {
            console.log(`     ⚠ hub 同步失败（产物目录已改名）: ${hubError.message}`);
          }
        }
      }
    } catch (e) {
      renamedFail += 1;
      console.log(`  ✘ 重命名失败 ${r.titleDir}`);
      console.log(`     原因: ${e.message}`);
    }
  }
  console.log(`\n重命名结果：成功 ${renamedOk} · 失败 ${renamedFail}`);
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
