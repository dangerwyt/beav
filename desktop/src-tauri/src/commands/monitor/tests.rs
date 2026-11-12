use super::{
    clip_chars, detail_author_may_override, extract_four_digit_year, format_hms,
    published_before_cutoff, resolve_watch_id, srt_cues, srt_outline, watch_dedupe_key,
};

#[test]
fn monitor_access_checks_aliases_batches_shares_and_deletes() {
    use super::*;
    struct ResetOwner;
    impl Drop for ResetOwner { fn drop(&mut self) { crate::identity::clear_request_owner(); } }
    let _reset = ResetOwner;
    let mut store = AppStore::default();
    store.competitor_watchlist.push(crate::CompetitorWatchRecord {
        id: "shared-watch".into(), owner: "bob".into(), shared_to: vec!["alice".into()],
        ..Default::default()
    });
    for (id, owner, watch) in [("own", "alice", None), ("private", "bob", None), ("shared", "bob", Some("shared-watch"))] {
        store.monitored_posts.push(MonitoredPostRecord {
            id: id.into(), owner: owner.into(), watch_id: watch.map(str::to_string), ..Default::default()
        });
    }
    crate::identity::set_request_owner(Some("alice".into()));
    assert!(crate::identity::require_admin().is_err());
    for key in ["id", "postId", "post_id"] {
        assert!(guard_store_target(&store, "monitor:remix-generate", &json!({key: "private"})).is_err());
        assert!(guard_store_target(&store, "monitor:remix-generate", &json!({key: "own"})).is_ok());
    }
    assert!(guard_store_target(&store, "monitor:material-files", &json!({"ids": ["own", "private"]})).is_err());
    assert!(guard_store_target(&store, "monitor:material-files", &json!({"ids": ["own", "shared"]})).is_ok());
    assert!(guard_store_target(&store, "monitor:material-files", &json!({"ids": [123]})).is_err());
    assert!(guard_store_target(&store, "monitor:post-remove", &json!({"id": "shared"})).is_err());
    assert!(guard_store_target(&store, "monitor:export-post", &json!({"postId": "missing"})).is_err());
    let visible: Vec<_> = store.monitored_posts.iter().filter(|p| post_accessible(&store, p)).map(|p| p.id.as_str()).collect();
    assert_eq!(visible, vec!["own", "shared"]);
    crate::identity::clear_request_owner();
    assert!(crate::identity::require_admin().is_ok());
    assert!(guard_store_target(&store, "monitor:post-remove", &json!({"id": "private"})).is_ok());
}

#[test]
fn history_backfill_without_id_only_selects_own_or_shared_watches() {
    use super::*;
    struct ResetOwner;
    impl Drop for ResetOwner { fn drop(&mut self) { crate::identity::clear_request_owner(); } }
    let _reset = ResetOwner;
    let mut store = AppStore::default();
    for (id, owner, shared) in [("own", "alice", false), ("shared", "bob", true), ("private", "bob", false)] {
        store.competitor_watchlist.push(crate::CompetitorWatchRecord {
            id: id.into(), owner: owner.into(), enabled: true,
            shared_to: if shared { vec!["alice".into()] } else { vec![] }, ..Default::default()
        });
    }
    crate::identity::set_request_owner(Some("alice".into()));
    let ids: Vec<_> = history_backfill_watches(&store, None).into_iter().map(|watch| watch.id).collect();
    assert_eq!(ids, ["own", "shared"]);
    assert!(history_backfill_watches(&store, Some("private")).is_empty());
    assert_eq!(history_backfill_watches(&store, Some("shared")).len(), 1);
    crate::identity::clear_request_owner();
    assert_eq!(history_backfill_watches(&store, None).len(), 3);
}

#[test]
fn failed_transcription_keeps_previous_content_and_records_error() {
    use super::*;
    let mut record = PostVideoAnalysisRecord { transcript: "旧正文".into(), transcript_srt: "旧字幕".into(), transcript_status: "ok".into(), ..Default::default() };
    apply_transcription_result(&mut record, "", "failed", Some("timeout".into()), "");
    assert_eq!(record.transcript, "旧正文");
    assert_eq!(record.transcript_srt, "旧字幕");
    assert_eq!(record.transcript_error.as_deref(), Some("timeout"));
    apply_transcription_result(&mut record, "新正文", "ok", None, "新字幕");
    assert_eq!(record.transcript, "新正文");
    assert_eq!(record.transcript_srt, "新字幕");
    assert!(record.transcript_error.is_none());
}

