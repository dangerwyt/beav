/** Repair saved frame lists and exports through IPC; never rerun ffmpeg or transcription. */
import fs from 'node:fs';
import path from 'node:path';
import { parseArgs } from 'node:util';
import { pathToFileURL } from 'node:url';
import { describeStorePath, resolveStorePath } from './_store-path.mjs';

export function frameRepairs(store, postId = '') {
  const posts = new Set((store.monitoredPosts || []).map((post) => post.id));
  let orphaned = 0;
  const repairs = (store.postVideoAnalyses || []).filter((record) => !postId || record.postId === postId).flatMap((record) => {
    const before = Array.isArray(record.frames) ? record.frames : [];
    const metadata = before.filter((file) => file.split(/[\\/]/).at(-1).startsWith('._')).length;
    if (!metadata) return [];
    if (!posts.has(record.postId)) {
      orphaned += 1;
      return [];
    }
    // Match the backend's frame_<positive integer>.jpg rule and numeric ordering.
    const numbered = before.flatMap((file) => {
      const match = file.split(/[\\/]/).at(-1).match(/^frame_(\d+)\.jpg$/);
      return match && BigInt(match[1]) > 0n && BigInt(match[1]) <= 18446744073709551615n ? [{ number: BigInt(match[1]), file }] : [];
    }).sort((a, b) => a.number < b.number ? -1 : a.number > b.number ? 1 : a.file < b.file ? -1 : a.file > b.file ? 1 : 0);
    const frames = [...new Set(numbered.map((item) => item.file))];
    return [{ postId: record.postId, title: record.postTitle || '', before: before.length, frames, metadata }];
  });
  return { repairs, orphaned };
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

export async function repairFrames(store, { apply = false, postId = '' } = {}, call = invoke, log = console.log) {
  const { repairs, orphaned } = frameRepairs(store, postId);
  let repaired = 0;
  let skipped = orphaned;
  log(`发现 ${repairs.length} 条混入 ._ 元数据、需要修复的抽帧记录。`);
  if (orphaned) log(`跳过 ${orphaned} 条关联动态已不存在的受污染分析记录；保留原记录和文件。`);
  for (const repair of repairs) {
    log(`${repair.postId} | ${repair.title} | ${repair.before} → ${repair.frames.length} 张真实帧；._ 元数据 ${repair.metadata} 个`);
    if (!apply) continue;
    if (!repair.frames.length) throw new Error('记录中没有真实帧，已停止修复；请检查缓存文件。');
    const current = await call('monitor:video-analysis', { id: repair.postId });
    if (current.analysis === null) {
      skipped += 1;
      log(`跳过 ${repair.postId}：服务中分析记录已不存在，未重写此条导出文件。`);
      continue;
    }
    if (JSON.stringify(current.analysis?.frames) !== JSON.stringify(repair.frames)) {
      throw new Error('服务尚未更新，或这条记录正在重新解析。请更新服务、等待解析结束后重试；尚未导出此条。');
    }
    log('正在重写导出图片和 knowledge.md……');
    try {
      await call('monitor:export-post', { postId: repair.postId });
    } catch (error) {
      // The post can disappear after the store snapshot was read.
      if (error.message !== '动态不存在') throw error;
      skipped += 1;
      log(`跳过 ${repair.postId}：服务中动态不存在，未重写此条导出文件。`);
      continue;
    }
    repaired += 1;
    log('修复完成；旧编号的多余导出图片保存在素材目录内的 .beav-frame-backup-* 文件夹。');
  }
  if (apply) log(`本次修复完成：成功 ${repaired} 条，跳过 ${skipped} 条。`);
  else log('只读检查完成；需要修复时添加 --apply，可用 --post-id 限定单篇。');
  return { repaired, skipped };
}

async function main() {
  const { values } = parseArgs({ options: { apply: { type: 'boolean' }, store: { type: 'string' },
    'post-id': { type: 'string' }, help: { type: 'boolean' } } });
  if (values.help) {
    console.log('用法：node desktop/scripts/maintenance/repair-monitor-frames.mjs [--apply] [--post-id ID] [--store 文件]\n默认只读，只报告混入 ._ 元数据的抽帧记录；跳过关联动态已不存在的记录，不处理单纯乱序。--apply 通过运行中的新版 Beav 重写这些记录的导出图片和知识卡，保留正文、字幕、视频缓存；不重新转写。请先等待磁盘迁移/清理任务退出。');
    return;
  }
  const resolved = resolveStorePath(values.store);
  console.log(`读取数据：${describeStorePath(resolved)}`);
  const store = JSON.parse(fs.readFileSync(resolved.file, 'utf8'));
  await repairFrames(store, { apply: values.apply, postId: values['post-id'] });
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main().catch((error) => { console.error(`抽帧修复停止：${error.message}`); process.exitCode = 1; });
}
