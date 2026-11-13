import test from 'node:test';
import assert from 'node:assert/strict';
import { frameRepairs, repairFrames } from './repair-monitor-frames.mjs';

const contaminated = (postId) => ({ postId, frames: ['/frames/frame_001.jpg', '/frames/._frame_001.jpg'] });
const liveStore = (...records) => ({ monitoredPosts: records.map(({ postId }) => ({ id: postId })), postVideoAnalyses: records });

test('AppleDouble pairs inflate 49 real frames to 98 entries; diagnostics recover real order', () => {
  const frames = Array.from({ length: 49 }, (_, index) => String(index + 1).padStart(3, '0'))
    .reverse().flatMap((number) => [`/mini bak/frames/frame_${number}.jpg`, `/mini bak/frames/._frame_${number}.jpg`]);
  const { repairs: [repair] } = frameRepairs(liveStore({ postId: 'post-1', frames }));
  assert.equal(repair.before, 98);
  assert.equal(repair.metadata, 49);
  assert.equal(repair.frames.length, 49);
  assert.ok(repair.frames[0].endsWith('frame_001.jpg'));
  assert.ok(repair.frames.at(-1).endsWith('frame_049.jpg'));
  assert.equal(frames.length, 98); // Read-only detection.
});

test('numeric sort, duplicate removal and post selection match backend', () => {
  const store = liveStore(
    { postId: 'good', frames: ['/frames/frame_001.jpg'] },
    { postId: 'bad', frames: ['/frames/frame_1000.jpg', '/frames/frame_999.jpg', '/frames/frame_999.jpg', '/frames/._frame_999.jpg'] },
  );
  assert.equal(frameRepairs(store, 'good').repairs.length, 0);
  const { repairs: [repair] } = frameRepairs(store, 'bad');
  assert.deepEqual(repair.frames, ['/frames/frame_999.jpg', '/frames/frame_1000.jpg']);
  assert.equal(frameRepairs(store).repairs.length, 1);
});

test('pure reordering and duplicates do not trigger repairs; orphaned contaminated records are counted and preserved', async () => {
  const store = liveStore({ postId: 'reordered', frames: ['/frames/frame_002.jpg', '/frames/frame_001.jpg', '/frames/frame_001.jpg'] });
  store.postVideoAnalyses.unshift(contaminated('deleted'));
  const before = JSON.stringify(store);
  assert.deepEqual(frameRepairs(store), { repairs: [], orphaned: 1 });
  assert.deepEqual(frameRepairs(store, 'reordered'), { repairs: [], orphaned: 0 });
  const result = await repairFrames(store, { apply: true }, () => assert.fail('No IPC for orphaned or uncontaminated records'), () => {});
  assert.deepEqual(result, { repaired: 0, skipped: 1 });
  assert.equal(JSON.stringify(store), before);
});

test('read-only mode reports contamination without invoking IPC or changing the store', async () => {
  const store = liveStore(contaminated('live'));
  const before = JSON.stringify(store);
  const logs = [];
  await repairFrames(store, {}, () => assert.fail('Read-only mode must not invoke IPC'), (line) => logs.push(line));
  assert.ok(logs.some((line) => line.includes('2 → 1 张真实帧；._ 元数据 1 个')));
  assert.equal(JSON.stringify(store), before);
});

test('a post deleted after the snapshot is skipped and the next post is repaired through export only', async () => {
  const store = liveStore(contaminated('removed-analysis'), contaminated('removed'), contaminated('live'));
  store.postVideoAnalyses.unshift(contaminated('already-deleted'));
  const calls = [];
  const logs = [];
  const result = await repairFrames(store, { apply: true }, async (channel, payload) => {
    calls.push([channel, payload]);
    if (payload.id === 'removed-analysis') return { analysis: null };
    if (channel === 'monitor:video-analysis') return { analysis: { frames: ['/frames/frame_001.jpg'] } };
    if (payload.postId === 'removed') throw new Error('动态不存在');
    return { success: true };
  }, (line) => logs.push(line));
  assert.deepEqual(calls, [
    ['monitor:video-analysis', { id: 'removed-analysis' }],
    ['monitor:video-analysis', { id: 'removed' }], ['monitor:export-post', { postId: 'removed' }],
    ['monitor:video-analysis', { id: 'live' }], ['monitor:export-post', { postId: 'live' }],
  ]);
  assert.deepEqual(result, { repaired: 1, skipped: 3 });
  assert.ok(logs.at(-1).includes('成功 1 条，跳过 3 条'));
});

test('an outdated or changed analysis stops before export; records without real frames stop before IPC', async () => {
  const record = contaminated('live');
  const calls = [];
  await assert.rejects(repairFrames(liveStore(record), { apply: true }, async (channel) => {
    calls.push(channel);
    return { analysis: { frames: record.frames } };
  }, () => {}), /服务尚未更新/);
  assert.deepEqual(calls, ['monitor:video-analysis']);
  await assert.rejects(repairFrames(liveStore({ postId: 'empty', frames: ['/frames/._frame_001.jpg'] }),
    { apply: true }, () => assert.fail('No real frames; must not invoke IPC'), () => {}), /没有真实帧/);
});

test('unexpected export errors are not masked as missing posts and halt the batch', async () => {
  const calls = [];
  await assert.rejects(repairFrames(liveStore(contaminated('first'), contaminated('second')), { apply: true }, async (channel, payload) => {
    calls.push([channel, payload]);
    if (channel === 'monitor:video-analysis') return { analysis: { frames: ['/frames/frame_001.jpg'] } };
    throw new Error('外接盘不可用');
  }, () => {}), /外接盘不可用/);
  assert.equal(calls.length, 2);
  assert.equal(calls[1][1].postId, 'first');
});
