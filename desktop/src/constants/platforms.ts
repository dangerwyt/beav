/**
 * 平台统一配置：所有平台值/文案/主页域都从这里取，避免散落的 'xiaohongshu' / 'douyin' 硬编码。
 * 新增平台时只改这里 + 后端 `platform.rs` 的枚举。
 */

export const PLATFORMS = [
  { value: 'bilibili', label: 'B 站' },
  { value: 'xiaohongshu', label: '小红书' },
  { value: 'douyin', label: '抖音' },
  { value: 'weibo', label: '微博' },
  { value: 'zhihu', label: '知乎' },
  { value: 'weixin', label: '微信公众号' },
  { value: 'web', label: '其他网页' },
] as const;

export type Platform = (typeof PLATFORMS)[number]['value'];

export const PLATFORM_LABELS: Record<Platform, string> = {
  bilibili: 'B 站',
  xiaohongshu: '小红书',
  douyin: '抖音',
  weibo: '微博',
  zhihu: '知乎',
  weixin: '微信公众号',
  web: '其他网页',
};

/** 当前走无头浏览器（CDP）登录 / 抓取的平台；新增平台时在此放开。 */
export const CDP_PLATFORMS = ['xiaohongshu', 'douyin', 'bilibili'] as const;
export type CdpPlatform = (typeof CDP_PLATFORMS)[number];

/** CDP 平台的登录主页（Referer / 打开登录页用）。 */
export const PLATFORM_HOME: Record<CdpPlatform, string> = {
  xiaohongshu: 'https://www.xiaohongshu.com',
  douyin: 'https://www.douyin.com',
  bilibili: 'https://www.bilibili.com',
};

export function platformLabel(value: string): string {
  return PLATFORM_LABELS[value as Platform] ?? value;
}
