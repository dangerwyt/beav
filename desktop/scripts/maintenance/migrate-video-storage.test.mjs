import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { planMigration, applyPlan, cleanupPlan, copyVerified, rewriteVideoEmbeds } from './migrate-video-storage.mjs';

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
