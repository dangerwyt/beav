/** macOS: move exported videos and the monitor cache; keep notes/images/subtitles in place. */
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { createHash, randomUUID } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';
import { parseArgs } from 'node:util';
import { setTimeout as delay } from 'node:timers/promises';
import net from 'node:net';

const VIDEO = /^视频\.(mp4|mov|m4v|webm|mkv|avi|ogv)$/i;
const MARKER = '.beav-post-id';
const RECEIPT = '.beav-video-migration.json';
const DAEMON = 'http://127.0.0.1:31937';
const labels = ['com.beav.web', 'com.beav.desktop'];
const stat = (file) => fs.lstatSync(file, { throwIfNoEntry: false });
const inside = (root, file) => { const rel = path.relative(root, file); return rel && !rel.startsWith(`..${path.sep}`) && rel !== '..' && !path.isAbsolute(rel); };
const safeId = (id) => id && id !== '.' && id !== '..' && !/[\\/:\[\]|#]/.test(id);

export async function hash(file) {
  const digest = createHash('sha256');
  for await (const chunk of fs.createReadStream(file)) digest.update(chunk);
  return digest.digest('hex');
}

function regularFiles(root) {
  if (!stat(root)) return [];
  if (!stat(root).isDirectory() || stat(root).isSymbolicLink()) throw new Error(`源目录不是普通目录：${root}`);
  const found = [];
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const file = path.join(dir, entry.name);
      if (entry.isDirectory()) walk(file);
      else if (entry.isFile()) found.push(file);
      else throw new Error(`目录中含链接或特殊文件，请先检查：${file}`);
    }
  };
  walk(root);
  return found;
}

export function rewriteVideoEmbeds(text, id, names, prefixes = ['']) {
  return text.replace(/!\[\[([^\]\n]+)\]\]/g, (embed, target) => {
    const [file, ...options] = target.split('|');
    if (!prefixes.some((prefix) => names.some((name) => file === prefix + name))) return embed;
    return `![[导出视频/${id}/${file.split('/').at(-1)}${options.length ? `|${options.join('|')}` : ''}]]`;
  });
}

export function planMigration(exportsRoot, cacheRoot, volume, vault = '') {
  const videosRoot = path.join(volume, '导出视频');
  const cacheTarget = path.join(volume, '视频缓存');
  const plan = { version: 1, stage: 'planned', exportsRoot, cacheRoot, videosRoot, cacheTarget,
    cacheBackup: `${cacheRoot}.beav-migration-backup`, files: [], notes: [], services: [] };
  const walk = (dir) => {
    const entries = fs.readdirSync(dir, { withFileTypes: true });
    if (entries.some((entry) => entry.name === MARKER)) {
      const id = fs.readFileSync(path.join(dir, MARKER), 'utf8').trim();
      if (!safeId(id)) throw new Error(`无效帖子标识：${dir}`);
      const videos = entries.filter((entry) => VIDEO.test(entry.name));
      for (const entry of videos) {
        if (!entry.isFile()) throw new Error(`视频不是普通文件：${path.join(dir, entry.name)}`);
        plan.files.push({ source: path.join(dir, entry.name), target: path.join(videosRoot, id, entry.name), id, kind: 'export' });
      }
      if (videos.length) {
        const noteDirs = [dir];
        if (vault && inside(vault, dir)) noteDirs.push(path.join(vault, 'Beav导出', path.relative(exportsRoot, dir)));
        for (const noteDir of new Set(noteDirs)) {
          for (const name of ['knowledge.md', 'Beav知识卡.md']) {
            const file = path.join(noteDir, name);
            if (!stat(file)) continue;
            if (!stat(file).isFile()) throw new Error(`笔记不是普通文件：${file}`);
            const before = fs.readFileSync(file, 'utf8');
            const prefixes = ['', `${path.relative(exportsRoot, dir).split(path.sep).join('/')}/`];
            if (vault && inside(vault, dir)) prefixes.push(`${path.relative(vault, dir).split(path.sep).join('/')}/`);
            const after = rewriteVideoEmbeds(before, id, videos.map((entry) => entry.name), prefixes);
            // Only generated wiki embeds are migrated; never delete a video with an unhandled reference.
            const remaining = after.replace(/!\[\[导出视频\/[^\]\n]+\]\]/g, '');
            if (videos.some((entry) => remaining.includes(entry.name))) {
              throw new Error(`笔记含非标准视频引用，尚未修改：${file}`);
            }
            if (before !== after) plan.notes.push({ file, before, after });
          }
        }
      }
      return;
    }
    for (const entry of entries) {
      if (entry.name.startsWith('.') || entry.name === '导出视频') continue;
      if (entry.isDirectory()) walk(path.join(dir, entry.name));
      else if (entry.isSymbolicLink()) throw new Error(`导出目录含未识别链接：${path.join(dir, entry.name)}`);
    }
  };
  walk(exportsRoot);
  for (const file of regularFiles(cacheRoot)) {
    plan.files.push({ source: file, target: path.join(cacheTarget, path.relative(cacheRoot, file)), kind: 'cache' });
  }
  return plan;
}

