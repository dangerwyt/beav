#!/usr/bin/env node
/**
 * 同步 WorkBuddy 的**工作区记忆**到另一台机器（换机 / 家里 ↔ 公司）。
 *
 * ## 为什么需要这个
 *
 * WorkBuddy 的记忆分三层，只有**工作区记忆**是纯本地文件、不会跟着账号走：
 *
 * | 记忆类型 | 位置 | 跟账号走？ |
 * |---|---|---|
 * | 云端 profile（会话开头注入的） | 服务端 | ✅ 换机自动有 |
 * | 用户级 MEMORY.md / SOUL.md / IDENTITY.md | `~/.workbuddy/user-<uuid>-personal/` | ✅ 云端下发，换机自动有 |
 * | 对话历史 | 服务端 | ✅ |
 * | **工作区记忆** `<workspace>/.workbuddy/memory/` | 本地文件 | ❌ **需要手动同步** |
 *
 * 项目级的技术知识（踩坑、架构约定）应写进 `desktop/docs/`，跟着 git 走 ——
 * 那是**公开仓库**，不要把个人工作日志放进去。
 *
 * ## 用法
 *
 * ```bash
 * # 导出当前工作区记忆 → 私有 git 仓库，并推送
 * node scripts/sync-wb-memory.mjs push
 *
 * # 从私有 git 仓库拉取，合并进当前工作区记忆
 * node scripts/sync-wb-memory.mjs pull
 *
 * # 只看状态（当前工作区 / 仓库 / 待同步文件）
 * node scripts/sync-wb-memory.mjs status
 * ```
 *
 * ## 配置
 *
 * - `WB_MEMORY_REPO`：私有记忆仓库路径，默认 `~/wb-memory`
 * - `WB_WORKSPACE`：显式指定工作区目录（默认自动找最近改动过的那个）
 *
 * ## 合并规则
 *
 * 记忆是**按天追加**的日志，同名文件（`2026-10-30.md`）可能在两台机器上都被改过。
 * 合并时按 **mtime 新者胜** —— 不做三方合并，避免把两边的内容搅在一起。
 * 真要保留两边，请自己 `diff` 后手动处理（有冲突的文件会被打印出来）。
 *
 * ⚠️ 只同步 `memory/` 目录，**不**同步工作区里的代码、配置与密钥。
 */
import { execFileSync } from 'node:child_process';
import {
  existsSync,
  readdirSync,
  statSync,
  readFileSync,
  writeFileSync,
  copyFileSync,
  mkdirSync,
} from 'node:fs';
import { join, resolve, basename } from 'node:path';
import { homedir } from 'node:os';

const MODE = (process.argv[2] || 'status').toLowerCase();
const REPO_DIR = resolve(
  process.env.WB_MEMORY_REPO || join(homedir(), 'wb-memory'),
);
const WORKSPACE_ROOT = join(homedir(), 'WorkBuddy');

function log(...args) {
  console.log(...args);
}

function fail(message) {
  console.error(`✗ ${message}`);
  process.exit(1);
}

