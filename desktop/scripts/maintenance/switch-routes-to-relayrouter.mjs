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
import { resolveStorePath } from './_store-path.mjs';

/** scope -> [目标模型, 理由] */
const CHAT_MODEL = 'deepseek-v4-flash-0731';

const TARGETS = {
  // chat 系列：**必须一起切**。服务器上桌面端带 BEAV_SKIP_OFFICIAL_LOGIN=1 启动，
  // 官方源没有 token，留在 official 的话聊天/拆解/知识库全都会 401。
  chat: [CHAT_MODEL, '主对话模型（拆解、摘要都走它）'],
  knowledge: [CHAT_MODEL, '知识库问答'],
  redclaw: [CHAT_MODEL, '内容创作 Agent'],
  team: [CHAT_MODEL, '团队协作模式'],
  wander: [CHAT_MODEL, '自动漫游/探索'],
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
  const out = { apply: false, dryRun: false, ensureSource: false, status: false, store: '' };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === '--apply') out.apply = true;
    else if (a === '--dry-run') out.dryRun = true;
    else if (a === '--ensure-source') out.ensureSource = true;
    else if (a === '--status') out.status = true;
    else if (a === '--store') out.store = argv[++i];
  }
  return out;
}

const args = parseArgs(process.argv.slice(2));
if (!args.apply && !args.dryRun && !args.status) {
  console.error('用法: node scripts/maintenance/switch-routes-to-relayrouter.mjs --status | --dry-run | --apply [--ensure-source] [--store <路径>]');
  console.error('  首次配置：RELAYROUTER_API_KEY=<key> ... --apply --ensure-source');
  process.exit(1);
}
const storeResolved = resolveStorePath(args.store);
args.store = storeResolved.file;
if (!fs.existsSync(args.store)) {
  console.error(`store 不存在: ${args.store}（来源: ${storeResolved.from}）`);
  console.error('若数据在别处，用 --store <路径> 指定，或设置 BEAV_STORE_DIR。');
  process.exit(1);
}
console.log(`store: ${args.store}（来源: ${storeResolved.from}）`);

const store = JSON.parse(fs.readFileSync(args.store, 'utf8'));
store.settings = store.settings ?? {};
const settings = store.settings;

const parseJson = (value) => (typeof value === 'string' ? JSON.parse(value) : value);
const routes = parseJson(settings.ai_model_routes_json) ?? {};
const sources = parseJson(settings.ai_sources_json) ?? [];

/**
 * 投影文件 `model-config.json`（与 store 同目录）。
 *
 * 启动流程（src-tauri/src/startup/mod.rs）会先 `load_model_config_into_settings`：
 * 把 model-config.json 里的 providers / routes **写回 settings**。也就是说这个文件
 * 才是启动时的权威快照 —— 只改 beav-state.json 而不动它，下一次启动就会被它覆盖回去
 * （表现：脚本写完 --status 是 custom，桌面端一启动又变回 official，连源都没了）。
 */
const projectionPath = path.join(path.dirname(args.store), 'model-config.json');

const readProjection = () => {
  if (!fs.existsSync(projectionPath)) return null;
  try {
    return JSON.parse(fs.readFileSync(projectionPath, 'utf8'));
  } catch {
    return null;
  }
};

/**
 * `--status`：只读体检——把每个 scope 实际会打的「源 / 地址 / 模型 / key 是否配了」打出来。
 *
 * 为什么需要：拆解、聊天、知识库报 `HTTP 401 Authentication required` 时，第一嫌疑是
 * 该 scope 的路由还挂在 official（服务器带 BEAV_SKIP_OFFICIAL_LOGIN=1 启动，官方源没有
 * token → 必然 401），第二嫌疑是源在但 key 空。这两种在界面上都不容易看出来。
 */
if (args.status) {
  console.log(`\n源（${sources.length} 个）：`);
  for (const s of sources) {
    const key = String(s?.apiKey ?? '').trim();
    console.log(
      `  - ${String(s?.id ?? '?').padEnd(18)} ${String(s?.name ?? '').padEnd(14)} ` +
        `${String(s?.baseURL ?? '').padEnd(34)} model=${String(s?.model ?? '-').padEnd(26)} ` +
        `key=${key ? `已配置(${key.length}位, …${key.slice(-4)})` : '❌ 缺失'}`,
    );
  }

  const scopes = [...Object.keys(TARGETS), ...Object.keys(SKIP), 'transcription', 'video'];
  console.log('\n路由：');
  for (const scope of scopes) {
    const r = routes[scope] ?? {};
    const src = sources.find((s) => String(s?.id ?? '') === String(r.sourceId ?? ''));
    const key = String(src?.apiKey ?? '').trim();
    let flag = '';
    if (!r.mode) flag = '（未设置 → 跟随默认源）';
    else if (r.mode !== 'custom') flag = '⚠ 非 custom（official 源在跳过登录时会 401）';
    else if (!src) flag = '⚠ sourceId 指向的源不存在';
    else if (!key) flag = '⚠ 源缺少 API Key';
    console.log(
      `  ${scope.padEnd(15)} mode=${String(r.mode ?? '-').padEnd(9)} ` +
        `model=${String(r.model ?? '-').padEnd(26)} source=${src ? src.name : String(r.sourceId ?? '-')} ${flag}`,
    );
  }
  console.log(`\ndefault_ai_source_id = ${settings.default_ai_source_id ?? '(空)'}`);
  console.log('api_endpoint = ' + String(settings.api_endpoint ?? '(空)'));

  const projection = readProjection();
  if (!projection) {
    console.log('\n投影文件 model-config.json：不存在 → 启动时按 store 重建，改动会保留');
  } else {
    const projectionProviderIds = (projection.providers ?? []).map((item) => item.id).join(', ') || '(空)';
    const projectionChat = projection.routes?.chat ?? {};
    const storeChat = routes.chat ?? {};
    const sameProviders =
      projectionProviderIds === sources.map((item) => item.id).join(', ');
    const sameChat =
      String(projectionChat.mode ?? '') === String(storeChat.mode ?? '') &&
      String(projectionChat.model ?? '') === String(storeChat.model ?? '');
    console.log(
      `\n投影文件 model-config.json：providers=[${projectionProviderIds}] ` +
        `chat=${projectionChat.mode ?? '-'}/${projectionChat.model ?? '-'}`,
    );
    if (!sameProviders || !sameChat) {
      console.log('  ⚠ 与 store 不一致 —— 应用启动时会用这个文件覆盖 settings，脚本的改动会被吃掉');
      console.log('    处理：退出应用后把它改名/删除再启动（--apply 默认会自动改名备份）');
    } else {
      console.log('  与 store 一致');
    }
  }
  process.exit(0);
}