export async function copyVerified(source, target) {
  const before = stat(source);
  if (!before?.isFile()) throw new Error(`源文件不是普通文件：${source}`);
  const expected = await hash(source);
  if (stat(target)) {
    if (!stat(target).isFile() || await hash(target) !== expected) throw new Error(`目标已存在且内容不同，拒绝覆盖：${target}`);
  } else {
    fs.mkdirSync(path.dirname(target), { recursive: true });
    const temp = `${target}.beav-copy-${randomUUID()}`;
    try {
      fs.copyFileSync(source, temp, fs.constants.COPYFILE_EXCL);
      if (await hash(temp) !== expected) throw new Error(`复制校验失败：${source}`);
      if (stat(target)) throw new Error(`复制期间目标被创建：${target}`);
      fs.renameSync(temp, target);
    } finally { if (stat(temp)) fs.unlinkSync(temp); }
  }
  const after = stat(source);
  if (before.size !== after?.size || before.mtimeMs !== after?.mtimeMs) throw new Error(`复制期间源文件发生变化：${source}`);
  return expected;
}

function ensureLink(link, target) {
  const existing = stat(link);
  if (existing) {
    if (!existing.isSymbolicLink() || fs.realpathSync(link) !== fs.realpathSync(target)) throw new Error(`链接位置已被占用：${link}`);
    return;
  }
  fs.symlinkSync(target, link, process.platform === 'win32' ? 'junction' : 'dir');
}

export async function applyPlan(plan, save = () => {}) {
  if (plan.stage !== 'planned') return;
  const cacheSource = stat(plan.cacheBackup) ? plan.cacheBackup : plan.cacheRoot;
  const expectedCache = new Set(plan.files.filter((item) => item.kind === 'cache').map((item) => path.relative(plan.cacheRoot, item.source)));
  const actualCache = regularFiles(cacheSource).map((file) => path.relative(cacheSource, file));
  if (actualCache.length !== expectedCache.size || actualCache.some((file) => !expectedCache.has(file))) {
    throw new Error('缓存文件清单已变化，保留原缓存；请重新核对迁移记录。');
  }
  fs.mkdirSync(plan.videosRoot, { recursive: true });
  fs.mkdirSync(plan.cacheTarget, { recursive: true });
  for (const item of plan.files) {
    const source = item.kind === 'cache' && stat(plan.cacheBackup)
      ? path.join(plan.cacheBackup, path.relative(plan.cacheRoot, item.source)) : item.source;
    item.sha256 = await copyVerified(source, item.target);
    if (item.kind === 'export') {
      const marker = path.join(path.dirname(item.target), MARKER);
      if (stat(marker) && fs.readFileSync(marker, 'utf8').trim() !== item.id) throw new Error(`视频目录标识冲突：${marker}`);
      fs.writeFileSync(marker, item.id);
    }
  }
  save(); // Save hashes before changing paths. Source videos are retained until --cleanup.
  ensureLink(path.join(plan.exportsRoot, '导出视频'), plan.videosRoot);
  for (const note of plan.notes) {
    const current = fs.readFileSync(note.file, 'utf8');
    if (current !== note.before && current !== note.after) throw new Error(`笔记已修改，拒绝覆盖：${note.file}`);
    const temp = `${note.file}.beav-migration-tmp`;
    if (stat(temp) && (!stat(temp).isFile() || fs.readFileSync(temp, 'utf8') !== note.after)) throw new Error(`笔记临时文件冲突：${temp}`);
    if (!stat(temp)) fs.writeFileSync(temp, note.after, { flag: 'wx' });
    fs.renameSync(temp, note.file);
  }
  if (stat(plan.cacheRoot) && !stat(plan.cacheRoot).isSymbolicLink()) {
    if (stat(plan.cacheBackup)) throw new Error(`缓存备份已存在：${plan.cacheBackup}`);
    fs.renameSync(plan.cacheRoot, plan.cacheBackup);
  }
  try { ensureLink(plan.cacheRoot, plan.cacheTarget); }
  catch (error) {
    if (!stat(plan.cacheRoot) && stat(plan.cacheBackup)) fs.renameSync(plan.cacheBackup, plan.cacheRoot);
    throw error;
  }
  plan.stage = 'linked';
  save();
}

