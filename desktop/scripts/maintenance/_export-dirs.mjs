/**
 * 导出库目录扫描的公共实现（report / dedupe 两个脚本共用）。
 *
 * 产物目录的识别标记是 `.beav-post-id`（导出时写入）。导出树固定是
 * `导出根/平台/日期/作者/标题前缀/`，所以扫到带标记的目录就不再往下钻。
 */
import fs from 'node:fs';
import path from 'node:path';
import { resolveStorePath, describeStorePath } from './_store-path.mjs';

export const POST_ID_MARKER = '.beav-post-id';
export const BACKUP_PREFIX = '_dedupe-backup-';
/** Obsidian 卡片树在库里的固定子目录名（产物目录的镜像）。 */
export const HUB_DIR_NAME = 'Beav导出';

export function readStore(explicitStore = '') {
  const resolved = resolveStorePath(explicitStore);
  if (!fs.existsSync(resolved.file)) {
    throw new Error(`store 不存在: ${resolved.file}（来源=${resolved.from}）`);
  }
  return {
    resolved,
    label: describeStorePath(resolved),
    store: JSON.parse(fs.readFileSync(resolved.file, 'utf8')),
  };
}

/**
 * 解析要扫描的树：导出根（产物）+ Obsidian 库里的 `Beav导出/`（镜像卡片树）。
 * 返回绝对路径数组，第一项恒为导出根。
 */
export function resolveScanRoots(store, explicitRoot = '') {
  const exportRoot = path.resolve(
    String(explicitRoot || process.env.BEAV_EXPORT_ROOT || store.monitorExportRoot || '').trim(),
  );
  if (!exportRoot || !fs.existsSync(exportRoot)) {
    throw new Error(`导出根不存在: ${exportRoot || '(未配置 monitorExportRoot)'}`);
  }
  const roots = [exportRoot];
  const vault = String(store.monitorObsidianVault ?? '').trim();
  if (vault) {
    const hubRoot = path.join(vault, HUB_DIR_NAME);
    if (fs.existsSync(hubRoot)) roots.push(hubRoot);
  }
  return { exportRoot, roots };
}

/** 递归找出所有「帖子产物目录」。导出树只有 5 层，深度给 8 足够。 */
export function collectPostDirs(dir, depth = 0, out = []) {
  if (depth > 8) return out;
  let entries;
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return out;
  }
  if (entries.some((entry) => entry.name === POST_ID_MARKER)) {
    let postId = '';
    try {
      postId = fs.readFileSync(path.join(dir, POST_ID_MARKER), 'utf8').trim();
    } catch {
      postId = '';
    }
    if (postId) out.push({ postId, dir });
    return out; // 产物目录是扁平的，不再往下钻
  }
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    if (entry.name.startsWith('.')) continue;
    if (entry.name.startsWith(BACKUP_PREFIX)) continue;
    collectPostDirs(path.join(dir, entry.name), depth + 1, out);
  }
  return out;
}

export function listFiles(dir) {
  try {
    return fs
      .readdirSync(dir, { withFileTypes: true })
      .filter((entry) => entry.isFile() && !entry.name.startsWith('.'))
      .map((entry) => entry.name);
  } catch {
    return [];
  }
}

/** 目录内文件总字节数（不含隐藏文件，用于估算冗余占用）。 */
export function dirSize(dir) {
  let total = 0;
  for (const name of listFiles(dir)) {
    try {
      total += fs.statSync(path.join(dir, name)).size;
    } catch {
      // 统计失败就当 0，不影响报告
    }
  }
  return total;
}

/** 越「全」分越高：视频和转写稿最能代表这份产物是否完整。 */
export function scoreDir(dir) {
  const files = listFiles(dir);
  let total = files.length;
  if (files.some((name) => name.startsWith('视频.'))) total += 1000;
  if (files.includes('转写全文.txt')) total += 500;
  if (files.includes('正文.txt')) total += 100;
  if (files.some((name) => name.startsWith('抽帧_'))) total += 50;
  let mtime = 0;
  try {
    mtime = fs.statSync(dir).mtimeMs;
  } catch {
    mtime = 0;
  }
  return { total, mtime, files };
}

/** 按 postId 分组，返回 { all, byPost, groups }（groups 只含重复的）。 */
export function groupByPost(root) {
  const all = collectPostDirs(root);
  const byPost = new Map();
  for (const item of all) {
    if (!byPost.has(item.postId)) byPost.set(item.postId, []);
    byPost.get(item.postId).push(item.dir);
  }
  const groups = [...byPost.entries()].filter(([, dirs]) => dirs.length > 1);
  return { all, byPost, groups };
}

/** 从 `导出根/平台/日期/作者/标题` 里取平台（第一段），取不到就标「未知」。 */
export function platformOf(root, dir) {
  const rel = path.relative(root, dir);
  const first = rel.split(path.sep)[0];
  return first && first !== rel ? first : '未知';
}

export function formatBytes(bytes) {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${bytes} B`;
}
