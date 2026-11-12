import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { planMigration, applyPlan, cleanupPlan, copyVerified, rewriteVideoEmbeds, migrationStatus, verifyPlayback } from './migrate-video-storage.mjs';

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'beav-video-migration-'));
  t.after(() => {
    for (const link of [path.join(root, 'exports/导出视频'), path.join(root, 'workspace/monitor-videos')]) {
      if (fs.lstatSync(link, { throwIfNoEntry: false })?.isSymbolicLink()) fs.unlinkSync(link);
    }
    fs.rmSync(root, { recursive: true, force: true });
  });
  const exportsRoot = path.join(root, 'exports'), cache = path.join(root, 'workspace/monitor-videos'), volume = path.join(root, 'mini bak');
  const post = path.join(exportsRoot, '抖音/2026-11-12/作者/标题');
  for (const dir of [post, cache, volume]) fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(post, '.beav-post-id'), 'post-1');
  fs.writeFileSync(path.join(post, '视频.mp4'), 'export-video');
  fs.writeFileSync(path.join(post, 'knowledge.md'), '# 笔记\n![[视频.mp4]]\n![图](图片_1.jpg)');
  fs.writeFileSync(path.join(post, '图片_1.jpg'), 'image');
  fs.writeFileSync(path.join(post, '转写全文.txt'), 'transcript');
  fs.writeFileSync(path.join(cache, 'post-1.mp4'), 'cached-video');
  return { root, post, exportsRoot, cache, volume, plan: planMigration(exportsRoot, cache, volume) };
}

test('only videos and cache move; notes/images/subtitles stay and embeds resolve', async (t) => {
  const { plan, post, cache } = fixture(t);
  assert.equal(plan.files.length, 2);
  assert.equal(fs.lstatSync(cache).isSymbolicLink(), false);
  await applyPlan(plan);
  assert.equal(plan.stage, 'linked');
  assert.equal(fs.readFileSync(path.join(post, '视频.mp4'), 'utf8'), 'export-video');
  assert.match(fs.readFileSync(path.join(post, 'knowledge.md'), 'utf8'), /!\[\[导出视频\/post-1\/视频.mp4\]\]/);
  assert.equal(fs.readFileSync(path.join(cache, 'post-1.mp4'), 'utf8'), 'cached-video');
  assert.equal(fs.readFileSync(path.join(plan.exportsRoot, '导出视频/post-1/视频.mp4'), 'utf8'), 'export-video');
  assert.equal(fs.readFileSync(path.join(plan.videosRoot, 'post-1/.beav-post-id'), 'utf8'), 'post-1');
  await assert.rejects(cleanupPlan(plan), /先完成迁移/);
  plan.stage = 'verified';
  await cleanupPlan(plan);
  assert.equal(fs.existsSync(path.join(post, '视频.mp4')), false);
  assert.equal(fs.existsSync(plan.cacheBackup), false);
  for (const name of ['knowledge.md', '图片_1.jpg', '转写全文.txt']) assert.ok(fs.existsSync(path.join(post, name)));
  await cleanupPlan(plan); // Idempotent cleanup.
});

test('changed external copy prevents deletion of any original', async (t) => {
  const { plan, post } = fixture(t);
  await applyPlan(plan);
  plan.stage = 'verified';
  fs.writeFileSync(plan.files[0].target, 'corrupted');
  await assert.rejects(cleanupPlan(plan), /保留原件/);
  assert.ok(fs.existsSync(path.join(post, '视频.mp4')));
  assert.ok(fs.existsSync(path.join(plan.cacheBackup, 'post-1.mp4')));
});

test('copy conflicts preserve both files and interrupted copy can resume', async (t) => {
  const { plan, post, root } = fixture(t);
  const conflict = path.join(root, 'conflict.mp4');
  fs.writeFileSync(conflict, 'different');
  await assert.rejects(copyVerified(path.join(post, '视频.mp4'), conflict), /拒绝覆盖/);
  assert.equal(fs.readFileSync(conflict, 'utf8'), 'different');
  await assert.rejects(applyPlan(plan, () => { throw new Error('interrupted'); }), /interrupted/);
  assert.ok(fs.existsSync(path.join(post, '视频.mp4')));
  await applyPlan(plan);
  assert.equal(plan.stage, 'linked');
});

test('new cache files after planning cannot disappear during cutover', async (t) => {
  const { plan, cache } = fixture(t);
  fs.writeFileSync(path.join(cache, 'new.mp4'), 'new video');
  await assert.rejects(applyPlan(plan), /清单已变化/);
  assert.equal(fs.lstatSync(cache).isSymbolicLink(), false);
});

