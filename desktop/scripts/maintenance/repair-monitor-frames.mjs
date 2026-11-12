/** Repair saved frame lists and exports through IPC; never rerun ffmpeg or transcription. */
import fs from 'node:fs';
import path from 'node:path';
import { parseArgs } from 'node:util';
import { pathToFileURL } from 'node:url';
import { resolveStorePath } from './_store-path.mjs';

export function frameRepairs(store, postId = '') {
  return (store.postVideoAnalyses || []).filter((record) => !postId || record.postId === postId).flatMap((record) => {
    const before = Array.isArray(record.frames) ? record.frames : [];
    // Match the backend's frame_<positive integer>.jpg rule and numeric ordering.
    const numbered = before.flatMap((file) => {
      const match = file.split(/[\\/]/).at(-1).match(/^frame_(\d+)\.jpg$/);
      return match && BigInt(match[1]) > 0n && BigInt(match[1]) <= 18446744073709551615n ? [{ number: BigInt(match[1]), file }] : [];
    }).sort((a, b) => a.number < b.number ? -1 : a.number > b.number ? 1 : a.file < b.file ? -1 : a.file > b.file ? 1 : 0);
    const frames = [...new Set(numbered.map((item) => item.file))];
    if (JSON.stringify(frames) === JSON.stringify(before)) return [];
    return [{ postId: record.postId, title: record.postTitle || '', before: before.length, frames,
      metadata: before.filter((file) => file.split(/[\\/]/).at(-1).startsWith('._')).length }];
  });
}

async function invoke(channel, payload) {
  const response = await fetch('http://127.0.0.1:31937/api/ipc/invoke', {
    method: 'POST', headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ channel, payload }), signal: AbortSignal.timeout(300000),
  });
  const result = await response.json();
  if (!response.ok || result.success === false) throw new Error(result.error || `${channel} 失败（HTTP ${response.status}）`);
  return result;
}

async function main() {
  const { values } = parseArgs({ options: { apply: { type: 'boolean' }, store: { type: 'string' },
    'post-id': { type: 'string' }, help: { type: 'boolean' } } });
  if (values.help) {
    console.log('用法：node desktop/scripts/maintenance/repair-monitor-frames.mjs [--apply] [--post-id ID] [--store 文件]\n默认只读，报告元数据文件污染或乱序的抽帧记录。--apply 通过运行中的新版 Beav 重写这些记录的导出图片和知识卡，保留正文、字幕、视频缓存；不重新转写。请先等待磁盘迁移/清理任务退出。');
    return;
  }
  const store = JSON.parse(fs.readFileSync(resolveStorePath(values.store).file, 'utf8'));
  const repairs = frameRepairs(store, values['post-id']);
  console.log(`发现 ${repairs.length} 条需要整理的抽帧记录。`);
  for (const repair of repairs) {
    console.log(`${repair.postId} | ${repair.title} | ${repair.before} → ${repair.frames.length} 张真实帧；._ 元数据 ${repair.metadata} 个`);
    if (!values.apply) continue;
    if (!repair.frames.length) throw new Error('记录中没有真实帧，已停止修复；请检查缓存文件。');
    const current = await invoke('monitor:video-analysis', { id: repair.postId });
    if (JSON.stringify(current.analysis?.frames) !== JSON.stringify(repair.frames)) {
      throw new Error('服务尚未更新，或这条记录正在重新解析。请更新服务、等待解析结束后重试；尚未导出此条。');
    }
    console.log('正在重写导出图片和 knowledge.md……');
    await invoke('monitor:export-post', { postId: repair.postId });
    console.log('修复完成；旧编号的多余导出图片保存在素材目录内的 .beav-frame-backup-* 文件夹。');
  }
  if (!values.apply) console.log('只读检查完成；需要修复时添加 --apply，可用 --post-id 限定单篇。');
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main().catch((error) => { console.error(`抽帧修复停止：${error.message}`); process.exitCode = 1; });
}
