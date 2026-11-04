/**
 * 按 `帖子信息.json` 里的标题，把导出目录名统一成当前前缀长度（40 字）。
 *
 * 为什么需要：`EXPORT_TITLE_PREFIX_LEN` 从 20 调到 40 之后，只有「该帖下次被自动导出」
 * 时才会就地改名（惰性）。早先被截成 20 字的目录（`…AI根本没有在思`）如果那篇帖子
 * 再也不导出，名字就一直短着。这个脚本一次性把它们改到位。
 *
 * 标题真值取产物目录里的 `帖子信息.json`（导出时写入的 `title`），所以不依赖 store
 * 里的记录，也不受标题后续清洗影响 —— 改的就是这份产物当初记下的标题。
 *
 * 安全约束：
 *   · 默认 dry-run，只列不改；`--apply` 才真改名；
 *   · 只做同目录改名（`fs.renameSync`），不移动文件、不改内容；
 *   · 目标名已存在（别的帖子同前缀）就跳过，绝不覆盖；
 *   · 名字里已有 `_{postId}` 后缀的（历史撞名产物）保持不动；
 *   · 跳过 `_dedupe-backup-*` 备份目录。
 *
 * 注意：若配置了 Obsidian 资料库，`<vault>/Beav导出/` 里的镜像卡片是按旧路径写的，
 * 改名后那批笔记的链接会失效，直到对应帖子下次导出重写为止。脚本会提示这一点。
 *
 * 用法：
 *   node scripts/maintenance/rename-export-dirs.mjs                 # 预演
 *   node scripts/maintenance/rename-export-dirs.mjs --limit 20      # 只看前 20 条
 *   node scripts/maintenance/rename-export-dirs.mjs --apply         # 真改名
 */
import fs from 'node:fs';
import path from 'node:path';
import {
  readStore,
  resolveScanRoots,
  collectPostDirs,
  BACKUP_PREFIX,
} from './_export-dirs.mjs';

/** 与 Rust `EXPORT_TITLE_PREFIX_LEN` 保持一致（改一处记得改两处）。 */
const PREFIX_LEN = 40;
const META_FILE = '帖子信息.json';

function arg(name, fallback = '') {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
const hasFlag = (name) => process.argv.includes(`--${name}`);

const APPLY = hasFlag('apply');
const LIMIT = Number(arg('limit', '0')) || 0;

/** 复刻 Rust `sanitize_fs_name`：非法字符/控制字符→ `_`，折叠连续下划线，首尾去 `_`。 */
function sanitizeFsName(value) {
  const illegal = new Set(['\\', '/', ':', '*', '?', '"', '<', '>', '|']);
  let out = '';
  for (const char of value) {
    const code = char.codePointAt(0);
    const isControl = code < 0x20 || (code >= 0x7f && code <= 0x9f);
    out += illegal.has(char) || isControl ? '_' : char;
  }
  out = out.replace(/_{2,}/g, '_');
  return out.trim().replace(/^_+|_+$/g, '');
}

/** 复刻 Rust `title_export_prefix`。 */
function titlePrefix(title) {
  const trimmed = String(title ?? '').trim();
  if (!trimmed) return '未命名';
  const cut = [...trimmed].slice(0, PREFIX_LEN).join('');
  return sanitizeFsName(cut) || '未命名';
}

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
console.log(APPLY ? '模式: APPLY（会改名）' : '模式: dry-run（只看不改）');

const vault = String(ctx.store.monitorObsidianVault ?? '').trim();
if (vault && fs.existsSync(path.join(vault, 'Beav导出'))) {
  console.log(
    `⚠ 检测到 Obsidian 资料库 ${vault}：改名后 <Beav导出/> 里的镜像卡片要到该帖下次导出才会重写`,
  );
}

const dirs = collectPostDirs(scan.exportRoot);
const planned = [];
let skippedHasMeta = 0;
let skippedSame = 0;
let skippedConflict = 0;
let suffixCount = 0;

for (const item of dirs) {
  const current = path.basename(item.dir);
  if (current.startsWith(BACKUP_PREFIX)) continue;
  let title = '';
  try {
    const meta = JSON.parse(fs.readFileSync(path.join(item.dir, META_FILE), 'utf8'));
    title = String(meta.title ?? '');
  } catch {
    skippedHasMeta += 1;
    continue;
  }
  if (!title.trim()) {
    skippedHasMeta += 1;
    continue;
  }
  const desired = titlePrefix(title);
  if (current === desired) {
    skippedSame += 1;
    continue;
  }
  // 历史撞名产物带 `_{postId}` 后缀：允许去掉后缀还原干净名（下面 exists 检查会兜住真撞名）
  if (current.startsWith(`${desired}_`)) {
    suffixCount += 1;
  }
  const target = path.join(path.dirname(item.dir), desired);
  if (fs.existsSync(target)) {
    skippedConflict += 1;
    console.log(`! 跳过（目标已存在）: ${current}  →  ${desired}`);
    continue;
  }
  planned.push({ from: item.dir, to: target, current, desired });
}

const shown = LIMIT > 0 ? planned.slice(0, LIMIT) : planned;
for (const item of shown) {
  console.log(`${item.current}\n  → ${item.desired}`);
}
if (LIMIT > 0 && planned.length > LIMIT) {
  console.log(`…… 还有 ${planned.length - LIMIT} 条未列出（去掉 --limit 看全量）`);
}

let renamed = 0;
if (APPLY) {
  for (const item of planned) {
    try {
      fs.renameSync(item.from, item.to);
      renamed += 1;
    } catch (error) {
      console.log(`! 改名失败 ${item.current}: ${error.message}`);
    }
  }
}

console.log('');
console.log(
  `扫描产物目录 ${dirs.length} 个 · 待改名 ${planned.length} 个` +
    `（已是目标名 ${skippedSame} / 缺标题 ${skippedHasMeta} / 撞名跳过 ${skippedConflict}` +
    ` / 其中带 postId 后缀 ${suffixCount} 个（能还原干净名就还原，撞名则保留）`,
);
if (APPLY) {
  console.log(`已改名 ${renamed} 个（原名与新名都指向同一内容，仅目录名变化）`);
} else {
  console.log('加 --apply 才会真正改名（同目录 rename，不移动文件、不改内容）。');
}
