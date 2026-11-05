/**
 * 把 beav-content-remix 技能的三份方法论文档同步进源码目录。
 *
 * 为什么需要：`src-tauri/src/commands/remix_prompts/*.md` 是**复制**进来的，
 * 由 `include_str!` 编译期内嵌进提示词（拆解/分镜/改写各用一份）。技能升级后
 * 这些副本不会自动更新 —— 本脚本负责比对与覆盖，避免两边悄悄分叉。
 *
 * 用法：
 *   node scripts/maintenance/sync-remix-prompts.mjs            # 比对 + 覆盖有差异的文件
 *   node scripts/maintenance/sync-remix-prompts.mjs --check     # 只比对（有差异退出码 1，可进 CI）
 *   node scripts/maintenance/sync-remix-prompts.mjs --src <dir> # 指定技能 references 目录
 *   node scripts/maintenance/sync-remix-prompts.mjs --zip <zip> # 从技能包 zip 同步
 *
 * 不传 --src/--zip 时默认读 `~/.workbuddy/skills/beav-content-remix/references`。
 */
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';

/** 技能 references/ 里的文件 → 源码目录里的文件名（同名，列出便于核对） */
const FILES = ['deconstruction.md', 'storyboard.md', 'rewrite-playbook.md'];

const TARGET_DIR = path.resolve('src-tauri/src/commands/remix_prompts');
const DEFAULT_SRC = path.join(
  os.homedir(),
  '.workbuddy',
  'skills',
  'beav-content-remix',
  'references',
);

function arg(name, fallback = '') {
  const i = process.argv.indexOf(`--${name}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
const hasFlag = (name) => process.argv.includes(`--${name}`);

const CHECK_ONLY = hasFlag('check');
const zipPath = arg('zip');
const srcDir = zipPath ? '' : path.resolve(arg('src', DEFAULT_SRC));

const md5 = (buf) => crypto.createHash('md5').update(buf).digest('hex');

function readSource(file) {
  if (zipPath) {
    // 依赖系统 unzip（macOS/Linux 自带；Windows 的 Git Bash 也有）
    return execFileSync('unzip', ['-p', zipPath, `beav-content-remix/references/${file}`], {
      maxBuffer: 8 * 1024 * 1024,
    });
  }
  return fs.readFileSync(path.join(srcDir, file));
}

if (!zipPath && !fs.existsSync(srcDir)) {
  console.error(`技能目录不存在: ${srcDir}`);
  console.error('用 --src <dir> 或 --zip <zip> 指定技能来源。');
  process.exit(1);
}
if (!fs.existsSync(TARGET_DIR)) {
  console.error(`目标目录不存在: ${TARGET_DIR}（请在 desktop/ 下运行本脚本）`);
  process.exit(1);
}

let changed = 0;
let same = 0;
const report = [];

for (const file of FILES) {
  let incoming;
  try {
    incoming = readSource(file);
  } catch (error) {
    console.error(`读取失败 ${file}: ${error.message}`);
    process.exit(1);
  }
  const targetPath = path.join(TARGET_DIR, file);
  const current = fs.existsSync(targetPath) ? fs.readFileSync(targetPath) : null;
  const incomingHash = md5(incoming);
  const currentHash = current ? md5(current) : '(缺失)';

  if (current && incomingHash === currentHash) {
    same += 1;
    report.push(`= ${file.padEnd(22)} ${incomingHash}`);
    continue;
  }

  changed += 1;
  report.push(`~ ${file.padEnd(22)} ${currentHash} -> ${incomingHash}`);
  if (!CHECK_ONLY) {
    // 统一 LF：跨平台编译结果一致，也避免 git 反复提示换行符
    fs.writeFileSync(targetPath, incoming.toString('utf8').replace(/\r\n/g, '\n'), 'utf8');
  }
}

console.log(`技能来源: ${zipPath ? `zip ${zipPath}` : srcDir}`);
console.log(`源码目录: ${TARGET_DIR}\n`);
report.forEach((line) => console.log(`  ${line}`));
console.log('');

if (!changed) {
  console.log(`三份文档已是最新（${same} 个一致），无需改动。`);
  process.exit(0);
}

if (CHECK_ONLY) {
  console.log(`发现 ${changed} 个文件与技能不一致。去掉 --check 即可同步。`);
  process.exit(1);
}

console.log(`已同步 ${changed} 个文件。记得重新编译（include_str! 是编译期内嵌）：`);
console.log('  bash desktop/deploy/update-server.sh    # 或在 desktop/src-tauri 下 cargo build');