export async function cleanupPlan(plan, save = () => {}) {
  if (!['verified', 'cleaned'].includes(plan.stage)) throw new Error('必须先完成迁移和服务播放检查，再清理旧视频。');
  for (const [link, target] of [[path.join(plan.exportsRoot, '导出视频'), plan.videosRoot], [plan.cacheRoot, plan.cacheTarget]]) {
    if (!stat(link)?.isSymbolicLink() || fs.realpathSync(link) !== fs.realpathSync(target)) throw new Error(`目录链接已变化，拒绝清理：${link}`);
  }
  const removable = [];
  for (const item of plan.files) {
    const source = item.kind === 'cache' ? path.join(plan.cacheBackup, path.relative(plan.cacheRoot, item.source)) : item.source;
    if (!stat(source)) continue;
    if (!stat(source).isFile() || !stat(item.target)?.isFile()
      || await hash(source) !== item.sha256 || await hash(item.target) !== item.sha256) {
      throw new Error(`原件或外接盘副本发生变化，保留原件：${source}`);
    }
    removable.push(source);
  }
  // Refuse to delete new/unrecorded files or symlinks in the backup directory.
  const expected = new Set(removable.filter((file) => inside(plan.cacheBackup, file)));
  for (const file of regularFiles(plan.cacheBackup)) {
    if (!expected.has(file)) throw new Error(`缓存备份含未记录文件，拒绝清理：${file}`);
  }
  for (const file of removable) fs.unlinkSync(file);
  const removeEmpty = (dir) => {
    if (!stat(dir)) return;
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      if (entry.isDirectory()) removeEmpty(path.join(dir, entry.name));
    }
    fs.rmdirSync(dir); // Deliberately non-recursive: unexpected data cannot be deleted.
  };
  removeEmpty(plan.cacheBackup);
  plan.stage = 'cleaned';
  save();
}

async function invoke(channel) {
  const response = await fetch(`${DAEMON}/api/ipc/invoke`, { method: 'POST',
    headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ channel, payload: {} }), signal: AbortSignal.timeout(10000) });
  const value = await response.json();
  if (!response.ok || value.success === false) throw new Error(`读取 ${channel} 失败，请确认 Beav 服务已启动。`);
  return value;
}

function launchctl(...args) { return spawnSync('launchctl', args, { encoding: 'utf8' }); }
async function running(url) { try { return (await fetch(url, { signal: AbortSignal.timeout(2000) })).ok; } catch { return false; } }

function portOpen(port) {
  return new Promise((resolve) => {
    const socket = net.connect({ host: '127.0.0.1', port });
    const finish = (value) => { socket.destroy(); resolve(value); };
    socket.once('connect', () => finish(true));
    socket.once('error', () => finish(false));
    socket.setTimeout(1000, () => finish(true));
  });
}

async function stopServices(services) {
  const pids = [];
  for (const service of services) {
    const key = `gui/${process.getuid()}/${service}`;
    const status = launchctl('print', key);
    const pid = status.stdout?.match(/\bpid = (\d+)/)?.[1];
    if (pid) pids.push(Number(pid));
    if (status.status === 0 && launchctl('bootout', key).status !== 0) throw new Error(`停止服务失败：${service}`);
  }
  for (let attempt = 0; attempt < 30; attempt++) {
    const alive = pids.some((pid) => { try { process.kill(pid, 0); return true; } catch { return false; } });
    if (!alive && !await portOpen(31937) && !await portOpen(8090)) return;
    await delay(1000);
  }
  throw new Error('服务仍在运行，请关闭手动启动的 Beav 后重试。尚未移动文件。');
}