#[test]
fn export_write_errors_include_the_target_path() {
    use super::*;
    let root = std::env::temp_dir().join(make_id("beav-export-test"));
    std::fs::create_dir_all(&root).unwrap();
    let target = root.join("missing-parent").join("knowledge.md");
    let error = write_export_file(&target, "content").unwrap_err();
    assert!(error.contains("knowledge.md"));
    assert!(copy_if_changed(&root.join("missing-source"), &root.join("target")).is_err());
    std::fs::remove_dir(&root).unwrap();
}

#[test]
fn external_video_export_preserves_notes_and_rejects_a_disconnected_drive() {
    use super::*;
    let root = std::env::temp_dir().join(make_id("beav-video-export-test"));
    let exports = root.join("exports");
    let l4 = exports.join("platform/date/author/title");
    let disk = root.join("mini bak").join("导出视频");
    std::fs::create_dir_all(&l4).unwrap();
    std::fs::create_dir_all(&disk).unwrap();
    let name = "视频.mp4";
    assert_eq!(exported_video_path(&exports, &l4, "post-1", name).unwrap(), l4.join(name));
    assert_eq!(export_video_embed(&exports, &l4, "post-1", name, "hub/").unwrap(), "![[hub/视频.mp4]]");
    let link = exports.join("导出视频");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&disk, &link).unwrap();
    #[cfg(windows)]
    assert!(Command::new("cmd").args(["/C", "mklink", "/J"])
        .arg(&link).arg(&disk).output().unwrap().status.success());
    assert_eq!(exported_video_path(&exports, &l4, "post-1", name).unwrap(), link.join("post-1").join(name));
    assert_eq!(export_video_embed(&exports, &l4, "post-1", name, "hub/").unwrap(), "![[导出视频/post-1/视频.mp4]]");
    assert!(exported_video_path(&exports, &l4, "../escape", name).is_err());
    std::fs::write(l4.join("knowledge.md"), "note").unwrap();
    std::fs::write(l4.join("图片_1.jpg"), "image").unwrap();
    std::fs::remove_dir(&disk).unwrap();
    assert!(exported_video_path(&exports, &l4, "post-1", name).is_err());
    assert!(l4.join("knowledge.md").is_file());
    assert!(l4.join("图片_1.jpg").is_file());
    assert!(!disk.exists());
    #[cfg(unix)]
    std::fs::remove_file(&link).unwrap();
    #[cfg(windows)]
    std::fs::remove_dir(&link).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

/// 拆解输入：SRT 必须能解析出真实时间码，大纲按步长抽样且不越界。
#[test]
fn srt_outline_keeps_real_timecodes() {
    let srt = "1\n00:00:00,000 --> 00:00:03,000\n开场先说结论：这条视频会颠覆你的认知\n\n\
               2\n00:00:03,500 --> 00:00:07,000\n因为它讲的东西和常识相反\n\n\
               3\n00:02:00,000 --> 00:02:04,000\n两分钟处是转折点\n";
    let cues = srt_cues(srt);
    assert_eq!(cues.len(), 3);
    assert_eq!(cues[0].0, 0);
    assert_eq!(cues[2].0, 120);

    // 步长 90s：0s 与 120s 各抽一条，3.5s 那条被步长跳过
    let outline = srt_outline(&cues, 90, 20);
    let lines: Vec<&str> = outline.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("【00:00:00】"));
    assert!(lines[1].starts_with("【00:02:00】"));
}

/// 截断按 Unicode 字符，不能按字节（否则中文会被切成乱码）。
#[test]
fn clip_chars_is_char_based() {
    assert_eq!(clip_chars("一二三四五", 3), "一二三…");
    assert_eq!(clip_chars("  短句  ", 10), "短句");
    assert_eq!(format_hms(3725), "01:02:05");
}

