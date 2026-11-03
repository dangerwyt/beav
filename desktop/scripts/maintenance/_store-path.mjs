/**
 * 维护脚本共用的 store 路径解析。
 *
 * 复刻 Rust `persistence::build_store_path()` 的选择顺序：
 *   显式 --store → BEAV_STORE_DIR → **legacy RedBox** → Beav
 *
 * 为什么要单开这个模块：各脚本原先各自写死 `Library/Application Support/Beav/beav-state.json`，
 * 从 RedBox 时代迁过来的机器真实数据还在 `RedBox/redbox-state.json`，
 * 而 `Beav/` 下那个是应用后来新建的空文件 —— 脚本会读到空库，报告「0 条」，
 * 看起来像没有待处理数据，实际一条都没读到。
 */
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const BASE_DIR = process.platform === 'win32'
  ? (process.env.APPDATA ?? path.join(os.homedir(), 'AppData', 'Roaming'))
  : path.join(os.homedir(), 'Library', 'Application Support');

/** @returns {{ file: string, from: string }} */
export function resolveStorePath(explicit = '') {
  const given = String(explicit ?? '').trim();
  if (given) return { file: given, from: '--store' };

  const envDir = String(process.env.BEAV_STORE_DIR ?? '').trim();
  if (envDir) return { file: path.join(envDir, 'beav-state.json'), from: 'BEAV_STORE_DIR' };

  const legacy = path.join(BASE_DIR, 'RedBox', 'redbox-state.json');
  if (fs.existsSync(legacy)) return { file: legacy, from: 'legacy RedBox' };

  return { file: path.join(BASE_DIR, 'Beav', 'beav-state.json'), from: 'Beav' };
}

/** 解析后顺手打印来源，方便确认读到的是哪个库。 */
export function describeStorePath(resolved) {
  try {
    const stat = fs.statSync(resolved.file);
    const mb = (stat.size / 1024 / 1024).toFixed(2);
    return `${resolved.file}（来源=${resolved.from} · ${mb} MB · 修改于 ${stat.mtime.toISOString()}）`;
  } catch {
    return `${resolved.file}（来源=${resolved.from} · 不存在）`;
  }
}