async function startServices(services) {
  for (const service of [...services].reverse()) {
    if (launchctl('print', `gui/${process.getuid()}/${service}`).status === 0) continue;
    const plist = path.join(os.homedir(), 'Library/LaunchAgents', `${service}.plist`);
    if (launchctl('bootstrap', `gui/${process.getuid()}`, plist).status !== 0) throw new Error(`恢复服务失败：${service}，请运行 deploy/update-server.sh。`);
  }
  for (let attempt = 0; attempt < 60; attempt++) {
    if (await running(`${DAEMON}/api/ipc/health`) && (!services.includes('com.beav.web') || await running('http://127.0.0.1:8090/login'))) return;
    await delay(1000);
  }
  throw new Error('服务启动检查超时；原视频与缓存备份仍保留。');
}

async function verifyPlayback(plan) {
  for (const kind of ['export', 'cache']) {
    const item = plan.files.find((file) => file.kind === kind && /\.(mp4|webm|mov|m4v)$/i.test(file.target) && fs.statSync(file.target).size > 0);
    if (!item) continue;
    const source = kind === 'cache' ? item.source : path.join(plan.exportsRoot, '导出视频', item.id, path.basename(item.source));
    const response = await fetch(`${DAEMON}/api/file?path=${encodeURIComponent(source)}`, { headers: { Range: 'bytes=0-15' }, signal: AbortSignal.timeout(15000) });
    if (response.status !== 206) { await response.body?.cancel(); throw new Error(`视频读取检查失败（${response.status}）：${source}`); }
    const actual = Buffer.from(await response.arrayBuffer());
    const expected = Buffer.alloc(Math.min(16, fs.statSync(item.target).size));
    const fd = fs.openSync(item.target, 'r');
    try { fs.readSync(fd, expected, 0, expected.length, 0); } finally { fs.closeSync(fd); }
    if (!actual.equals(expected)) throw new Error(`视频读取内容不一致：${source}`);
  }
}

