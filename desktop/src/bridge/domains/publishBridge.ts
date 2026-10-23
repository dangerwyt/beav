import type { BridgeCore } from '../types';

export interface PublishPlatform {
  key: string;
  label: string;
  consoleUrl: string;
}

export interface PublishRecord {
  id: string;
  platform: string;
  postUrl?: string | null;
  title: string;
  publishedAt?: string | null;
  status: string; // "planned" | "published" | "failed"
  views: number;
  likes: number;
  comments: number;
  collects: number;
  linkedTopicId?: string | null;
  notes?: string | null;
  createdAt: number;
  updatedAt: number;
}

export interface PublishAddPayload {
  platform: string;
  postUrl?: string;
  title: string;
  publishedAt?: string;
  status?: string;
  views?: number;
  likes?: number;
  comments?: number;
  collects?: number;
  linkedTopicId?: string;
  notes?: string;
}

export interface PublishUpdatePayload {
  id: string;
  platform?: string;
  postUrl?: string;
  title?: string;
  publishedAt?: string;
  status?: string;
  views?: number;
  likes?: number;
  comments?: number;
  collects?: number;
  linkedTopicId?: string;
  notes?: string;
}

const TIMEOUT_MS = 15_000;

export function createPublishBridge(core: BridgeCore) {
  return {
    publish: {
      list: <T = { success: boolean; items: PublishRecord[] }>() =>
        core.invokeChannelGuarded<T>('publish:list', {}, {
          timeoutMs: TIMEOUT_MS,
          fallback: () => ({ success: false, items: [] }) as unknown as T,
        }),
      add: <T = { success: boolean; item: PublishRecord }>(payload: PublishAddPayload) =>
        core.invokeChannelGuarded<T>('publish:add', payload, {
          timeoutMs: TIMEOUT_MS,
          fallback: () => ({ success: false, item: null }) as unknown as T,
        }),
      update: <T = { success: boolean; item: PublishRecord }>(payload: PublishUpdatePayload) =>
        core.invokeChannelGuarded<T>('publish:update', payload, {
          timeoutMs: TIMEOUT_MS,
          fallback: () => ({ success: false, item: null }) as unknown as T,
        }),
      remove: <T = { success: boolean; removed: number }>(id: string) =>
        core.invokeChannelGuarded<T>('publish:remove', { id }, {
          timeoutMs: TIMEOUT_MS,
          fallback: () => ({ success: false, removed: 0 }) as unknown as T,
        }),
      platforms: <T = PublishPlatform[]>() =>
        core.invokeChannelGuarded<T>('publish:platforms', {}, {
          timeoutMs: TIMEOUT_MS,
          fallback: () => [] as unknown as T,
        }),
    },
  };
}
