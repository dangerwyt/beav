#!/usr/bin/env node
/**
 * 校验 Rust 源码里内嵌的 JS 片段语法。
 *
 * 为什么需要：`tools/headless_browser.rs` 把大量 CDP 注入脚本放在 `r#"..."#` raw string 里。
 * Rust 编译器**不校验** raw string 内的 JS 语法 —— 少一个括号、多一个逗号照样编译通过，
 * 直到运行时 `Runtime.evaluate` 返回 SyntaxError 才暴露，而且错误会被上层吞成「页面未解析到正文」，
 * 极难定位。改完任何注入脚本后跑一次这个脚本。
 *
 * 用法：node scripts/check-embedded-js-syntax.mjs
 */
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const TARGET = join(root, 'src-tauri', 'src', 'tools', 'headless_browser.rs');

// 注入脚本里的占位符，替换成合法 JS 值后再做语法校验
const PLACEHOLDERS = [
  [/__LIMIT__/g, '600'],
  [/__XHS__/g, 'false'],
  [/__DY__/g, 'true'],
  [/__NOTE_ID__/g, "'x'"],
  [/__PLATFORM__/g, "'douyin'"],
];

const source = readFileSync(TARGET, 'utf8');
const blocks = [...source.matchAll(/r#+"([\s\S]*?)"#+/g)];

let checked = 0;
let failed = 0;

for (const match of blocks) {
  const code = match[1];
  // 只挑真正的 JS 片段：HTML/CSS 模板里不会有箭头函数和 querySelector
  if (!code.includes('=>') && !code.includes('querySelector')) continue;

  checked += 1;
  const line = source.slice(0, match.index).split('\n').length;
  let js = code;
  for (const [re, value] of PLACEHOLDERS) js = js.replace(re, value);

  try {
    // new Function 只编译不执行 —— 足够暴露语法错误
    // eslint-disable-next-line no-new, no-new-func
    new Function(`return (${js});`);
  } catch (error) {
    failed += 1;
    console.error(`✗ 行 ${line}: ${error.message}`);
  }
}

if (failed > 0) {
  console.error(`\n内嵌 JS 语法检查失败：${failed}/${checked} 块有错`);
  process.exit(1);
}
console.log(`✓ 内嵌 JS 语法检查通过（${checked} 块）`);
