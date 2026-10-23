#!/usr/bin/env node
/**
 * 一次性维护脚本：回填「本地视频是否含音轨」标记（videoHasAudio）。
 *
 * 背景：抖音走 DASH，`play_addr` 只给人视频轨，历史下载的 mp4 全是无声的 ——
 * 语音转写因此必然失败（ffmpeg 报 `Output file does not contain any stream`）。
 * 抓取侧已修（下载后探测 + 自动补音轨），本脚本用于把**存量**文件的真实状态写回 store，
 * 让 UI 能直接标出哪些视频需要「重新下载」。
 *
 * 同时把「无音轨且转写为空」的分析记录标成 `transcriptStatus = 'no-audio'`，
 * 避免界面显示成「转写服务坏了」。
 *
 * ⚠️ 必须先退出 Beav，否则应用内存里的旧快照会覆盖本脚本的写入。
 *
 * 用法：
 *   node scripts/maintenance/fix-video-audio-flag.mjs
 *   node scripts/maintenance/fix-video-audio-flag.mjs --store "D:/path/beav-state.json" --dry-run
 *   node scripts/maintenance/fix-video-audio-flag.mjs --ffprobe "C:/ffmpeg/bin/ffprobe.exe"
 */
import { readFileSync, writeFileSync, copyFileSync, existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { homedir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = join(dirname(fileURLToPath(import.meta.url)), '..', '..');

const DEFAULT_STORE =
  process.platform === 'win32'
    ? join(process.env.APPDATA ?? join(homedir(), 'AppData', 'Roaming'), 'Beav', 'beav-state.json')
    : join(homedir(), 'Library', 'Application Support', 'Beav', 'beav-state.json');

const SUFFIX = process.platform === 'win32' ? '.exe' : '';
const TRIPLE =
  process.platform === 'win32'
    ? 'x86_64-pc-windows-msvc'
    : process.arch === 'arm64'
      ? 'aarch64-apple-darwin'
      : 'x86_64-apple-darwin';
const DEFAULT_FFPROBE = join(root, 'src-tauri', 'binaries', `ffprobe-${TRIPLE}${SUFFIX}`);

function arg(name, fallback = null) {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

const storePath = arg('store', DEFAULT_STORE);
const ffprobe = arg('ffprobe', DEFAULT_FFPROBE);
const dryRun = process.argv.includes('--dry-run');

const NO_AUDIO_REASON =
  '源视频没有音轨（抖音 DASH 音视频分离，只下到了视频轨），无法转写。重新下载该视频会自动补音轨后再分析。';

function hasAudio(file) {
  if (!existsSync(file)) return null;
  const out = spawnSync(
    ffprobe,
    ['-v', 'error', '-select_streams', 'a', '-show_entries', 'stream=codec_name', '-of', 'csv=p=0', file],
    { encoding: 'utf8', timeout: 30_000 },
  );
  if (out.error) return null;
  return String(out.stdout ?? '').trim().length > 0;
}

function main() {
  if (!existsSync(storePath)) {
    console.error(`找不到 store 文件：${storePath}`);
    process.exit(1);
  }
  if (!existsSync(ffprobe)) {
    console.error(`找不到 ffprobe：${ffprobe}\n可用 --ffprobe <path> 指定`);
    process.exit(1);
  }

  const store = JSON.parse(readFileSync(storePath, 'utf8'));
  const posts = store.monitoredPosts ?? store.monitored_posts ?? [];
  const analyses = store.postVideoAnalyses ?? store.post_video_analyses ?? [];

  const probed = new Map();
  let withAudio = 0;
  let withoutAudio = 0;
  let skipped = 0;

  for (const p of posts) {
    const file = p.videoLocalPath ?? p.video_local_path;
    if (!file) continue;
    const flag = hasAudio(file);
    if (flag === null) {
      skipped += 1;
      continue;
    }
    p.videoHasAudio = flag;
    probed.set(p.id, flag);
    if (flag) withAudio += 1;
    else withoutAudio += 1;
  }

  let marked = 0;
  for (const a of analyses) {
    const id = a.postId ?? a.post_id;
    if (probed.get(id) === false && !String(a.transcript ?? '').trim()) {
      a.transcriptStatus = 'no-audio';
      a.transcriptError = NO_AUDIO_REASON;
      marked += 1;
    }
  }

  console.log(`store: ${storePath}`);
  console.log(`ffprobe: ${ffprobe}`);
  console.log(`带本地视频: ${withAudio + withoutAudio}`);
  console.log(`  有音轨: ${withAudio}`);
  console.log(`  无音轨: ${withoutAudio}`);
  console.log(`  探测失败/文件缺失: ${skipped}`);
  console.log(`分析记录标为 no-audio: ${marked}`);

  if (dryRun) {
    console.log('\n--dry-run：未写入');
    return;
  }
  if (withAudio + withoutAudio === 0) {
    console.log('\n没有可探测的本地视频');
    return;
  }
  const backup = storePath.replace(/\.json$/, `.backup-audio-${Date.now()}.json`);
  copyFileSync(storePath, backup);
  writeFileSync(storePath, JSON.stringify(store, null, 2), 'utf8');
  console.log(`\n已写入；备份: ${backup}`);
}

main();