/**
 * `--ensure-source`：没配过 RelayRouter 时顺手把源建出来（需要 RELAYROUTER_API_KEY）。
 *
 * 为什么放在脚本里：服务器上通常只有 SSH，去界面里点「添加供应商」很别扭；
 * 而且切换路由本来就要先有源，两步合成一步少一次踩坑机会。
 * Key 从环境变量读而不是命令行参数，避免留在 shell history / ps 输出里。
 */
let sourcesChanged = false;
if (args.ensureSource) {
  const apiKey = String(process.env.RELAYROUTER_API_KEY ?? '').trim();
  const existing = sources.find((s) => String(s?.baseURL ?? '').toLowerCase().includes('relayrouter'));
  if (!existing) {
    if (!apiKey) {
      console.error('缺少 RELAYROUTER_API_KEY 环境变量（新增源需要 API Key）');
      console.error('用法：RELAYROUTER_API_KEY=<你的key> node scripts/maintenance/switch-routes-to-relayrouter.mjs --apply --ensure-source');
      process.exit(1);
    }
    const created = {
      id: 'relayrouter',
      name: 'RelayRouter',
      baseURL: 'https://api.relayrouter.ai/v1',
      apiKey,
      model: 'deepseek-v4-flash-0731',
    };
    sources.push(created);
    sourcesChanged = true;
    console.log(`将新增 AI 源：${created.name}（${created.baseURL}）`);
  } else if (apiKey && apiKey !== existing.apiKey) {
    existing.apiKey = apiKey;
    sourcesChanged = true;
    console.log('将更新 RelayRouter 的 API Key');
  } else {
    console.log('RelayRouter 源已存在，无需新增');
  }
}

const relay = sources.find((s) => String(s?.baseURL ?? '').toLowerCase().includes('relayrouter'));
if (!relay) {
  console.error('ai_sources_json 里找不到 RelayRouter 源');
  console.error('先执行一次带 --ensure-source 并设置 RELAYROUTER_API_KEY，或在桌面端「设置 → AI 供应商」里添加。');
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
if (sourcesChanged) console.log('另需写入 ai_sources_json（新增/更新 RelayRouter 源）');

if (args.dryRun) {
  console.log('\n[dry-run] 未写入');
  process.exit(0);
}
if (changed.length === 0 && !sourcesChanged) {
  console.log('\n无需写入');
  process.exit(0);
}

const stamp = new Date().toISOString().replace(/[-:T]/g, '').slice(0, 14);
const backup = `${args.store}.bak-${stamp}`;
fs.copyFileSync(args.store, backup);
console.log(`\n已备份 -> ${backup}`);

if (sourcesChanged) settings.ai_sources_json = JSON.stringify(sources);
if (changed.length > 0) settings.ai_model_routes_json = JSON.stringify(routes);
fs.writeFileSync(args.store, `${JSON.stringify(store, null, 2)}\n`, 'utf8');
console.log(`已写入 ${args.store}`);

// 关键一步：改名备份投影文件。
// 启动时 load_model_config_into_settings 会把 model-config.json 的 providers/routes
// 写回 settings —— 留着旧的（official）就等于下次启动自动回滚。
if (fs.existsSync(projectionPath)) {
  const projectionBackup = `${projectionPath}.bak-${stamp}`;
  fs.renameSync(projectionPath, projectionBackup);
  console.log(`已备份并移除投影文件 -> ${projectionBackup}`);
  console.log('（启动时会按新的 settings 重建它；不删的话启动会把路由改回 official）');
}

console.log('\n=== 回读校验 ===');
const back = JSON.parse(fs.readFileSync(args.store, 'utf8')).settings.ai_model_routes_json;
const backRoutes = parseJson(back);
for (const scope of [...Object.keys(TARGETS), ...Object.keys(SKIP)]) {
  const r = backRoutes[scope] ?? {};
  console.log(`  ${scope.padEnd(14)} mode=${String(r.mode).padEnd(9)} model=${r.model}`);
}