/// 回归守卫：有监控归属的帖子，详情页作者不得覆盖 watch.name。
///
/// 实测事故：同一博主的三条抖音帖子导出成三个目录 ——「@抖音精选 App官方账号」
/// 「@抖音科技」「拿幸·AI启示录」。前两个是详情页兜底链抓到的正文 `@提及`，
/// 因为旧代码在 `post_detail` 里无条件 `item.author = detail.author`。
#[test]
fn detail_author_never_overrides_watch_author() {
    // 有归属 + 已有作者 → 拒绝覆盖（核心回归）
    assert!(!detail_author_may_override(Some("watch-1"), "拿幸·AI启示录"));
    // 有归属但作者为空 → 允许补一个
    assert!(detail_author_may_override(Some("watch-1"), ""));
    assert!(detail_author_may_override(Some("watch-1"), "   "));
    // 无归属（adhoc 临时帖）→ 允许采用详情页作者
    assert!(detail_author_may_override(None, "任意作者"));
    assert!(detail_author_may_override(None, ""));
    // 空 / 空白 watch_id 视同无归属
    assert!(detail_author_may_override(Some(""), "已有作者"));
    assert!(detail_author_may_override(Some("  "), "已有作者"));
}

/// 回归守卫：年份下限判定宁可漏删不可误删。
///
/// 各平台 `published_at` 的实际文案差异很大（ISO 截断 / 中文日期 / 无年份短格式 /
/// 相对日期）。解析不出年份的一律视为「不早于下限」——2026 年的帖子绝不能被删。
#[test]
fn published_before_cutoff_judges_conservatively() {
    // 早于 2026 → 删
    assert!(published_before_cutoff(Some("2024-03-15")));
    assert!(published_before_cutoff(Some("2024年3月15日")));
    assert!(published_before_cutoff(Some("2019-12-31 23:59")));
    // 2026 及以后 → 留
    assert!(!published_before_cutoff(Some("2026-01-01")));
    assert!(!published_before_cutoff(Some("2026-11-01")));
    // 解析不出年份 → 留（相对日期 / 无年份短格式 / 空 / 缺字段）
    assert!(!published_before_cutoff(Some("3天前")));
    assert!(!published_before_cutoff(Some("03-15")));
    assert!(!published_before_cutoff(Some("")));
    assert!(!published_before_cutoff(None));
    // 五位以上数字不是年份（时间戳被误塞进来也不能按前四位判）
    assert!(!published_before_cutoff(Some("1793952000000")));
}

/// 独立四位段才算年份：避开长数字子串和粘连数字。
#[test]
fn extract_four_digit_year_needs_digit_boundaries() {
    assert_eq!(extract_four_digit_year("2024-03-15"), Some(2024));
    assert_eq!(extract_four_digit_year("发布于 2019年12月"), Some(2019));
    assert_eq!(extract_four_digit_year("03-15"), None);
    assert_eq!(extract_four_digit_year("3天前"), None);
    assert_eq!(extract_four_digit_year("1793952000000"), None);
}

/// 回归守卫：已有归属的帖子不能被另一个监控项抢走。
///
/// 实测踩坑：同一博主建两个监控项 / 两个用户监控同一博主时，
/// 落库若无条件覆盖 watch_id，先抓的一方卡片上的「已收录 N 篇」会凭空归零，
/// 看起来就像「抓到了却没存进去」。
#[test]
fn resolve_watch_id_keeps_existing_owner() {
    // 已有归属且原监控项还在 → 保留原归属，不被抢
    assert_eq!(
        resolve_watch_id(Some("watch-a"), Some("watch-b"), true),
        Some("watch-a".to_string())
    );
    // 孤儿记录（无归属）→ 归给当前监控项
    assert_eq!(
        resolve_watch_id(None, Some("watch-b"), false),
        Some("watch-b".to_string())
    );
    // 新记录 → 归给当前监控项
    assert_eq!(resolve_watch_id(None, None, false), None);
    // 已有归属 + 无当前监控项（临时粘贴单帖）→ 仍保留原归属
    assert_eq!(
        resolve_watch_id(Some("watch-a"), None, true),
        Some("watch-a".to_string())
    );
}

/// 回归守卫：原监控项已被删除时，帖子要能被新监控项接管。
///
/// `watch_remove` 不删帖子，所以这些记录会永远挂在不存在的 watch 上 ——
/// 卡片数恒为 0，再抓也永远接不回来。
#[test]
fn resolve_watch_id_reclaims_orphaned_watch() {
    // 原监控项已删 → 归给当前监控项
    assert_eq!(
        resolve_watch_id(Some("deleted-watch"), Some("watch-b"), false),
        Some("watch-b".to_string())
    );
    // 原监控项已删 + 没有当前监控项 → 无归属（不凭空造一个）
    assert_eq!(resolve_watch_id(Some("deleted-watch"), None, false), None);
}

