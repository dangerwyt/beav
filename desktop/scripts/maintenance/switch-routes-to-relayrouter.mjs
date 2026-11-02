/**
 * 把多模态 scope 从官方源切到 RelayRouter。
 *
 *   node scripts/maintenance/switch-routes-to-relayrouter.mjs --dry-run
 *   node scripts/maintenance/switch-routes-to-relayrouter.mjs --apply
 *
 * ⚠️ 必须先退出应用，否则内存里的旧快照会覆盖本脚本的写入。
 *
 * 只改 settings.ai_model_routes_json 里指定的几个 scope，
 * 其余字段与 chat / transcription 路由原样保留。
 */
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const DEFAULT_STORE = process.platform === 'win32'
  ? path.join(process.env.APPDATA ?? path.join(os.homedir(), 'AppData', 'Roaming'), 'Beav', 'beav-state.json')
  : path.join(os.homedir(), 'Library', 'Application Support', 'Beav', 'beav-state.json');

/** scope -> [目标模型, 理由] */
const TARGETS = {
  embedding: ['text-embedding-3-small', '同名模型，向量空间不变，无需重建索引'],
  image: ['gpt-image-2-c', '官方 gpt-image-2-cheap 的便宜档等价物（已实测通过）'],
  videoAnalysis: ['qwen3.8-omni-flash', '官方 qwen3.5-omni-flash 的升级版'],
  voiceTts: ['gpt-4o-mini-tts', '官方 speech-2.8-turbo 是 MiniMax 模型，RelayRouter 的 OpenAI 适配器不支持其音频端点（HTTP 500），改用 OpenAI TTS（实测返回 audio/mpeg）'],
  visualIndex: ['qwen3-vl-plus', '官方 Beav-1.0 为专有模型，改用通用视觉模型'],
};

/**
 * 明确不切：RelayRouter 上没有对应能力的模型。
 *
 * voiceClone（音色克隆）——两条独立的封锁，缺一不可：
 *   1) RelayRouter 只有 MiniMax-Voice-Design（文字描述"设计"音色）和 kling-custom-voices，
 *      没有 /audio/voices「创建 + 轮询」这套协议，克隆模型本身就无法路由。
 *   2) 更关键：voice_service.rs 的 resolve_voice_config 里 base_url 优先级是
 *      payload → tts_route → clone_route → settings.voice_endpoint，
 *      **tts_route 压过 clone_route**，而 voiceTts 与 voiceClone 共用同一个 gateway。
 *      所以本脚本把 voiceTts 切到 RelayRouter 之后，即使下面 voiceClone 的 route 仍是
 *      mode=official，它实际也会打到 https://api.relayrouter.ai/v1/audio/voices —— 等于失效。
 *
 * 结论：只要 voiceTts 走 RelayRouter，音色克隆就是不可用的；route 里写 official 只是
 * 保留原值以便日后需要时回切，不代表当前可用（数字人只能用内置音色）。
 * 若将来要恢复克隆：把 voiceTts 也切回官方源（且官方源要有 token），两者必须同源。
 */
const SKIP = {
  voiceClone: '无 /audio/voices 克隆协议；且与 voiceTts 共用 gateway 而 tts_route 优先 → 实际不可用',
};

function parseArgs(argv) {
  const out = { apply: false, dryRun: false, store: DEFAULT_STORE };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === '--apply') out.apply = true;
    else if (a === '--dry-run') out.dryRun = true;
    else if (a === '--store') out.store = argv[++i];
  }
  return out;
}

const args = parseArgs(process.argv.slice(2));
if (!args.apply && !args.dryRun) {
  console.error('用法: node scripts/maintenance/switch-routes-to-relayrouter.mjs --dry-run | --apply');
  process.exit(1);
}
if (!fs.existsSync(args.store)) {
  console.error(`store 不存在: ${args.store}`);
  process.exit(1);
}

const store = JSON.parse(fs.readFileSync(args.store, 'utf8'));
store.settings = store.settings ?? {};
const settings = store.settings;

const parseJson = (value) => (typeof value === 'string' ? JSON.parse(value) : value);
const routes = parseJson(settings.ai_model_routes_json) ?? {};
const sources = parseJson(settings.ai_sources_json) ?? [];

const relay = sources.find((s) => String(s?.baseURL ?? '').toLowerCase().includes('relayrouter'));
if (!relay) {
  console.error('ai_sources_json 里找不到 RelayRouter 源');
  process.exit(1);
}
const relayId = relay.id;
console.log(`RelayRouter sourceId = ${relayId}  (${relay.baseURL})`);
console.log('');

const changed = [];
for (const [scope, [model, why]] of Object.entries(TARGETS)) {
  const before = routes[scope] ?? {};
  const after = { mode: 'custom', model, sourceId: relayId };
  if (JSON.stringify(before) === JSON.stringify(after)) {
    console.log(`= ${scope.padEnd(14)} 已是目标值，跳过`);
    continue;
  }
  console.log(`~ ${scope.padEnd(14)} ${before.mode}/${before.model}  ->  custom/${model}`);
  console.log(`  ${''.padEnd(14)} ${why}`);
  routes[scope] = after;
  changed.push(scope);
}

for (const [scope, why] of Object.entries(SKIP)) {
  const cur = routes[scope] ?? {};
  console.log(`! ${scope.padEnd(14)} 保持官方（${cur.mode}/${cur.model}）—— ${why}`);
}

console.log(`\n共 ${changed.length} 项待改: ${changed.join(', ') || '无'}`);

if (args.dryRun) {
  console.log('\n[dry-run] 未写入');
  process.exit(0);
}
if (changed.length === 0) {
  console.log('\n无需写入');
  process.exit(0);
}

const stamp = new Date().toISOString().replace(/[-:T]/g, '').slice(0, 15);
const backup = `${args.store}.bak-${stamp}`;
fs.copyFileSync(args.store, backup);
console.log(`\n已备份 -> ${backup}`);

settings.ai_model_routes_json = JSON.stringify(routes);
fs.writeFileSync(args.store, `${JSON.stringify(store, null, 2)}\n`, 'utf8');
console.log(`已写入 ${args.store}`);

console.log('\n=== 回读校验 ===');
const back = JSON.parse(fs.readFileSync(args.store, 'utf8')).settings.ai_model_routes_json;
const backRoutes = parseJson(back);
for (const scope of [...Object.keys(TARGETS), ...Object.keys(SKIP)]) {
  const r = backRoutes[scope] ?? {};
  console.log(`  ${scope.padEnd(14)} mode=${String(r.mode).padEnd(9)} model=${r.model}`);
}