test('hub wiki references change without rewriting image/text links; unsupported references stop safely', (t) => {
  const { post, exportsRoot, cache, volume } = fixture(t);
  assert.equal(rewriteVideoEmbeds('![[库/作者/视频.mp4|600]]\n![图](图片.jpg)\n![[爆文拆解]]', 'post-1', ['视频.mp4'], ['库/作者/']),
    '![[导出视频/post-1/视频.mp4|600]]\n![图](图片.jpg)\n![[爆文拆解]]');
  assert.equal(rewriteVideoEmbeds('![[别的帖子/视频.mp4]]', 'post-1', ['视频.mp4']), '![[别的帖子/视频.mp4]]');
  fs.writeFileSync(path.join(post, 'knowledge.md'), '<video src="视频.mp4"></video>');
  assert.throws(() => planMigration(exportsRoot, cache, volume), /非标准视频引用/);
  assert.ok(fs.existsSync(path.join(post, '视频.mp4')));
});

test('unrecorded backup files prevent cleanup', async (t) => {
  const { plan, post } = fixture(t);
  await applyPlan(plan);
  plan.stage = 'verified';
  fs.writeFileSync(path.join(plan.cacheBackup, 'unexpected.txt'), 'keep');
  await assert.rejects(cleanupPlan(plan), /未记录文件/);
  assert.ok(fs.existsSync(path.join(post, '视频.mp4')));
});

async function playbackServer(t, handler) {
  const server = http.createServer(handler);
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  t.after(() => { server.closeAllConnections(); return new Promise((resolve) => server.close(resolve)); });
  return `http://127.0.0.1:${server.address().port}`;
}

test('status distinguishes copied files from playback verification and does not mutate files', async (t) => {
  const { plan } = fixture(t);
  assert.equal(migrationStatus(plan).stage, 'planned');
  assert.equal(migrationStatus(plan).hashed, 0);
  await applyPlan(plan);
  const summary = migrationStatus(plan);
  assert.equal(summary.stage, 'linked');
  assert.match(summary.description, /尚未验证通过/);
  assert.equal(summary.hashed, plan.files.length);
  assert.ok(summary.links.every((link) => link.connected));
  assert.ok(summary.cacheBackupExists);
  plan.services = ['com.beav.desktop'];
  const receipt = path.join(plan.exportsRoot, '.beav-video-migration.json');
  const before = JSON.stringify(plan);
  fs.writeFileSync(receipt, before);
  const script = fileURLToPath(new URL('./migrate-video-storage.mjs', import.meta.url));
  const result = spawnSync(process.execPath, [script, '--exports', plan.exportsRoot, '--volume', path.dirname(plan.videosRoot), '--status'], { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(JSON.parse(result.stdout).stage, 'linked');
  assert.equal(fs.readFileSync(receipt, 'utf8'), before);
  fs.unlinkSync(path.join(plan.exportsRoot, '导出视频'));
  assert.equal(migrationStatus(plan).links[0].connected, false);
});

test('playback timeout leaves migration linked and a retry only reads videos', async (t) => {
  const { plan } = fixture(t);
  await applyPlan(plan);
  const before = JSON.stringify(plan);
  const logs = [];
  let requests = 0, responding = false;
  const daemon = await playbackServer(t, (req, res) => {
    requests++;
    if (!responding) return; // Simulate a daemon that accepts connections but stalls on disk reads.
    assert.equal(req.headers.range, 'bytes=0-15');
    const file = new URL(req.url, 'http://localhost').searchParams.get('path');
    const bytes = fs.readFileSync(file).subarray(0, 16);
    res.writeHead(206, { 'Content-Length': bytes.length }); res.end(bytes);
  });
  await assert.rejects(verifyPlayback(plan, { daemon, timeoutMs: 100, report: (line) => logs.push(line) }), /文件已切换.*导出视频读取验证未通过.*--verify/);
  assert.equal(plan.stage, 'linked');
  assert.equal(JSON.stringify(plan), before);
  assert.ok(logs.some((line) => line.includes('3/3')));
  assert.ok(fs.existsSync(plan.files[0].source));
  assert.ok(fs.existsSync(plan.cacheBackup));
  const failedRequests = requests;
  responding = true;
  await verifyPlayback(plan, { daemon, report: () => {} });
  assert.equal(requests - failedRequests, 2); // One export and one cache sample, no copy pass.
  assert.equal(JSON.stringify(plan), before);
});

test('transient server errors retry, permanent permission failures do not', async (t) => {
  const { plan } = fixture(t);
  await applyPlan(plan);
  let requests = 0, deny = false;
  const daemon = await playbackServer(t, (req, res) => {
    requests++;
    if (deny) { res.writeHead(404); res.end(); return; }
    if (requests === 1) { res.writeHead(503); res.end(); return; }
    const file = new URL(req.url, 'http://localhost').searchParams.get('path');
    res.writeHead(206); res.end(fs.readFileSync(file).subarray(0, 16));
  });
  await verifyPlayback(plan, { daemon, timeoutMs: 100, report: () => {} });
  assert.equal(requests, 3);
  deny = true;
  await assert.rejects(verifyPlayback(plan, { daemon, report: () => {} }), /HTTP 404/);
  assert.equal(requests, 4);
  assert.equal(plan.stage, 'linked');
});