/// 回归守卫：同一博主的判重必须忽略会变的 xsec_token。
///
/// 小红书主页链接每次会话都带不同的 `xsec_token` / `xsec_source`，
/// 带 token 比较会把同一个博主判成两个 → 重复建监控项 → 帖子互相抢归属。
#[test]
fn watch_dedupe_key_ignores_volatile_token() {
    let with_token = watch_dedupe_key(
        "alice",
        "xiaohongshu",
        Some("https://www.xiaohongshu.com/user/profile/abc?xsec_token=TOKEN1&xsec_source=pc_feed"),
    );
    let other_token = watch_dedupe_key(
        "alice",
        "xiaohongshu",
        Some("https://www.xiaohongshu.com/user/profile/abc?xsec_token=TOKEN2&xsec_source=pc_search"),
    );
    assert_eq!(with_token, other_token);
    assert!(with_token.is_some());
}

#[test]
fn watch_dedupe_key_separates_users_and_platforms() {
    let base = watch_dedupe_key("alice", "xiaohongshu", Some("https://x.com/u/abc"));
    // 不同用户 → 不同 key（各人可以监控同一个博主）
    assert_ne!(
        base,
        watch_dedupe_key("bob", "xiaohongshu", Some("https://x.com/u/abc"))
    );
    // 不同平台 → 不同 key
    assert_ne!(
        base,
        watch_dedupe_key("alice", "douyin", Some("https://x.com/u/abc"))
    );
    // 不同博主 → 不同 key
    assert_ne!(
        base,
        watch_dedupe_key("alice", "xiaohongshu", Some("https://x.com/u/xyz"))
    );
    // 无主页 URL → 不判重（返回 None）
    assert!(watch_dedupe_key("alice", "xiaohongshu", None).is_none());
    assert!(watch_dedupe_key("alice", "xiaohongshu", Some("   ")).is_none());
}

/// 回归守卫：导出目录名按标题前 40 个字符取，中文栏目前缀不该把标题「吃光」。
///
/// 实测事故：`【抖音独家】` 占 6 个字符，取前 10 只剩 `Open`，
/// 三条标题完全不同的帖子撞成同一个目录名，只能靠 `_{postId}` 后缀区分，
/// 目录树里一堆 `【抖音独家】Open_mpost-1793...` 根本认不出是哪条。
#[test]
fn export_title_prefix_keeps_chinese_titles_distinct() {
    let titles = [
        "【抖音独家】OpenAI Images 2.5 重磅发布",
        "【抖音独家】OpenAI 为何紧急暂停 Astra 测试？",
        "【抖音独家】OpenAI 宣布与微软分家",
    ];
    let prefixes: Vec<String> = titles
        .iter()
        .map(|t| super::title_export_prefix(t, super::EXPORT_TITLE_PREFIX_LEN))
        .collect();
    for (i, a) in prefixes.iter().enumerate() {
        for b in prefixes.iter().skip(i + 1) {
            assert_ne!(a, b, "标题前缀不该相撞：{a}");
        }
    }
    // 按 Unicode 字符切，不会把中文切坏；40 字足够装下完整短标题（尾部空格被 trim 掉）
    assert_eq!(prefixes[0], "【抖音独家】OpenAI Images 2.5 重磅发布");
    // 长标题严格按 40 字截断（常量被调小时会在这里暴露）
    let long = super::title_export_prefix(&"黄".repeat(60), super::EXPORT_TITLE_PREFIX_LEN);
    assert_eq!(long.chars().count(), super::EXPORT_TITLE_PREFIX_LEN);
    // 空标题回退
    assert_eq!(
        super::title_export_prefix("   ", super::EXPORT_TITLE_PREFIX_LEN),
        "未命名"
    );
}