async function main() {
  const { values } = parseArgs({ options: { exports: { type: 'string' }, volume: { type: 'string' },
    apply: { type: 'boolean' }, cleanup: { type: 'boolean' }, help: { type: 'boolean' } } });
  if (values.help) {
    console.log('用法：node scripts/maintenance/migrate-video-storage.mjs --exports "/原导出目录" --volume "/Volumes/mini bak" [--apply | --cleanup]\n默认只读预览。--apply 复制校验、修改视频引用并切换缓存，保留原件。确认播放正常后 --cleanup 校验并删除旧视频及缓存备份。');
    return;
  }
  if (process.platform !== 'darwin') throw new Error('实际迁移仅支持 macOS；单元测试可在其他系统运行。');
  if (!values.exports || !values.volume || (values.apply && values.cleanup)) throw new Error('必须指定 --exports 和 --volume；--apply 与 --cleanup 不能同时使用。');
  const exportsRoot = path.resolve(values.exports), volume = path.resolve(values.volume);
  if (!stat(exportsRoot)?.isDirectory() || stat(exportsRoot).isSymbolicLink()) throw new Error('原导出目录必须是内置盘上的普通目录，不能是整目录链接。');
  if (path.dirname(volume) !== '/Volumes' || !stat(volume)?.isDirectory() || stat(volume).isSymbolicLink()
    || fs.statSync(volume).dev === fs.statSync('/Volumes').dev) throw new Error(`外接硬盘未挂载：${volume}`);
  if (inside(volume, exportsRoot) || exportsRoot === volume) throw new Error('导出根目录应保留在内置盘。');
  const receipt = path.join(exportsRoot, RECEIPT);
  let plan, vault = '', fresh = false;
  if (stat(receipt)) {
    if (!stat(receipt).isFile()) throw new Error('迁移记录不是普通文件。');
    plan = JSON.parse(fs.readFileSync(receipt, 'utf8'));
    if (plan.version !== 1 || plan.exportsRoot !== exportsRoot || plan.videosRoot !== path.join(volume, '导出视频')
      || plan.cacheTarget !== path.join(volume, '视频缓存') || plan.cacheBackup !== `${plan.cacheRoot}.beav-migration-backup`
      || !path.isAbsolute(plan.cacheRoot) || path.basename(plan.cacheRoot) !== 'monitor-videos') throw new Error('迁移记录与本次路径不一致。');
    if (!Array.isArray(plan.services) || !plan.services.includes('com.beav.desktop') || plan.services.some((service) => !labels.includes(service))) throw new Error('迁移记录中的服务列表无效。');
    for (const item of plan.files) {
      if (!['cache', 'export'].includes(item.kind) || (item.kind === 'export' && (!VIDEO.test(path.basename(item.source)) || !safeId(item.id)))
        || !inside(item.kind === 'cache' ? plan.cacheRoot : exportsRoot, item.source)
        || !inside(item.kind === 'cache' ? plan.cacheTarget : plan.videosRoot, item.target)) throw new Error('迁移记录中存在越界路径。');
    }
  } else {
    if (values.cleanup) throw new Error('未找到迁移记录，不能清理。');
    const config = await invoke('monitor:get-export-root');
    if (config.externalVideoStorageVersion !== 1 || path.resolve(config.effectiveRoot) !== exportsRoot) throw new Error('请先运行 deploy/update-server.sh 更新服务，并核对原导出目录。');
    vault = (await invoke('monitor:get-obsidian-vault')).vault || '';
    for (const target of [path.join(exportsRoot, '导出视频'), path.join(volume, '导出视频'), path.join(volume, '视频缓存'), `${config.videoCacheRoot}.beav-migration-backup`]) {
      if (stat(target)) throw new Error(`迁移位置已存在，请先核对，脚本不会覆盖：${target}`);
    }
    if (stat(config.videoCacheRoot)?.isSymbolicLink()) throw new Error('视频缓存已是链接，请先核对位置。');
    plan = planMigration(exportsRoot, config.videoCacheRoot, volume, vault);
    fresh = true;
    plan.services = labels.filter((label) => launchctl('print', `gui/${process.getuid()}/${label}`).status === 0);
    if (!plan.services.includes('com.beav.desktop')) throw new Error('脚本要求使用 com.beav.desktop 的 launchd 部署；请先确认服务启动方式。');
  }
  console.log(`笔记、图片、字幕保留：${exportsRoot}\n导出视频：${plan.videosRoot}\n视频缓存：${plan.cacheTarget}\n待处理文件：${plan.files.length}，视频笔记：${plan.notes.length}，阶段：${plan.stage}`);
  if (!values.apply && !values.cleanup) { console.log('只读预览完成，未改动文件。'); return; }
  for (const dir of [plan.videosRoot, plan.cacheTarget]) {
    if (stat(dir) && (!stat(dir).isDirectory() || stat(dir).isSymbolicLink())) throw new Error(`目标必须是普通目录：${dir}`);
  }
  const probe = path.join(volume, `.beav-write-test-${randomUUID()}`);
  fs.writeFileSync(probe, '', { flag: 'wx' }); fs.unlinkSync(probe);
  const save = () => {
    const temp = `${receipt}.${randomUUID()}.tmp`;
    fs.writeFileSync(temp, JSON.stringify(plan, null, 2), { flag: 'wx' });
    fs.renameSync(temp, receipt);
  };
  save();
  try {
    await stopServices(plan.services);
    if (values.cleanup) await cleanupPlan(plan, save);
    else {
      if (fresh) {
        const services = plan.services;
        plan = planMigration(exportsRoot, plan.cacheRoot, volume, vault);
        plan.services = services;
        save();
      }
      const bytes = plan.files.reduce((sum, file) => sum + (stat(file.source)?.size || 0), 0);
      const space = fs.statfsSync(volume);
      if (plan.stage === 'planned' && space.bavail * space.bsize < bytes) throw new Error('外接盘剩余空间不足，原文件未改动。');
      await applyPlan(plan, save);
    }
  } finally { await startServices(plan.services); }
  await verifyPlayback(plan);
  if (plan.stage === 'linked') { plan.stage = 'verified'; save(); }
  console.log(values.cleanup ? '旧视频与缓存备份已清理；笔记、图片、字幕仍在原目录。' : '迁移和服务视频读取检查通过。原件仍保留；请检查 Web 视频和 knowledge.md 内联视频，再运行 --cleanup 释放内置盘空间。');
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main().catch((error) => { console.error(`迁移停止：${error.message}`); process.exitCode = 1; });
}