function run(cmd, args, cwd) {
  return execFileSync(cmd, args, {
    cwd,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
}

/** 找到当前工作区：优先环境变量，否则取记忆文件最多的那个。 */
function resolveWorkspace() {
  if (process.env.WB_WORKSPACE) {
    const dir = resolve(process.env.WB_WORKSPACE);
    if (!existsSync(join(dir, '.workbuddy', 'memory'))) {
      fail(`指定的工作区没有 .workbuddy/memory：${dir}`);
    }
    return dir;
  }
  if (!existsSync(WORKSPACE_ROOT)) {
    fail(`找不到工作区根目录：${WORKSPACE_ROOT}（可用 WB_WORKSPACE 指定）`);
  }
  const candidates = [];
  for (const entry of readdirSync(WORKSPACE_ROOT, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const memoryDir = join(WORKSPACE_ROOT, entry.name, '.workbuddy', 'memory');
    if (!existsSync(memoryDir)) continue;
    candidates.push({ dir: join(WORKSPACE_ROOT, entry.name), count: listFiles(memoryDir).length });
  }
  if (!candidates.length) {
    fail(`${WORKSPACE_ROOT} 下没有任何含 .workbuddy/memory 的工作区`);
  }
  // 按**记忆文件数**选，而不是「最近改动过」——
  // 后者会选到刚建的新工作区（里面只有当天一份），把真正的积累漏掉。
  candidates.sort((a, b) => b.count - a.count);
  if (candidates.length > 1) {
    log(`发现 ${candidates.length} 个工作区，按记忆文件数选取：`);
    for (const item of candidates) {
      log(`   ${item.count} 个文件  ${item.dir}`);
    }
  }
  return candidates[0].dir;
}

function memoryDirOf(workspaceDir) {
  return join(workspaceDir, '.workbuddy', 'memory');
}

function listFiles(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir, { withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => entry.name)
    .sort();
}

function ensureRepo() {
  if (!existsSync(join(REPO_DIR, '.git'))) {
    log(`初始化记忆仓库：${REPO_DIR}`);
    mkdirSync(REPO_DIR, { recursive: true });
    run('git', ['init'], REPO_DIR);
    run('git', ['remote', 'add', 'origin', 'https://github.com/dangerwyt/wb-memory.git'], REPO_DIR);
  }
  const remotes = run('git', ['remote'], REPO_DIR);
  if (!remotes.includes('origin')) {
    fail(`记忆仓库没有 origin 远端：${REPO_DIR}\n请先 git remote add origin <你的私有仓库地址>`);
  }
}

function cmdStatus() {
  const workspace = resolveWorkspace();
  const memoryDir = memoryDirOf(workspace);
  log(`工作区      : ${workspace}`);
  log(`记忆目录    : ${memoryDir}`);
  log(`记忆仓库    : ${REPO_DIR}${existsSync(join(REPO_DIR, '.git')) ? '' : '（尚未初始化）'}`);
  const local = listFiles(memoryDir);
  const remote = listFiles(join(REPO_DIR, 'memory'));
  log(`本地文件    : ${local.length} 个${local.length ? `（${local.join(', ')}）` : ''}`);
  log(`仓库文件    : ${remote.length} 个${remote.length ? `（${remote.join(', ')}）` : ''}`);
  const onlyLocal = local.filter((name) => !remote.includes(name));
  const onlyRemote = remote.filter((name) => !local.includes(name));
  if (onlyLocal.length) log(`仅本地      : ${onlyLocal.join(', ')}`);
  if (onlyRemote.length) log(`仅仓库      : ${onlyRemote.join(', ')}`);
  if (!onlyLocal.length && !onlyRemote.length) log('文件清单一致（内容差异需 push/pull 才会同步）');
}

function cmdPush() {
  const workspace = resolveWorkspace();
  const memoryDir = memoryDirOf(workspace);
  ensureRepo();
  const target = join(REPO_DIR, 'memory');
  mkdirSync(target, { recursive: true });
  const files = listFiles(memoryDir);
  if (!files.length) {
    log('工作区记忆目录为空，无需推送');
    return;
  }
  for (const name of files) {
    copyFileSync(join(memoryDir, name), join(target, name));
  }
  run('git', ['add', 'memory'], REPO_DIR);
  const status = run('git', ['status', '--porcelain'], REPO_DIR);
  if (!status) {
    log('记忆仓库无变化，无需推送');
    return;
  }
  const stamp = new Date().toISOString().replace(/[:.]/g, '-');
  run('git', ['commit', '-m', `chore(memory): 同步工作区记忆 ${stamp}`], REPO_DIR);
  log(`已提交 ${files.length} 个记忆文件，推送到 origin…`);
  try {
    run('git', ['push', 'origin', 'HEAD'], REPO_DIR);
    log('✓ 推送完成');
  } catch (error) {
    const detail = error.stderr || error.message;
    fail(`推送失败（请检查网络/凭据）：${String(detail).trim()}`);
  }
}

function cmdPull() {
  const workspace = resolveWorkspace();
  const memoryDir = memoryDirOf(workspace);
  ensureRepo();
  log('拉取记忆仓库…');
  try {
    run('git', ['pull', '--ff-only', 'origin', 'HEAD'], REPO_DIR);
  } catch (error) {
    const detail = error.stderr || error.message;
    fail(`拉取失败：${String(detail).trim()}`);
  }
  const source = join(REPO_DIR, 'memory');
  const files = listFiles(source);
  if (!files.length) {
    log('记忆仓库没有文件，跳过');
    return;
  }
  mkdirSync(memoryDir, { recursive: true });
  const conflicts = [];
  let copied = 0;
  for (const name of files) {
    const incoming = join(source, name);
    const local = join(memoryDir, name);
    if (existsSync(local)) {
      const localMtime = statSync(local).mtimeMs;
      const incomingMtime = statSync(incoming).mtimeMs;
      if (localMtime >= incomingMtime) {
        log(`跳过 ${name}（本地更新）`);
        continue;
      }
      conflicts.push(name);
    }
    copyFileSync(incoming, local);
    copied += 1;
  }
  log(`✓ 合并完成：写入 ${copied} 个文件到 ${memoryDir}`);
  if (conflicts.length) {
    log(`⚠ 以下文件本地与仓库都改过，已按 mtime 取较新的一份，如需保留两边请手动 diff：`);
    for (const name of conflicts) log(`   - ${name}`);
  }
}

if (MODE === 'push') {
  cmdPush();
} else if (MODE === 'pull') {
  cmdPull();
} else if (MODE === 'status') {
  cmdStatus();
} else {
  fail(`未知命令：${MODE}（可用：status / push / pull）`);
}