/// 回归守卫：视频没变过就不要再抽帧。
///
/// 抽帧是 `fps=1/5` 解全片，46 分钟的视频要抽 550 帧；重复点「分析视频」
/// 或转写失败后重试时，本来只想重跑 ASR，却每次都把整片重解一遍。
#[test]
fn frames_reusable_only_when_video_unchanged() {
    // 没有帧 → 必须抽
    assert!(!super::frames_are_reusable(false, Some(2_000), 1_000));
    assert!(!super::frames_are_reusable(false, None, 1_000));
    // 帧比视频下载时间新 → 复用
    assert!(super::frames_are_reusable(true, Some(2_000), 1_000));
    assert!(super::frames_are_reusable(true, Some(1_000), 1_000));
    // 帧比视频旧 → 说明重新下载过，重抽
    assert!(!super::frames_are_reusable(true, Some(999), 1_000));
    // 老数据没有 video_downloaded_at → 有帧就复用
    assert!(super::frames_are_reusable(true, Some(1), 0));
    // 拿不到帧的 mtime → 不敢复用
    assert!(!super::frames_are_reusable(true, None, 1_000));
}

#[test]
fn monitor_frames_exclude_appledouble_and_sort_by_frame_number() {
    use super::*;
    let root = std::env::temp_dir().join(make_id("beav-frame-list-test"));
    std::fs::create_dir_all(&root).unwrap();
    let mut recorded = Vec::new();
    for number in (1..=49).rev() {
        for name in [format!("frame_{number:03}.jpg"), format!("._frame_{number:03}.jpg")] {
            let path = root.join(name);
            std::fs::write(&path, "fixture").unwrap();
            recorded.push(path.display().to_string());
        }
    }
    std::fs::create_dir(root.join("frame_050.jpg")).unwrap();
    std::fs::write(root.join("not-a-frame.jpg"), "fixture").unwrap();
    assert_eq!(recorded.len(), 98);
    let frames = normalized_monitor_frames(&recorded);
    assert_eq!(frames.len(), 49);
    assert!(frames[0].ends_with("frame_001.jpg"));
    assert!(frames[48].ends_with("frame_049.jpg"));
    assert_eq!(read_monitor_frames(&root).unwrap(), frames);
    let large_numbers = vec![root.join("frame_1000.jpg").display().to_string(), root.join("frame_999.jpg").display().to_string()];
    assert!(normalized_monitor_frames(&large_numbers)[0].ends_with("frame_999.jpg"));
    assert!(monitor_frame_number(Path::new("._frame_001.jpg")).is_none());
    assert!(monitor_frame_number(Path::new("frame_000.jpg")).is_none());
    assert!(monitor_frame_number(Path::new("frame_1x.jpg")).is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn repair_frame_exports_renumbers_real_images_and_preserves_surplus_in_backup() {
    use super::*;
    let root = std::env::temp_dir().join(make_id("beav-frame-repair-test"));
    let cache = root.join("cache");
    let exports = root.join("exports");
    std::fs::create_dir_all(&cache).unwrap();
    std::fs::create_dir_all(&exports).unwrap();
    let mut frames = Vec::new();
    for (number, bytes) in [(1, "real-one"), (2, "real-two")] {
        let path = cache.join(format!("frame_{number:03}.jpg"));
        std::fs::write(&path, bytes).unwrap();
        frames.push(path.display().to_string());
    }
    for number in 1..=4 {
        std::fs::write(exports.join(format!("抽帧_{number:03}.jpg")), "bad-data").unwrap();
    }
    std::fs::write(exports.join("图片_1.jpg"), "cover").unwrap();
    export_monitor_frames(&frames, &exports, true).unwrap();
    assert_eq!(std::fs::read_to_string(exports.join("抽帧_001.jpg")).unwrap(), "real-one");
    assert_eq!(std::fs::read_to_string(exports.join("抽帧_002.jpg")).unwrap(), "real-two");
    assert!(!exports.join("抽帧_003.jpg").exists());
    let backup = std::fs::read_dir(&exports).unwrap().flatten().find(|entry| entry.file_name().to_string_lossy().starts_with(".beav-frame-backup-")).unwrap().path();
    assert_eq!(std::fs::read_to_string(backup.join("抽帧_003.jpg")).unwrap(), "bad-data");
    assert!(backup.join("抽帧_004.jpg").exists());
    assert_eq!(std::fs::read_to_string(exports.join("图片_1.jpg")).unwrap(), "cover");
    let mut bundle = Vec::new();
    collect_bundle_files(&exports, &exports, &mut bundle);
    assert!(!bundle.iter().any(|file| file["name"].as_str().unwrap().contains(".beav-frame-backup-")));
    assert_eq!(std::fs::read_to_string(&frames[1]).unwrap(), "real-two");
    assert!(export_monitor_frames(&[], &exports, true).is_err());
    assert!(exports.join("抽帧_001.jpg").exists());
    std::fs::remove_dir_all(root).unwrap();
}

/// 回归守卫：详情抓取时用干净文案纠正列表页的脏标题。
///
/// 实测：抖音列表页 title = 「播放量 + 文案 + 文案重复」，
/// 详情页 desc 是干净的。修不好就只能靠事后清洗脚本回洗，新数据还会持续变脏。
#[test]
fn corrected_title_strips_play_count_prefix_only_when_detail_agrees() {
    // 典型场景：列表脏、详情干净 → 剥掉播放量前缀
    assert_eq!(
        super::corrected_title_from_detail(
            "1.8万日本资产泡沫，哪哪都是泡泡~ #财经 #日本经济",
            "日本资产泡沫，哪哪都是泡泡~ #财经 #日本经济"
        )
        .as_deref(),
        Some("日本资产泡沫，哪哪都是泡泡~ #财经 #日本经济")
    );
    // 详情带集数前缀，剥集数后仍应识别为「数字属于文案」→ 不改
    assert_eq!(
        super::corrected_title_from_detail(
            "30+亿美元！新加坡史上最大洗钱案 #洗钱罪",
            "第8集 | 30+亿美元！新加坡史上最大洗钱案 #洗钱罪"
        ),
        None
    );
    // 标题不以数字开头（小红书/B 站的正常标题）→ 不动
    assert_eq!(
        super::corrected_title_from_detail(
            "10年没涨价的洗衣液，究竟藏了什么猫腻？ #洗衣液",
            "10年没涨价的洗衣液，究竟藏了什么猫腻？ #洗衣液"
        ),
        None
    );
    // 详情里没有这个数字 → 说明它不是播放量前缀，只是标题内容 → 不动
    assert_eq!(
        super::corrected_title_from_detail(
            "4000亿家电一哥，如何转型？ #原来家还可以这样",
            "4000亿家电一哥，如何转型？ #原来家还可以这样"
        ),
        None
    );
    // 剥掉后剩余文案不在详情里 → 判断不可靠 → 不动
    assert_eq!(
        super::corrected_title_from_detail(
            "1.8万完全不相干的文案 #话题",
            "另一篇完全无关的正文内容"
        ),
        None
    );
}

/// 回归守卫：集数前缀剥离。
#[test]
fn strip_episode_prefix_handles_common_forms() {
    assert_eq!(super::strip_episode_prefix("第8集 | 30+亿美元！"), "30+亿美元！");
    assert_eq!(super::strip_episode_prefix("第 3 期｜真实标题"), "真实标题");
    assert_eq!(super::strip_episode_prefix("第8集真实标题"), "真实标题");
    // 不是集数前缀 → 原样返回
    assert_eq!(super::strip_episode_prefix("日本资产泡沫"), "日本资产泡沫");
    assert_eq!(super::strip_episode_prefix("第8"), "第8");
    assert_eq!(super::strip_episode_prefix(""), "");
}

/// 回归守卫：正文覆盖要按「脏/净」而不只是长度。
///
/// 实测事故：抖音坏提取抓出的噪音版 71 字比干净文案 48 字长，
/// 只比长度会让垃圾永久锁在库里，重新抓多少次都不生效。
#[test]
fn content_replace_prefers_clean_over_longer_noisy() {
    let noisy = "在慕尼黑宫殿参加了一场闭门会议... #德国7.0万20492712969举报发布时间：2025-09-19 17:30";
    let clean = "世界500强CFO纯干货分享 零基础看懂财报——利润表#财务 #会计 #财务报表 #职场 #财报";
    // 旧的脏 → 允许用更短的干净版覆盖（核心回归）
    assert!(super::should_replace_content(noisy, clean));
    // 旧的干净、旧更长 → 不覆盖（防止被标题级内容冲掉）
    assert!(!super::should_replace_content(clean, "世界500强CFO"));
    // 旧的干净、新不短 → 覆盖
    assert!(super::should_replace_content(clean, &format!("{clean}补充")));
    // 等长 → 覆盖（视为刷新）
    assert!(super::should_replace_content(clean, clean));
    // 旧为空 → 覆盖
    assert!(super::should_replace_content("", "任意内容"));
}
