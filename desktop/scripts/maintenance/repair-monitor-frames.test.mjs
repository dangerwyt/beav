import test from 'node:test';
import assert from 'node:assert/strict';
import { frameRepairs } from './repair-monitor-frames.mjs';

test('AppleDouble pairs inflate 49 real frames to 98 entries; diagnostics recover real order', () => {
  const frames = Array.from({ length: 49 }, (_, index) => String(index + 1).padStart(3, '0'))
    .reverse().flatMap((number) => [`/mini bak/frames/frame_${number}.jpg`, `/mini bak/frames/._frame_${number}.jpg`]);
  const [repair] = frameRepairs({ postVideoAnalyses: [{ postId: 'post-1', frames }] });
  assert.equal(repair.before, 98);
  assert.equal(repair.metadata, 49);
  assert.equal(repair.frames.length, 49);
  assert.ok(repair.frames[0].endsWith('frame_001.jpg'));
  assert.ok(repair.frames.at(-1).endsWith('frame_049.jpg'));
  assert.equal(frames.length, 98); // Read-only detection.
});

test('numeric sort, duplicate removal and post selection match backend', () => {
  const store = { postVideoAnalyses: [
    { postId: 'good', frames: ['/frames/frame_001.jpg'] },
    { postId: 'bad', frames: ['/frames/frame_1000.jpg', '/frames/frame_999.jpg', '/frames/frame_999.jpg', '/frames/._frame_999.jpg'] },
  ] };
  assert.equal(frameRepairs(store, 'good').length, 0);
  const [repair] = frameRepairs(store, 'bad');
  assert.deepEqual(repair.frames, ['/frames/frame_999.jpg', '/frames/frame_1000.jpg']);
  assert.equal(frameRepairs(store).length, 1);
});
