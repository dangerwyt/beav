import { convertFileSrc } from '@tauri-apps/api/core';
import { extractLocalAssetPathCandidate, isLocalAssetSource } from '../../shared/localAsset';
import { browserFileUrl, isTauriRuntime } from '../bridge/browserHost';

const SAFE_RENDERABLE_PROTOCOL = /^(https?:|data:|blob:|file:)/i;
const IMAGE_FILE_HINT = /\.(png|jpe?g|webp|gif|bmp|svg|avif)(?:[?#].*)?$/i;

function toFileUrl(pathValue: string): string {
    const normalized = String(pathValue || '').trim().replace(/\\/g, '/');
    if (!normalized) return '';
    if (/^[a-zA-Z]:\//.test(normalized)) {
        return `file:///${encodeURI(normalized)}`;
    }
    return `file://${encodeURI(normalized)}`;
}

function toTauriAssetUrl(value: string): string {
    const candidate = extractLocalAssetPathCandidate(value);
    if (!candidate) return '';
    // Web 版（非 Tauri 环境）：本机路径交给同源 /api/file 代理读取，
    // 否则浏览器会拦截 file:// / 无法访问服务器磁盘。
    if (!isTauriRuntime()) return browserFileUrl(candidate);
    try {
        return convertFileSrc(candidate);
    } catch {
        return toFileUrl(candidate);
    }
}

export function resolveAssetUrl(value: string | null | undefined): string {
    const raw = String(value || '').trim();
    if (!raw) return '';
    if (/^(https?:|data:|blob:)/i.test(raw)) return raw;
    if (isLocalAssetSource(raw)) return toTauriAssetUrl(raw) || raw;
    if (SAFE_RENDERABLE_PROTOCOL.test(raw)) return raw;
    return raw;
}

export function hasRenderableAssetUrl(value: string | null | undefined): boolean {
    const raw = String(value || '').trim();
    if (!raw) return false;
    if (/^javascript:/i.test(raw)) return false;
    if (SAFE_RENDERABLE_PROTOCOL.test(raw)) return true;
    if (isLocalAssetSource(raw)) return true;
    return IMAGE_FILE_HINT.test(raw);
}

export function isLocalAssetUrl(value: string | null | undefined): boolean {
    return isLocalAssetSource(String(value || '').trim());
}
