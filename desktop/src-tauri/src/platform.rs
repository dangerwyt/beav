//! 平台枚举：把散落在各处的 `"xiaohongshu"` / `"douyin"` 等字面量收敛到一处。
//!
//! 存储层仍用 `String`（兼容旧数据），逻辑判断统一走 `Platform::parse()`。
//! 新增平台（如 Facebook / Twitter）时，只在这里加变体 + 能力开关，调用方不用再改字符串。

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Xiaohongshu,
    Douyin,
    Bilibili,
    Weibo,
    Zhihu,
    Weixin,
    Facebook,
    Twitter,
    Web,
}

impl Platform {
    pub const ALL: &'static [Platform] = &[
        Platform::Xiaohongshu,
        Platform::Douyin,
        Platform::Bilibili,
        Platform::Weibo,
        Platform::Zhihu,
        Platform::Weixin,
        Platform::Facebook,
        Platform::Twitter,
        Platform::Web,
    ];

    /// 规范化后的存储值（与历史 JSON 完全一致，不要随意改）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Platform::Xiaohongshu => "xiaohongshu",
            Platform::Douyin => "douyin",
            Platform::Bilibili => "bilibili",
            Platform::Weibo => "weibo",
            Platform::Zhihu => "zhihu",
            Platform::Weixin => "weixin",
            Platform::Facebook => "facebook",
            Platform::Twitter => "twitter",
            Platform::Web => "web",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "xiaohongshu" | "xhs" | "redbook" => Some(Platform::Xiaohongshu),
            "douyin" | "dy" => Some(Platform::Douyin),
            "bilibili" | "bili" => Some(Platform::Bilibili),
            "weibo" | "wb" => Some(Platform::Weibo),
            "zhihu" => Some(Platform::Zhihu),
            "weixin" | "wechat" => Some(Platform::Weixin),
            "facebook" | "fb" => Some(Platform::Facebook),
            "twitter" | "x" => Some(Platform::Twitter),
            "web" => Some(Platform::Web),
            _ => None,
        }
    }

    /// 从 URL（或 host）推断平台。
    pub fn from_host(raw: &str) -> Option<Self> {
        let s = raw.to_ascii_lowercase();
        if s.contains("xiaohongshu") || s.contains("xhslink") {
            Some(Platform::Xiaohongshu)
        } else if s.contains("douyin") {
            Some(Platform::Douyin)
        } else if s.contains("bilibili") || s.contains("b23.tv") {
            Some(Platform::Bilibili)
        } else if s.contains("weibo") {
            Some(Platform::Weibo)
        } else if s.contains("zhihu") {
            Some(Platform::Zhihu)
        } else if s.contains("weixin") || s.contains("wechat") {
            Some(Platform::Weixin)
        } else if s.contains("facebook") || s.contains("fb.com") {
            Some(Platform::Facebook)
        } else if s.contains("twitter") || s.contains("x.com") {
            Some(Platform::Twitter)
        } else {
            None
        }
    }

    /// 当前由无头浏览器（CDP）采集的站。新增站时在这里开闸。
    pub const fn is_cdp(self) -> bool {
        matches!(self, Platform::Xiaohongshu | Platform::Douyin)
    }

    /// 是否支持「下载视频 → 抽帧 → 语音转写」这一套。
    pub const fn supports_video(self) -> bool {
        matches!(self, Platform::Xiaohongshu | Platform::Douyin)
    }

    /// 登录 / Referer 用的站点主域。
    pub const fn home_url(self) -> &'static str {
        match self {
            Platform::Xiaohongshu => "https://www.xiaohongshu.com",
            Platform::Douyin => "https://www.douyin.com",
            Platform::Bilibili => "https://www.bilibili.com",
            Platform::Weibo => "https://weibo.com",
            Platform::Zhihu => "https://www.zhihu.com",
            Platform::Weixin => "https://channels.weixin.qq.com",
            Platform::Facebook => "https://www.facebook.com",
            Platform::Twitter => "https://x.com",
            Platform::Web => "https://www.google.com",
        }
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::Platform;

    #[test]
    fn parse_roundtrips_all_variants() {
        for platform in Platform::ALL {
            assert_eq!(Platform::parse(platform.as_str()), Some(*platform));
        }
    }

    #[test]
    fn parse_accepts_aliases() {
        assert_eq!(Platform::parse("XHS"), Some(Platform::Xiaohongshu));
        assert_eq!(Platform::parse("dy"), Some(Platform::Douyin));
        assert_eq!(Platform::parse("X"), Some(Platform::Twitter));
        assert_eq!(Platform::parse("fb"), Some(Platform::Facebook));
        assert_eq!(Platform::parse("unknown"), None);
    }

    #[test]
    fn host_detection() {
        assert_eq!(
            Platform::from_host("https://www.xiaohongshu.com/explore/1"),
            Some(Platform::Xiaohongshu)
        );
        assert_eq!(
            Platform::from_host("https://www.douyin.com/video/1"),
            Some(Platform::Douyin)
        );
        assert_eq!(Platform::from_host("https://example.com"), None);
    }

    #[test]
    fn capabilities_are_gated() {
        assert!(Platform::Douyin.is_cdp());
        assert!(!Platform::Facebook.is_cdp());
        assert!(!Platform::Weixin.supports_video());
    }
}
