//! 平台抓取适配器（PlatformAdapter）。
//!
//! 目标：把「按平台分支」的抓取差异（选择器、登录页、Referer 等）收敛到各自的适配器里，
//! 上层流程只依赖 trait，不再写 `if platform == "douyin"`。
//!
//! 当前实现覆盖小红书 / 抖音；Facebook / Twitter 后续各加一个实现即可，
//! 不用再改 `monitor.rs` 的主流程。

use crate::platform::Platform;

/// 一个平台的抓取差异。所有方法都应是纯常量，便于静态注册。
pub trait PlatformAdapter: Send + Sync {
    fn platform(&self) -> Platform;

    /// 列表页等待「帖子锚点」出现的 CSS 选择器。
    fn list_wait_selector(&self) -> &'static str;

    /// 详情页等待「正文容器」出现的 CSS 选择器（可用逗号列表）。
    fn detail_wait_selector(&self) -> &'static str;

    /// 详情页等待「正文文本真正填充」的选择器（客户端渲染有延迟）。
    fn content_wait_selector(&self) -> &'static str;

    /// 下载/登录用的站点主页（Referer）。
    fn home_url(&self) -> &'static str {
        self.platform().home_url()
    }
}

pub struct XiaohongshuAdapter;
impl PlatformAdapter for XiaohongshuAdapter {
    fn platform(&self) -> Platform {
        Platform::Xiaohongshu
    }
    fn list_wait_selector(&self) -> &'static str {
        "a[href*=\"/explore/\"]"
    }
    fn detail_wait_selector(&self) -> &'static str {
        "#detail-title, #detail-desc, .note-content"
    }
    fn content_wait_selector(&self) -> &'static str {
        "#detail-desc, .note-text, .desc"
    }
}

pub struct DouyinAdapter;
impl PlatformAdapter for DouyinAdapter {
    fn platform(&self) -> Platform {
        Platform::Douyin
    }
    fn list_wait_selector(&self) -> &'static str {
        "a[href*=\"/video/\"]"
    }
    fn detail_wait_selector(&self) -> &'static str {
        "[data-e2e=\"video-desc\"], .video-info-detail"
    }
    fn content_wait_selector(&self) -> &'static str {
        "[data-e2e=\"video-desc\"]"
    }
}

static XIAOHONGSHU: XiaohongshuAdapter = XiaohongshuAdapter;
static DOUYIN: DouyinAdapter = DouyinAdapter;

/// 按枚举取适配器；未接入的平台返回 `None`（调用方走默认/引导）。
pub fn adapter_for(platform: Platform) -> Option<&'static dyn PlatformAdapter> {
    match platform {
        Platform::Xiaohongshu => Some(&XIAOHONGSHU),
        Platform::Douyin => Some(&DOUYIN),
        _ => None,
    }
}

/// 按字符串取适配器（兼容存储层的 `String`）。
pub fn adapter_for_str(raw: &str) -> Option<&'static dyn PlatformAdapter> {
    Platform::parse(raw).and_then(adapter_for)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapters_registered_for_cdp_platforms() {
        assert!(adapter_for(Platform::Xiaohongshu).is_some());
        assert!(adapter_for(Platform::Douyin).is_some());
        assert!(adapter_for(Platform::Facebook).is_none());
    }

    #[test]
    fn douyin_uses_video_selector() {
        let adapter = adapter_for_str("douyin").expect("douyin adapter");
        assert!(adapter.list_wait_selector().contains("/video/"));
        assert!(adapter.content_wait_selector().contains("video-desc"));
    }

    #[test]
    fn xhs_uses_explore_selector() {
        let adapter = adapter_for_str("xiaohongshu").expect("xhs adapter");
        assert!(adapter.list_wait_selector().contains("/explore/"));
    }
}
