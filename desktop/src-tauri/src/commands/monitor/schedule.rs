use super::*;

/// 读取某帖的视频分析沉淀（字幕 + 抽帧 + 摘要）。
pub(super) fn video_analysis(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    let rec = with_store(state, |store| {
        Ok(store.post_video_analyses.iter().find(|a| a.post_id == id).cloned())
    })?;
    Ok(json!({
        "success": true,
        "analysis": rec.map(|mut r| {
            r.frames = normalized_monitor_frames(&r.frames);
            serde_json::to_value(&r).unwrap_or_else(|_| json!({}))
        }),
    }))
}

/// 每日 00:00 批量预取的**增量窗口下限**。
///
/// 「单次抓取条数」默认只有 12，那是给手动点「抓取」用的单次窗口。每日预取要覆盖
/// 一整天的增量，博主单日爆更一旦超过窗口大小，主页里排在窗口之外的帖子
/// 既不在今天的抓取范围、昨天也没抓到 → 永久漏抓。所以预取窗口取
/// 「用户设置值」与该下限的**较大者**，并在汇总里回显实际使用的 limit。
pub(super) const DAILY_PREFETCH_MIN_LIMIT: usize = 30;

/// 每日预取**存量回填**的条数上限（每个博主每次）。
///
/// 预取只补新帖的话，历史上「从没打开过」的帖永远没有正文（lazy 抓取的遗留），
/// 用户点开仍要现场抓 → 不满足「点开直接看到正文」。这里每晚额外回填一批存量，
/// 逐日消化 backlog。回填对象 = 从没抓过详情（`detail_fetched == false` 且正文为空）的帖，
/// 按时间**倒序**（新的优先，用户更可能先看最近的）。
pub(super) const DAILY_PREFETCH_BACKFILL_LIMIT: usize = 10;

/// 「补历史」标记的版本前缀。
///
/// v1（有 bug 的版本）在**深翻一条都没抓到**时也会打标记 —— 例如该平台 Chrome 当时没连上，
/// `fetch_monitor` 提前返回 `scanned: 0`。这会把一次临时故障固化成**永久**的「已补过」，
/// 之后无论手动点还是每晚自动跑都不会再补。
///
/// 改成版本化标记：v1 打的标记不带前缀 → 一律视为**无效**（自动补历史会重试），
/// v2 之后只在真抓到内容时才打标记（带 `v2:` 前缀）。
pub(super) const HISTORY_BACKFILL_FLAG_VERSION: &str = "v2";

/// 判断「已补历史」标记是否有效（只有 v2 之后、且真的抓到了内容的才算）。
pub(super) fn history_backfill_valid(flag: Option<&str>) -> bool {
    flag.map(|value| value.starts_with(&format!("{HISTORY_BACKFILL_FLAG_VERSION}:")))
        .unwrap_or(false)
}

/// 「补历史」统一上限：主页深翻最大条数 + 单次补正文条数（两者共用）。
///
/// 曾用两个独立常量（旧的 `HISTORY_BACKFILL_LIMIT` / `HISTORY_BACKFILL_DETAIL_LIMIT`，各 200），
/// 导致「滚动能翻出 300 条、detail 只补 200 条」的错位。现合并为单一「近期窗口」上限：
/// 滚动最多抓 `BACKFILL_MAX` 条，本次抓到的帖**全部**补正文，再用存量未抓正文的旧帖补满到
/// `BACKFILL_MAX`。1000+ 帖的博主默认只覆盖最新 500 条（旧帖不抓）；要全量把此值调大即可
/// （每条详情要开一个 CDP tab，耗时线性增加）。
pub(super) const BACKFILL_MAX: usize = 500;

/// 补历史的年份下限：只收录 `BACKFILL_YEAR_MIN` 年（含）以后发布的动态，
/// 更早的历史帖在补完正文、拿到发布日期后直接删掉（连同互动快照）。
pub(super) const BACKFILL_YEAR_MIN: i32 = 2026;

/// 判断 `published_at` 是否早于补历史年份下限。
///
/// 日期文案随平台而异：`2024-03-15`（小红书 ISO 截断）/ `2024年3月15日` / `03-15`（无年份）
/// / `3天前`（相对日期）。规则取保守侧：
/// - 能解析出**四位年份**且 < 下限 → 早于下限（要删）；
/// - 解析不出年份（无年份短格式、相对日期通常都表示近期）→ 视为不早于下限，
///   宁可多留，也不能因为解析失败把 2026 年的帖子误删。
pub(super) fn published_before_cutoff(published_at: Option<&str>) -> bool {
    let Some(text) = published_at.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    extract_four_digit_year(text).is_some_and(|year| year < BACKFILL_YEAR_MIN)
}

/// 从文本里取出第一段**独立**的四位年份数字（前后都不是数字才算边界）。
///
/// 只取独立四位段是为了避开两种误配：五位以上数字（时间戳 `1793952000000`）
/// 和被夹在更长数字里的子串。2026 年以后平台若改文案，这里只会退化为
/// 「解析不出年份 → 保留」，不会反向误删。
pub(super) fn extract_four_digit_year(text: &str) -> Option<i32> {
    let chars: Vec<char> = text.chars().collect();
    // 短于 4 个字符不可能含四位年份。必须先行返回：`len - 4` 在 len < 4 时会下溢，
    // 写成 `0..=len.saturating_sub(4)` 则退化成 `0..=0`，随后 `chars[0..4]` 直接 panic
    // （"3天前" 这种相对日期正好 3 个字符，是最常见的踩点）。
    if chars.len() < 4 {
        return None;
    }
    for i in 0..=(chars.len() - 4) {
        let window = &chars[i..i + 4];
        if !window.iter().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let left_ok = i == 0 || !chars[i - 1].is_ascii_digit();
        let right_ok = i + 4 >= chars.len() || !chars[i + 4].is_ascii_digit();
        if left_ok && right_ok {
            let year: String = window.iter().collect();
            return year.parse().ok();
        }
    }
    None
}

/// 补历史时并行处理视频的 worker 数。
///
/// 取流、下载、转写都是长任务，串行处理上百条视频耗时很久。多 worker 并行按倍数省时间，
/// 每个 worker 各占 1 个 CDP tab + 1 路 ffmpeg。3 路是实测拐点：再往上并发打开
/// douyin/xhs 视频页容易触发风控 / 取流超时，而磁盘与网络已经接近饱和。
/// 调大更快，但建议保持在 1~4。
pub(super) const VIDEO_PARALLEL_WORKERS: usize = 3;

/// 监控抓取（每日预取 / 补历史）的统计累加器。
///
/// 把「新帖补全」与「存量回填」两条路径的计数合并到一处。
#[derive(Default)]
pub(super) struct MonitorFetchStats {
    watched: i64,
    new_posts: i64,
    detail_ok: i64,
    detail_skipped: i64,
    skip_note: Option<String>,
    videos: i64,
    transcribed: i64,
    failed: i64,
    /// 回收的僵尸「downloading」视频数（上次进程被杀留下的）
    recovered_downloading: i64,
    /// 本轮回填的存量帖数
    backfilled: i64,
    /// 本次因发布年份早于 `BACKFILL_YEAR_MIN` 被丢弃的历史帖数
    dropped_old_posts: i64,
    /// 待处理的历史视频帖数（历史视频不自动下载，交给用户手动）
    pending_videos: i64,
    /// 因已有「已补历史」标记而被跳过的博主数
    already_backfilled: i64,
    /// 服务器该平台未登录而跳过的博主数（游客视角只能拿到首屏）
    not_logged_in: i64,
    /// 触到深翻上限（BACKFILL_MAX=500）、历史可能还有更多的博主数
    limit_reached: i64,
}

impl MonitorFetchStats {
    /// 补全单帖的正文 / 媒体类型，返回是否**真的落库**。
    ///
    /// 注意：`post_detail` 在「Chrome 未连接 / 页面未解析」时**也返回 Ok**（原因在 note 里），
    /// 所以不能拿 Ok 当成功——必须回读记录看 content / detail_fetched 是否真的落库。
    /// 否则统计会说「已补全」而实际正文还是空的（用户点开看不到正文，且无从排查）。
    fn enrich_detail(&mut self, state: &State<'_, AppState>, id: &str) -> bool {
        let mut detail_note: Option<String> = None;
        match post_detail(state, &json!({ "id": id, "refresh": false })) {
            Ok(result) => {
                detail_note = result
                    .get("note")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(String::from);
            }
            Err(error) => {
                self.failed += 1;
                crate::logging::emit_legacy_line(
                    crate::logging::event::LogSource::Host,
                    crate::logging::event::LogLevel::Warn,
                    "monitor",
                    "monitor.daily.detail_failed",
                    format!("[monitor daily] detail fetch for {id} failed: {error}"),
                    json!({ "postId": id, "error": error }),
                    None,
                );
            }
        }
        let enriched = with_store(state, |store| {
            Ok(store
                .monitored_posts
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.detail_fetched || !p.content.trim().is_empty())
                .unwrap_or(false))
        })
        .unwrap_or(false);
        if enriched {
            self.detail_ok += 1;
        } else {
            self.detail_skipped += 1;
            if self.skip_note.is_none() {
                self.skip_note = detail_note;
            }
        }
        enriched
    }
}

/// 回收僵尸「downloading」视频状态。
///
/// `download_video` 开始时把状态置为 `downloading`，成功才改 `done`、失败改 `failed`。
/// 若进程在下载途中被杀（重启 / 崩溃），状态会**永远卡在 downloading**。
/// 而自动下载的守卫是「仅 `none` 才下」→ 这条视频将再也不会被自动重试，永久卡死。
/// 每日预取是后台单线程串行执行，不会有并发下载，所以这里无条件把残留的
/// `downloading` 复位成 `none`，让本轮重新下载。
pub(super) fn recover_stuck_downloading(state: &State<'_, AppState>) -> Result<i64, String> {
    with_store_mut(state, |store| {
        let mut recovered = 0i64;
        for post in store.monitored_posts.iter_mut() {
            if post.video_status == "downloading" {
                post.video_status = "none".to_string();
                recovered += 1;
            }
        }
        Ok(recovered)
    })
}

/// 每日 00:00 批量预取：列表抓取（按 URL 去重，仅新增）→ 逐条补全正文/媒体类型 →
/// 视频帖自动下载 + 语音转写，保证用户次日 09:00 打开即见全文、视频已转写。
///
/// 设计要点：
/// - 复用 `fetch_monitor`（URL 去重，仅回传新帖），ad-hoc 逻辑完全不动；
/// - `post_detail` 先把 `detail_fetched` / `content` / `media_type` 落库，二次打开秒开；
/// - 额外**回填存量**：从没抓过详情的旧帖也补上（否则点开仍要现场抓）；
/// - 仅当 `media_type == "video"` 且平台为小红书 / 抖音时，才 `download_video` + `analyze_video`；
/// - 视频仅在该帖 `video_status == "none"` 时自动下（幂等：已下过 / 失败过不重复下）；
/// - 每条帖独立 try，单帖失败不阻断其余；全程在常驻调度器的 `spawn_blocking` 内同步执行。
pub fn run_daily_prefetch(app: AppHandle) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let now = crate::now_i64();
    // 顺手清理上轮残留的孤儿 SQLite WAL/回滚副文件（贴合「每日」节奏，保证垃圾不积累）。
    let _ = crate::sqlite_cleanup::cleanup_orphaned_sqlite_sidecars(&app);
    let mut stats = MonitorFetchStats::default();

    // 先回收上次残留的僵尸 downloading 状态，否则那批视频永远不会被自动重试
    stats.recovered_downloading = recover_stuck_downloading(&state)?;

    let watches: Vec<crate::CompetitorWatchRecord> = with_store(&state, |store| {
        Ok(store
            .competitor_watchlist
            .iter()
            .filter(|watch| watch.enabled)
            .cloned()
            .collect())
    })?;

    // —— 每日任务前置：按平台自动拉起 Chrome ——
    // 平台账号是共享的，cookie 存在各自 profile 里，凌晨不需要人扫码；但「用户前一天
    // 把 Chrome 关掉」才是常态。不主动拉起的话整轮会是：列表抓不到 → newPosts=0 →
    // 正文/视频全 0，界面上只看到一堆 0，真相只留在一条 warn 日志里（2026-11-03 那轮
    // 就是这么静默失败的）。这里先补齐，拉不起来的平台记进 summary.chromeMissing。
    let mut chrome_missing: Vec<String> = Vec::new();
    {
        // 只处理需要 CDP 的平台（小红书 / 抖音 / B 站），并按平台去重。
        // 注意不能让某个博主缺少 platform 字段就整批跳过，回退到从主页 URL 推断。
        let mut platforms: Vec<String> = Vec::new();
        for watch in &watches {
            let platform = if watch.platform.trim().is_empty() {
                watch
                    .profile_url
                    .as_deref()
                    .map(platform_of)
                    .unwrap_or_default()
            } else {
                watch.platform.clone()
            };
            let is_cdp = Platform::parse(&platform)
                .map(|parsed| parsed.is_cdp())
                .unwrap_or(false);
            if is_cdp && !platforms.iter().any(|known| known == &platform) {
                platforms.push(platform);
            }
        }
        for platform in platforms {
            // 会话还活着就直接复用（launch 内部也会复用，这里先判掉省掉 8s 等待）。
            if headless_browser::chrome_session_alive(&platform) {
                continue;
            }
            let Some(chrome_path) = headless_browser::detect_chrome_path() else {
                chrome_missing.push(platform);
                continue;
            };
            // 用 home_url 而不是 login_url：绝大多数时候 profile 里的登录态还在，
            // 直接开首页就能抓；真掉登录才需要人去「登录管理」重新扫码。
            let landing = headless_browser::qr_platform_spec(&platform)
                .map(|spec| spec.home_url)
                .unwrap_or("about:blank");
            let launched = headless_browser::profile_dir_for(&app, &platform).and_then(|profile| {
                headless_browser::launch_chrome_for_login(
                    &chrome_path,
                    &profile,
                    &platform,
                    landing,
                )
            });
            match launched {
                Ok(port) => {
                    println!("[monitor daily] auto-launched chrome for {platform} on port {port}");
                    crate::logging::emit_legacy_line(
                        crate::logging::event::LogSource::Host,
                        crate::logging::event::LogLevel::Info,
                        "monitor",
                        "monitor.daily.chrome_launched",
                        format!("[monitor daily] auto-launched chrome for {platform}: port {port}"),
                        json!({ "platform": platform, "port": port }),
                        None,
                    );
                }
                Err(error) => {
                    chrome_missing.push(platform.clone());
                    crate::logging::emit_legacy_line(
                        crate::logging::event::LogSource::Host,
                        crate::logging::event::LogLevel::Warn,
                        "monitor",
                        "monitor.daily.chrome_launch_failed",
                        format!(
                            "[monitor daily] auto-launch chrome for {platform} failed: {error}"
                        ),
                        json!({ "platform": platform, "error": error }),
                        None,
                    );
                }
            }
        }
    }

    // 每日增量窗口：取「用户设置的单次抓取条数」与下限的较大者
    let daily_limit = resolve_scan_limit(&state, &json!({})).max(DAILY_PREFETCH_MIN_LIMIT);

    // —— 自动补历史（仅一次）——
    // 从没补过历史的博主（首次接入 / 老数据）先深翻一次主页，把更早的历史帖一次性落库。
    // 补过之后打上 `history_backfilled_at` 标记，后续每晚不再重复深翻（深翻要滚 ~35s，不能天天做）。
    let pending_history: Vec<String> = watches
        .iter()
        .filter(|watch| watch.history_backfilled_at.is_none())
        .map(|watch| watch.id.clone())
        .collect();
    for watch_id in pending_history {
        match run_history_backfill(
            app.clone(),
            &json!({ "watchId": watch_id, "quiet": true }),
        ) {
            Ok(result) => {
                let added = result.get("newPosts").and_then(Value::as_i64).unwrap_or(0);
                let enriched = result.get("detailEnriched").and_then(Value::as_i64).unwrap_or(0);
                if added > 0 {
                    println!(
                        "[monitor daily] history backfill for {watch_id}: {added} posts, {enriched} enriched"
                    );
                }
            }
            Err(error) => {
                stats.failed += 1;
                crate::logging::emit_legacy_line(
                    crate::logging::event::LogSource::Host,
                    crate::logging::event::LogLevel::Warn,
                    "monitor",
                    "monitor.daily.history_failed",
                    format!("[monitor daily] history backfill failed: {error}"),
                    json!({ "watchId": watch_id, "error": error }),
                    None,
                );
            }
        }
    }

    // 视频候选先攒着，等所有博主都处理完再统一跑两阶段流水线（见下方 run_videos_two_phase）。
    // 原来是在「博主 → 帖子」的双重循环里「下载一条 → 转写一条」串行做，
    // 10 条视频就要串 10 轮，每轮都含 下载(~30s) + 抽帧 + 转写(~20s)。
    let mut pending_videos: Vec<String> = Vec::new();

    for watch in &watches {
        stats.watched += 1;
        let fetch = match fetch_monitor(
            &state,
            &json!({ "watchId": watch.id, "limit": daily_limit }),
            None,
            false,
        ) {
            Ok(value) => value,
            Err(error) => {
                stats.failed += 1;
                crate::logging::emit_legacy_line(
                    crate::logging::event::LogSource::Host,
                    crate::logging::event::LogLevel::Warn,
                    "monitor",
                    "monitor.daily.fetch_failed",
                    format!(
                        "[monitor daily] list fetch for {} failed: {}",
                        watch.name, error
                    ),
                    json!({ "watchId": watch.id, "error": error }),
                    None,
                );
                continue;
            }
        };
        let created = fetch
            .get("created")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        stats.new_posts += created.len() as i64;

        // 1) 新帖：补全正文 / 媒体类型（让 09:00 打开秒开）
        let mut targets: Vec<String> = Vec::new();
        for post in &created {
            if let Some(id) = post.get("id").and_then(Value::as_str) {
                targets.push(id.to_string());
            }
        }

        // 2) 存量回填：历史上从没抓过详情的旧帖（lazy 抓取遗留），点开仍要现场抓。
        //    只回填小红书/抖音（只有这两个平台支持本地抓正文），按时间倒序取前 N 条。
        let backlog: Vec<String> = with_store(&state, |store| {
            let mut pending: Vec<(i64, String)> = store
                .monitored_posts
                .iter()
                .filter(|p| p.watch_id.as_deref() == Some(watch.id.as_str()))
                .filter(|p| Platform::parse(&p.platform).map(|x| x.is_cdp()).unwrap_or(false))
                .filter(|p| !p.detail_fetched && p.content.trim().is_empty())
                .map(|p| {
                    (
                        p.created_at.parse::<i64>().unwrap_or(0),
                        p.id.clone(),
                    )
                })
                .collect();
            // 新的优先（用户更可能先看最近的）
            pending.sort_by(|a, b| b.0.cmp(&a.0));
            Ok(pending
                .into_iter()
                .take(DAILY_PREFETCH_BACKFILL_LIMIT)
                .map(|(_, id)| id)
                .collect())
        })?;
        stats.backfilled += backlog.len() as i64;
        for id in backlog {
            if !targets.contains(&id) {
                targets.push(id);
            }
        }

        for id in targets {
            let enriched = stats.enrich_detail(&state, &id);
            // 正文没抓到就别折腾视频了（大概率是 Chrome 没连上）
            if !enriched {
                continue;
            }

            // 3) 只登记视频候选（仅小红书 / 抖音，且尚未下载过）——
            //    真正的下载 + 转写在所有博主处理完后由 run_videos_two_phase 统一并行跑。
            let rec = with_store(&state, |store| {
                Ok(store.monitored_posts.iter().find(|p| p.id == id).cloned())
            })?;
            let (is_video, platform_ok, status_none) = rec
                .as_ref()
                .map(|r| {
                    (
                        r.media_type == "video" || r.post_url.contains("/video/"),
                        Platform::parse(&r.platform).map(|p| p.is_cdp()).unwrap_or(false),
                        r.video_status == "none",
                    )
                })
                .unwrap_or((false, false, false));

            if is_video && platform_ok && status_none {
                pending_videos.push(id);
            }
        }
    }

    // 3) 视频阶段：两阶段并行（下载 VIDEO_PARALLEL_WORKERS 路 → 转写 TRANSCRIBE_PARALLEL_WORKERS 路）。
    //    与补历史的 spawn_video_workers_ex 同构，区别只在于这里必须**同步等待并拿回计数**
    //    写进 summary，否则前端看到的 videosDownloaded / videosTranscribed 会恒为 0。
    let (videos_done, transcribed_done, video_failed) =
        run_videos_two_phase(&app, pending_videos);
    stats.videos += videos_done;
    stats.transcribed += transcribed_done;
    stats.failed += video_failed;

    let summary = json!({
        "success": true,
        "watched": stats.watched,
        // 实际使用的增量窗口（每博主每次抓取的上限）
        "limit": daily_limit,
        "newPosts": stats.new_posts,
        "detailEnriched": stats.detail_ok,
        // 正文没抓到的条数 + 首个原因（如「Chrome 未连接」），便于一眼定位为什么打开不秒显
        "detailSkipped": stats.detail_skipped,
        "skipNote": stats.skip_note,
        // 本轮回填的存量帖数
        "backfilled": stats.backfilled,
        // 回收的僵尸 downloading 数
        "recoveredDownloading": stats.recovered_downloading,
        "videosDownloaded": stats.videos,
        "videosTranscribed": stats.transcribed,
        "failed": stats.failed,
        // 自动拉起失败的平台（拉不起来 → 该平台整轮抓不到东西，界面要显式提示，
        // 否则用户只看到一排 0，以为「今天没更新」）。
        "chromeMissing": chrome_missing,
        "ranAt": now,
    });
    println!("[monitor daily] prefetch summary: {}", summary);
    // 通知前端：常驻 APP 的帖子列表是内存旧快照，预取补全正文/视频后必须刷新，
    // 否则用户次日打开仍是「未抓正文」状态（openDetail 命中不到本地缓存 → 不秒开）。
    let _ = app.emit("monitor:prefetch-done", summary.clone());
    Ok(summary)
}

/// 历史视频并行处理：把 `video_ids` 分给若干 worker 线程，逐个「下载 → 抽帧/转写」。
///
/// 由 `run_history_backfill` 在**补正文之前**调用，与 detail 抓取重叠进行：
/// 总耗时从 `detail + video` 降到约 `max(detail, video)`。
/// - 每个视频独立 CDP target（`navigate_and_eval_settle` 内部 `create_target`），与 detail 互不干扰；
/// - 每条独立容错：下载失败跳过转写，但进度照常推进（不会卡在某个坏链接）；
/// - 进度用原子计数累加，`fetch_add` 返回值唯一 → `videos-done` 恰好触发一次。
pub(super) fn spawn_video_workers(app: AppHandle, watch_id: String, video_ids: Vec<String>) {
    spawn_video_workers_ex(app, watch_id, video_ids, false)
}

/// 每日预取的视频阶段：**同步**跑两阶段并行流水线并返回统计。
///
/// - 阶段一（`VIDEO_PARALLEL_WORKERS` 路）：下载 → 抽帧。
///   抽帧用 `analyze_video({ skipTranscript: true })`，它会建好 `post_video_analyses`
///   记录并把 `transcript_status` 置为 `pending`，阶段二据此接棒。
/// - 阶段二（`TRANSCRIBE_PARALLEL_WORKERS` 路）：`transcribe_video_only` 只补语音转写。
///
/// 为什么不在下载 worker 里顺手转写：转写是「抽音频 → 上传 → 等 ASR 返回」，单条 10~40s，
/// 串进去会把并行度吃掉（补历史那边已有同样结论）。分阶段后两条流水线互不阻塞。
///
/// 与 `spawn_video_workers_ex` 的区别只有一点：那个是 fire-and-forget（后台补历史用），
/// 这里是**同步等待**——每日预取要把 `videosDownloaded` / `videosTranscribed` / `failed`
/// 写进 summary 返回给前端，异步版本这三个数会恒为 0。
///
/// 返回 `(下载成功数, 转写成功数, 失败数)`；失败数含下载失败、抽帧失败、转写失败。
pub(super) fn run_videos_two_phase(app: &AppHandle, video_ids: Vec<String>) -> (i64, i64, i64) {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::{Arc, Mutex};

    let total = video_ids.len();
    if total == 0 {
        return (0, 0, 0);
    }

    let queue = Arc::new(Mutex::new(video_ids.into_iter().collect::<VecDeque<String>>()));
    let pending = Arc::new(Mutex::new(Vec::<String>::new()));
    let downloaded = Arc::new(AtomicI64::new(0));
    let transcribed = Arc::new(AtomicI64::new(0));
    let failed = Arc::new(AtomicI64::new(0));

    // 阶段一：并行下载 + 抽帧（跳过转写）
    let download_workers = VIDEO_PARALLEL_WORKERS.min(total).max(1);
    std::thread::scope(|scope| {
        for _ in 0..download_workers {
            let app = app.clone();
            let queue = Arc::clone(&queue);
            let pending = Arc::clone(&pending);
            let downloaded = Arc::clone(&downloaded);
            let failed = Arc::clone(&failed);
            scope.spawn(move || {
                let state = app.state::<AppState>();
                loop {
                    // 锁中毒也要取回内层数据：否则一次 panic 会把剩余队列静默吐掉。
                    let next = queue
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .pop_front();
                    let Some(id) = next else { break };

                    match download_video(&app, &state, &json!({ "id": id.clone() })) {
                        // `download_video` 在「平台不支持 / Chrome 未连接」这类前置条件下
                        // 返回的是 `Ok({success:false, note})` 而不是 Err —— 那种情况并没有
                        // 落盘。以前只判 Err 就 `downloaded.fetch_add`，会让 videosDownloaded
                        // 虚高，而 videoLocalPath 仍是空、阶段二逐条失败。这里判 success 字段。
                        Ok(value) => {
                            let ok = value
                                .get("success")
                                .and_then(Value::as_bool)
                                .unwrap_or(true);
                            if !ok {
                                let note = value
                                    .get("note")
                                    .and_then(Value::as_str)
                                    .unwrap_or("未知原因")
                                    .to_string();
                                failed.fetch_add(1, Ordering::Relaxed);
                                crate::logging::emit_legacy_line(
                                    crate::logging::event::LogSource::Host,
                                    crate::logging::event::LogLevel::Warn,
                                    "monitor",
                                    "monitor.daily.download_skipped",
                                    format!("[monitor daily] video download for {id} skipped: {note}"),
                                    json!({ "postId": id, "note": note }),
                                    None,
                                );
                                continue;
                            }
                        }
                        Err(error) => {
                            failed.fetch_add(1, Ordering::Relaxed);
                            crate::logging::emit_legacy_line(
                                crate::logging::event::LogSource::Host,
                                crate::logging::event::LogLevel::Warn,
                                "monitor",
                                "monitor.daily.download_failed",
                                format!("[monitor daily] video download for {id} failed: {error}"),
                                json!({ "postId": id, "error": error }),
                                None,
                            );
                            continue;
                        }
                    }
                    downloaded.fetch_add(1, Ordering::Relaxed);

                    match analyze_video(
                        &app,
                        &state,
                        &json!({ "id": id.clone(), "skipTranscript": true }),
                    ) {
                        Ok(_) => pending
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .push(id),
                        Err(error) => {
                            failed.fetch_add(1, Ordering::Relaxed);
                            crate::logging::emit_legacy_line(
                                crate::logging::event::LogSource::Host,
                                crate::logging::event::LogLevel::Warn,
                                "monitor",
                                "monitor.daily.analyze_failed",
                                format!("[monitor daily] video analyze for {id} failed: {error}"),
                                json!({ "postId": id, "error": error }),
                                None,
                            );
                        }
                    }
                }
            });
        }
    });

    // 阶段二：并行转写（只补转写，不重下不重抽帧）
    let batch: Vec<String> = std::mem::take(
        &mut *pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );
    let batch_total = batch.len();
    if batch_total > 0 {
        let tqueue = Arc::new(Mutex::new(batch.into_iter().collect::<VecDeque<String>>()));
        let transcribe_workers = TRANSCRIBE_PARALLEL_WORKERS.min(batch_total).max(1);
        std::thread::scope(|scope| {
            for _ in 0..transcribe_workers {
                let app = app.clone();
                let tqueue = Arc::clone(&tqueue);
                let transcribed = Arc::clone(&transcribed);
                let failed = Arc::clone(&failed);
                scope.spawn(move || {
                    let state = app.state::<AppState>();
                    loop {
                        let next = tqueue
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .pop_front();
                        let Some(id) = next else { break };
                        match transcribe_video_only(&app, &state, &json!({ "id": id.clone() })) {
                            Ok(_) => {
                                transcribed.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(error) => {
                                failed.fetch_add(1, Ordering::Relaxed);
                                crate::logging::emit_legacy_line(
                                    crate::logging::event::LogSource::Host,
                                    crate::logging::event::LogLevel::Warn,
                                    "monitor",
                                    "monitor.daily.transcribe_failed",
                                    format!("[monitor daily] transcribe for {id} failed: {error}"),
                                    json!({ "postId": id, "error": error }),
                                    None,
                                );
                            }
                        }
                    }
                });
            }
        });
    }

    (
        downloaded.load(Ordering::Relaxed),
        transcribed.load(Ordering::Relaxed),
        failed.load(Ordering::Relaxed),
    )
}

/// 带模式的视频 worker：`patch_only` 为真时只补音轨（视频轨不重下、帧不重抽）。
pub(super) fn spawn_video_workers_ex(
    app: AppHandle,
    watch_id: String,
    video_ids: Vec<String>,
    patch_only: bool,
) {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::{Arc, Mutex};

    let total = video_ids.len() as i64;
    if total <= 0 {
        return;
    }
    let workers = VIDEO_PARALLEL_WORKERS.min(total as usize).max(1);
    let queue = Arc::new(Mutex::new(video_ids.into_iter().collect::<VecDeque<String>>()));
    let done = Arc::new(AtomicI64::new(0));
    // 下载成功（已抽帧）的 id 收集起来，等下载阶段整体结束后再串行补转写。
    // 转写要等 ASR（单条 10~40s），串进下载 worker 会把 3 路并行拖成事实上的 1 路。
    let pending_transcribe = Arc::new(Mutex::new(Vec::<String>::new()));

    // 先推一条 0/total，让前端进度条立刻出现（否则要等第一个视频跑完才有事件）
    let _ = app.emit(
        "monitor:backfill-progress",
        json!({ "watchId": watch_id, "scanned": 0, "total": total, "phase": "videos" }),
    );

    for _ in 0..workers {
        let app = app.clone();
        let wid = watch_id.clone();
        let wid_log = watch_id.clone();
        let queue = Arc::clone(&queue);
        let done = Arc::clone(&done);
        let pending_transcribe = Arc::clone(&pending_transcribe);
        let spawned = std::thread::Builder::new()
            .name("backfill-videos".to_string())
            .spawn(move || {
                let state = app.state::<AppState>();
                loop {
                    let next = queue.lock().ok().and_then(|mut q| q.pop_front());
                    let Some(id) = next else { break };
                    let id_for_analyze = id.clone();
                    if patch_only {
                        // 只补音轨：帧 / 分析记录都沿用已有的，补完直接排队转写。
                        if let Err(error) =
                            download_video(&app, &state, &json!({ "id": id, "patchAudio": true }))
                        {
                            println!("[monitor history] patch audio {id} failed: {error}");
                        } else if let Ok(mut list) = pending_transcribe.lock() {
                            list.push(id_for_analyze);
                        }
                    } else if let Err(error) = download_video(&app, &state, &json!({ "id": id })) {
                        println!("[monitor history] background video download {id} failed: {error}");
                    } else if let Err(error) = analyze_video(
                        &app,
                        &state,
                        &json!({ "id": id_for_analyze, "skipTranscript": true }),
                    ) {
                        println!("[monitor history] background video analyze {id} failed: {error}");
                    } else if let Ok(mut list) = pending_transcribe.lock() {
                        list.push(id_for_analyze);
                    }
                    let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                    let _ = app.emit(
                        "monitor:backfill-progress",
                        json!({ "watchId": wid, "scanned": finished, "total": total, "phase": "videos" }),
                    );
                    if finished >= total {
                        let _ = app.emit(
                            "monitor:backfill-progress",
                            json!({ "watchId": wid, "scanned": total, "total": total, "phase": "videos-done" }),
                        );
                        let _ = app.emit(
                            "monitor:prefetch-done",
                            json!({ "source": "backfill-videos", "watchId": wid, "videos": total }),
                        );
                        println!("[monitor history] background videos finished: {total}");
                        let batch = pending_transcribe
                            .lock()
                            .map(|mut list| std::mem::take(&mut *list))
                            .unwrap_or_default();
                        spawn_transcribe_workers(app.clone(), wid.clone(), batch);
                    }
                }
            });
        if let Err(error) = spawned {
            crate::logging::emit_legacy_line(
                crate::logging::event::LogSource::Host,
                crate::logging::event::LogLevel::Warn,
                "monitor",
                "monitor.history.video_thread_failed",
                format!("[monitor history] spawn video worker failed: {error}"),
                json!({ "watchId": wid_log, "error": error.to_string() }),
                None,
            );
        }
    }
}

/// 转写并行度。ASR 服务普遍有并发限流（fun-asr 一类），2 路是稳妥值。
/// 与下载并行度分开：下载受网络/磁盘约束，转写受服务端限流约束，混在一起调只会顾此失彼。
pub(super) const TRANSCRIBE_PARALLEL_WORKERS: usize = 2;

/// 下载阶段结束后单独跑的语音转写队列。
///
/// 为何后置：转写要「抽音频 → 上传 → 等 ASR 返回」，单条 10~40s。串进下载 worker 会让
/// 每路 worker 在每条视频后多等几十秒，3 路并行被拖成事实上的 1 路。这里等下载全部结束
/// （帧已抽好）再接棒，两条流水线互不阻塞。
pub(super) fn spawn_transcribe_workers(app: AppHandle, watch_id: String, ids: Vec<String>) {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::{Arc, Mutex};

    let total = ids.len() as i64;
    if total <= 0 {
        return;
    }
    let workers = TRANSCRIBE_PARALLEL_WORKERS.min(total as usize).max(1);
    let queue = Arc::new(Mutex::new(ids.into_iter().collect::<VecDeque<String>>()));
    let done = Arc::new(AtomicI64::new(0));

    let _ = app.emit(
        "monitor:backfill-progress",
        json!({ "watchId": watch_id, "scanned": 0, "total": total, "phase": "transcribe" }),
    );

    for _ in 0..workers {
        let app = app.clone();
        let wid = watch_id.clone();
        let queue = Arc::clone(&queue);
        let done = Arc::clone(&done);
        let _ = std::thread::Builder::new()
            .name("backfill-transcribe".to_string())
            .spawn(move || {
                let state = app.state::<AppState>();
                loop {
                    let next = queue.lock().ok().and_then(|mut q| q.pop_front());
                    let Some(id) = next else { break };
                    if let Err(error) = transcribe_video_only(&app, &state, &json!({ "id": id })) {
                        println!("[monitor history] background transcribe {id} failed: {error}");
                    }
                    let finished = done.fetch_add(1, Ordering::Relaxed) + 1;
                    let _ = app.emit(
                        "monitor:backfill-progress",
                        json!({ "watchId": wid, "scanned": finished, "total": total, "phase": "transcribe" }),
                    );
                    if finished >= total {
                        let _ = app.emit(
                            "monitor:prefetch-done",
                            json!({ "source": "backfill-transcribe", "watchId": wid, "transcribed": total }),
                        );
                        println!("[monitor history] background transcribe finished: {total}");
                    }
                }
            });
    }
}

/// 批量补音轨：给「已下载但无音轨」的视频重新取流，**只下音频轨**再合并。
///
/// 为什么单开一个入口：抖音 DASH 只给视频轨，历史下载的 54 条全是无声 mp4。
/// 整体重下要再拉 2GB+ 且每条数分钟；补音轨只多下几十 KB 音频 + 一次取流，
/// 3 路并行下几十条几分钟就能补完。补完自动排队转写。
pub(super) fn patch_audio_batch(app: AppHandle, payload: &Value) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let watch_id = str_field(payload, "watchId").or_else(|| str_field(payload, "watch_id"));
    let ids: Vec<String> = with_store(&state, |store| {
        Ok(store
            .monitored_posts
            .iter()
            .filter(|p| {
                watch_id
                    .as_deref()
                    .map(|w| p.watch_id.as_deref() == Some(w))
                    .unwrap_or(true)
            })
            .filter(|p| p.video_status == "done" && p.video_has_audio == Some(false))
            .map(|p| p.id.clone())
            .collect())
    })?;
    let total = ids.len() as i64;
    if total == 0 {
        return Ok(json!({
            "success": true,
            "queued": 0,
            "note": "没有需要补音轨的视频（都已含音轨，或尚未下载）",
        }));
    }
    spawn_video_workers_ex(app, watch_id.unwrap_or_default(), ids, true);
    Ok(json!({ "success": true, "queued": total }))
}

/// 补历史：把博主主页**更早的历史帖**也抓下来落库。
///
/// 日常抓取只取主页最新的 N 条（增量窗口 30），更早的历史永远进不来 ——
/// 首次接入尤其明显（只拿到最近 30 条），而且随着博主继续发新帖，
/// 那些没抓过的历史帖只会不断滑出窗口，最终永久丢失。
///
/// 这里用大 limit **深翻**主页（滚动加载到底），把能翻到的历史帖一次性落库；
/// 并限量补全一批正文（最近的优先），让最新的一批立刻能秒开。
///
/// - 目标：`payload.watchId` 指定单个博主；缺省则对全部 enabled 博主执行；
/// - `force`：无视 `history_backfilled_at` 标记强制重补（手动按钮用）；
/// - 历史视频：**自动下载 + 抽帧/转写**，且与「补正文」并行（`spawn_video_workers`），
///   进度经 `monitor:backfill-progress`(phase=videos) 推给前端；不阻塞本调用返回。
pub(super) fn history_backfill_watches(store: &AppStore, watch_id: Option<&str>) -> Vec<crate::CompetitorWatchRecord> {
    store.competitor_watchlist.iter()
        .filter(|watch| watch.enabled)
        .filter(|watch| watch_id.is_none() || watch_id == Some(watch.id.as_str()))
        .filter(|watch| crate::identity::can_access(&watch.owner, &watch.shared_to))
        .cloned().collect()
}

pub fn run_history_backfill(app: AppHandle, payload: &Value) -> Result<Value, String> {
    let state = app.state::<AppState>();
    let now = crate::now_i64();
    let watch_id = str_field(payload, "watchId").or_else(|| str_field(payload, "watch_id"));
    let force = payload
        .get("force")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // quiet=true：被每日预取内部调用时不再单独发事件（外层会统一发一次）
    let quiet = payload
        .get("quiet")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let watches: Vec<crate::CompetitorWatchRecord> = with_store(&state, |store| {
        Ok(history_backfill_watches(&store, watch_id.as_deref()))
    })?;

    let mut stats = MonitorFetchStats::default();
    let mut scanned_total = 0i64;
    // 落库诊断累计（抓到的条数 / 已存在 / 域名不符丢弃）
    let mut scraped_total = 0i64;
    let mut updated_total = 0i64;
    let mut dropped_total = 0i64;
    // 实际执行了几次 fetch（>1 说明被重复触发了）
    let mut fetch_runs = 0i64;
    // 主页声明的作品总数（进度条分母）：取各博主中的最大值
    let mut profile_total_max = 0i64;
    // 每个博主的滚动诊断（真机排查「只翻到首屏」用）
    let mut scroll_diags: Vec<(String, Value)> = Vec::new();
    // 每个博主的视频诊断：(名称, 帖总数, mediaType=video 的数, mediaType 缺失的数, 本次待下载数)
    let mut video_diag: Vec<(String, i64, i64, i64, i64)> = Vec::new();

    for watch in &watches {
        // 该平台本次的登录态（写进滚动诊断，便于一次判断「未登录」还是「页面不给更多」）
        let mut login_state: Option<bool> = None;
        // 已补过且不是强制 → 跳过（避免每晚重复深翻）
        if !force && history_backfill_valid(watch.history_backfilled_at.as_deref()) {
            stats.already_backfilled += 1;
            continue;
        }
        stats.watched += 1;

        // 前置检查：服务器该平台未登录时，深翻只能拿到**游客视角的首屏**（通常 3-5 条），
        // 表现和"翻页坏了"一模一样。账号被顶掉/cookie 过期都会走到这里，
        // 所以先说清楚，别让人误判成抓取 bug。
        if let Some(platform) = crate::platform::Platform::parse(&watch.platform) {
            let platform_key = platform.as_str();
            let port = headless_browser::active_chrome_port(platform_key);
            let logged_in = if port == 0 {
                false
            } else {
                match tauri::async_runtime::block_on(headless_browser::ws_url_for_port(port)) {
                    Ok(ws_url) => tauri::async_runtime::block_on(
                        headless_browser::login_logged_in(&ws_url, platform_key),
                    )
                    .unwrap_or(false),
                    Err(_) => false,
                }
            };
            if !logged_in {
                stats.not_logged_in += 1;
                let note = format!(
                    "服务器未登录{}，只拿到游客视角的首屏；请先扫码登录平台账号再补历史",
                    platform.label()
                );
                if stats.skip_note.is_none() {
                    stats.skip_note = Some(note.clone());
                }
                crate::logging::emit_legacy_line(
                    crate::logging::event::LogSource::Host,
                    crate::logging::event::LogLevel::Warn,
                    "monitor",
                    "monitor.history.not_logged_in",
                    format!("[monitor history] {}: {}", watch.name, note),
                    json!({ "watchId": watch.id, "platform": platform_key }),
                    None,
                );
                continue;
            }
            login_state = Some(true);
        }

        // 深翻：limit 拉满，滚动加载直到翻不动
        let result = match fetch_monitor(
            &state,
            &json!({ "watchId": watch.id, "limit": BACKFILL_MAX }),
            Some(app.clone()),
            !quiet,
        ) {
            Ok(value) => value,
            Err(error) => {
                stats.failed += 1;
                crate::logging::emit_legacy_line(
                    crate::logging::event::LogSource::Host,
                    crate::logging::event::LogLevel::Warn,
                    "monitor",
                    "monitor.history.fetch_failed",
                    format!(
                        "[monitor history] deep fetch for {} failed: {}",
                        watch.name, error
                    ),
                    json!({ "watchId": watch.id, "error": error }),
                    None,
                );
                continue;
            }
        };
        fetch_runs += 1;
        let scanned = result.get("scanned").and_then(Value::as_i64).unwrap_or(0);
        scraped_total += result.get("scrapedPosts").and_then(Value::as_i64).unwrap_or(0);
        updated_total += result.get("updatedExisting").and_then(Value::as_i64).unwrap_or(0);
        dropped_total += result.get("droppedDomain").and_then(Value::as_i64).unwrap_or(0);
        let exhausted = result
            .get("historyExhausted")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // 滚动诊断：告诉用户「为什么只翻到 N 条」（首屏就停 / 滚错容器 / 页面不给更多）
        if let Some(diag) = result.get("scrollDiag").filter(|value| !value.is_null()) {
            // 带上登录态：游客视角只能拿到首屏几条，有登录态还停在几条 = 页面不给更多
            let mut diag = diag.clone();
            if let Some(obj) = diag.as_object_mut() {
                obj.insert(
                    "loggedIn".to_string(),
                    json!(login_state.unwrap_or(false)),
                );
            }
            scroll_diags.push((watch.name.clone(), diag.clone()));
            // 落日志：桌面端补历史时也能直接 grep 到，不用靠界面
            crate::logging::emit_legacy_line(
                crate::logging::event::LogSource::Host,
                crate::logging::event::LogLevel::Warn,
                "monitor",
                "monitor.history.scroll_diag",
                format!("[monitor history] {} scroll diag: {}", watch.name, diag),
                json!({ "watchId": watch.id, "diag": diag }),
                None,
            );
        }
        scanned_total += scanned;
        // 主页声明的作品总数（进度条分母）：取各博主中的最大值
        let watch_profile_total = result
            .get("profileTotal")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if watch_profile_total > profile_total_max {
            profile_total_max = watch_profile_total;
        }

        // —— 关键守卫：一条都没抓到就不能打「已补历史」标记 ——
        // 典型场景：该平台 Chrome 没连上（fetch 提前返回 scanned=0）。
        // 若此时照样打标记，这个博主就被**永久**排除在补历史之外，
        // 用户手动点也只会得到「已补过」——一个临时故障变成了永久状态。
        if scanned == 0 {
            stats.failed += 1;
            let note = result
                .get("note")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .unwrap_or("未抓到任何帖子")
                .to_string();
            if stats.skip_note.is_none() {
                stats.skip_note = Some(note.clone());
            }
            crate::logging::emit_legacy_line(
                crate::logging::event::LogSource::Host,
                crate::logging::event::LogLevel::Warn,
                "monitor",
                "monitor.history.empty_result",
                format!(
                    "[monitor history] deep fetch for {} got nothing: {}",
                    watch.name, note
                ),
                json!({ "watchId": watch.id, "note": note }),
                None,
            );
            // 不打标记 → 下一晚仍会自动重试
            continue;
        }

        let created = result
            .get("created")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        // created 顺序 = 主页 DOM 顺序 = 时间倒序（新的在前）
        stats.new_posts += created.len() as i64;

        // 深翻建出来的是**历史帖**，不该带「新抓取」徽标（一次几百条全标「新」会显得很坏）。
        // 只清 is_new 标记，created_at 等原始信息保留。
        let backfilled_ids: Vec<String> = created
            .iter()
            .filter_map(|post| post.get("id").and_then(Value::as_str).map(String::from))
            .collect();
        if !backfilled_ids.is_empty() {
            with_store_mut(&state, |store| {
                for post in store.monitored_posts.iter_mut() {
                    if backfilled_ids.iter().any(|id| id == &post.id) {
                        post.is_new = false;
                    }
                }
                Ok::<(), String>(())
            })?;
        }

        // —— 历史视频：必须在「补正文」之后启动 ——
        //
        // 视频与否由 `media_type` 决定，而 `media_type` 只有**抓详情页**时才会被修正
        // （post_detail 里 detail.mediaType 落库）。列表抓取拿不到媒体类型，
        // 新帖的默认值是 "image"。
        //
        // 原实现把视频筛选放在补正文**之前**（想靠并行省时间），于是筛选时
        // media_type 全是 "image" → 一个视频帖都选不出来 → 「补齐了 47 篇但
        // 一个视频都没下载」。小红书 URL 是 /explore/xxx，兜底条件
        // `post_url.contains("/video/")` 也匹配不上。
        //
        // 每日增量路径（run_daily_prefetch）本来就是「先 enrich_detail 再判视频」，
        // 补历史这里改成同样的顺序：先补正文（顺带把 media_type 修正准），
        // 再挑出视频帖丢给 worker。
        //
        // 补全正文：本次滚出的帖【全部】补正文（不再受固定 200 截断），
        // 再用「存量里还没抓过正文」的旧帖补满到 BACKFILL_MAX（近期窗口 500）。
        let mut detail_targets: Vec<String> = created
            .iter()
            .filter_map(|post| post.get("id").and_then(Value::as_str).map(String::from))
            .collect();
        // 新帖不够填满窗口时，用「存量里还没抓过正文」的补上（同样是最近的优先）。
        // 年份下限同样生效：已知发布日期早于下限的存量帖不再花详情抓取的额度。
        if detail_targets.len() < BACKFILL_MAX {
            let backlog: Vec<String> = with_store(&state, |store| {
                let mut pending: Vec<(i64, String)> = store
                    .monitored_posts
                    .iter()
                    .filter(|p| p.watch_id.as_deref() == Some(watch.id.as_str()))
                    .filter(|p| Platform::parse(&p.platform).map(|x| x.is_cdp()).unwrap_or(false))
                    .filter(|p| !p.detail_fetched && p.content.trim().is_empty())
                    .filter(|p| !published_before_cutoff(p.published_at.as_deref()))
                    .map(|p| (p.created_at.parse::<i64>().unwrap_or(0), p.id.clone()))
                    .collect();
                pending.sort_by(|a, b| b.0.cmp(&a.0));
                Ok(pending
                    .into_iter()
                    .map(|(_, id)| id)
                    .filter(|id| !detail_targets.contains(id))
                    .take(BACKFILL_MAX - detail_targets.len())
                    .collect())
            })?;
            stats.backfilled += backlog.len() as i64;
            detail_targets.extend(backlog);
        }

        for id in &detail_targets {
            stats.enrich_detail(&state, id);
        }

        // —— 年份下限：BACKFILL_YEAR_MIN 年以前发布的历史动态直接丢弃 ——
        //
        // 日期只有补完正文才拿得到（主页列表不带发布时间），所以删除动作放在这里。
        // 两个位置约束：
        // 1. 必须先于「视频挑选」执行，否则下架前的老视频会照常排进下载队列，
        //    用户看到的就是「补完历史，开始下载 2019 年的视频」；
        // 2. 快照随帖一起删（与 post_remove 的级联语义一致），不留给复盘当孤儿。
        let dropped_ids: Vec<String> = with_store_mut(&state, |store| {
            let stale: Vec<String> = store
                .monitored_posts
                .iter()
                .filter(|p| detail_targets.contains(&p.id))
                .filter(|p| published_before_cutoff(p.published_at.as_deref()))
                .map(|p| p.id.clone())
                .collect();
            if !stale.is_empty() {
                store.monitored_posts.retain(|p| !stale.contains(&p.id));
                store.post_snapshots.retain(|s| !stale.contains(&s.post_id));
            }
            Ok(stale)
        })?;
        if !dropped_ids.is_empty() {
            // 计数修正：new_posts 在上文按 created 全量计过，被年份砍掉的要扣回去。
            // （backlog 回填的计数 stats.backfilled 同理不再修正 —— 该数字表示「补了
            // 多少条正文」，正文确实补了，只是落库前被删，统计口径不变。）
            let dropped_created = created
                .iter()
                .filter(|post| {
                    post.get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| dropped_ids.iter().any(|x| x == id))
                })
                .count() as i64;
            stats.new_posts -= dropped_created;
            stats.dropped_old_posts = dropped_ids.len() as i64;
            crate::logging::emit_legacy_line(
                crate::logging::event::LogSource::Host,
                crate::logging::event::LogLevel::Info,
                "monitor",
                "monitor.history.dropped_old",
                format!(
                    "[monitor history] {}: dropped {} posts published before {BACKFILL_YEAR_MIN}",
                    watch.name,
                    dropped_ids.len()
                ),
                json!({ "watchId": watch.id, "dropped": dropped_ids.len(), "minYear": BACKFILL_YEAR_MIN }),
                None,
            );
        }

        // 补完正文后，media_type 已被详情页修正为真实的 "video" / "image"，
        // 这时才能准确挑出视频帖（顺序要求见上面的说明）。
        // 幂等：只处理 video_status=="none" 的帖；取流与详情各开独立 CDP target，互不干扰。
        //
        // 同时统计该博主的媒体类型分布 —— 「补齐了 47 篇但一个视频都没下载」时，
        // 看这行就能立刻分辨是「压根没识别出视频帖」还是「识别出来了但下载失败」。
        let (video_ids, video_total, video_type_video, video_type_missing) = with_store(
            &state,
            |store| {
                let mut ids: Vec<String> = Vec::new();
                let mut total = 0i64;
                let mut typed_video = 0i64;
                let mut missing_type = 0i64;
                for post in store
                    .monitored_posts
                    .iter()
                    .filter(|p| p.watch_id.as_deref() == Some(watch.id.as_str()))
                    .filter(|p| Platform::parse(&p.platform).map(|x| x.is_cdp()).unwrap_or(false))
                {
                    total += 1;
                    if post.media_type.trim().is_empty() {
                        missing_type += 1;
                    } else if post.media_type == "video" {
                        typed_video += 1;
                    }
                    if (post.media_type == "video" || post.post_url.contains("/video/"))
                        && post.video_status == "none"
                    {
                        ids.push(post.id.clone());
                    }
                }
                Ok((ids, total, typed_video, missing_type))
            },
        )?;
        stats.pending_videos += video_ids.len() as i64;
        video_diag.push((watch.name.clone(), video_total, video_type_video, video_type_missing, video_ids.len() as i64));
        if !video_ids.is_empty() {
            spawn_video_workers(app.clone(), watch.id.clone(), video_ids);
        }

        // 打上「已补历史」标记，后续不再每晚重复深翻。
        // 只有上面确认 scanned > 0（真的抓到了）才会走到这里 —— Chrome 没连上时不会打标记。
        with_store_mut(&state, |store| {
            if let Some(slot) = store
                .competitor_watchlist
                .iter_mut()
                .find(|entry| entry.id == watch.id)
            {
                slot.history_backfilled_at = Some(format!("{HISTORY_BACKFILL_FLAG_VERSION}:{now}"));
                slot.updated_at = now.to_string();
            }
            Ok::<(), String>(())
        })?;
        if !exhausted {
            // 触到深翻上限（BACKFILL_MAX=500）而非翻到底 → 这位博主历史可能还有更多，值得再翻
            stats.limit_reached += 1;
        }
    }

    // 汇总滚动诊断 → 一句话提示（让用户不用翻日志就知道「为什么只翻到首屏」）
    let mut hint: Option<String> = None;
    // 提示等级：ok = 正常（含"已抓全"），warn = 需要关注
    let mut hint_level: &str = "ok";
    for (name, diag) in &scroll_diags {
        let scrolled = diag
            .get("scrolled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let counts: Vec<i64> = diag
            .get("counts")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(|v| v.as_i64()).collect::<Vec<i64>>())
            .unwrap_or_default();
        let profile_total = diag
            .get("profileTotal")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let captured = counts.last().copied().unwrap_or(0);
        let first = counts.first().copied().unwrap_or(0);
        let grew = counts.len() >= 2 && captured > first;
        // 抓到的数量达到主页声明的作品数 → 已经抓全，不该报警
        let reached_total = profile_total > 0 && captured >= profile_total;
        // 主页声明数解析不到时（小红书新版文案可能不是"47 笔记"），
        // 用滚动位置判断是否真的到底 —— 到底且不再增长就是抓全了。
        let reached_bottom = diag
            .get("reachedBottom")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let eval_errors = diag
            .get("evalErrors")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let all_done = reached_total || (profile_total == 0 && reached_bottom);

        let text = if eval_errors > 0 && !reached_total {
            // 求值失败 = 页面连接断过（标签被关/CDP 断开），结果可能不完整，
            // 绝不能说成"已抓全"。
            format!("{name}：抓取过程中页面连接中断 {eval_errors} 次（标签可能被关闭），本次结果可能不完整")
        } else if reached_total {
            format!("{name}：已抓全 {captured} 篇（与主页声明的 {profile_total} 篇一致）")
        } else if all_done {
            format!("{name}：已翻到主页底部，共抓到 {captured} 篇")
        } else if !scrolled {
            format!("{name}：页面没有发生滚动（滚动容器可能选错）")
        } else if !grew {
            format!(
                "{name}：滚动后没有加载出更多内容（停在 {captured} 条，共 {} 轮）",
                counts.len()
            )
        } else if profile_total > 0 {
            format!("{name}：抓到 {captured} 篇，主页声明 {profile_total} 篇（可能还有未加载的）")
        } else {
            format!("{name}：滚动加载正常（{first} → {captured} 条）")
        };
        if !all_done || eval_errors > 0 {
            hint_level = "warn";
        }
        if hint.is_none() {
            hint = Some(text);
        }
    }

    // 库内实况：直接读 store，定位"新建了 N 条却只显示几条"这类问题
    let (store_for_watch, store_total, store_orphan) = {
        let target_watch = watch_id.clone();
        with_store(&state, |store| {
            let total = store.monitored_posts.iter().filter(|post| post_accessible(&store, post)).count() as i64;
            let for_watch = target_watch
                .as_deref()
                .map(|id| {
                    store
                        .monitored_posts
                        .iter()
                        .filter(|post| post.watch_id.as_deref() == Some(id) && post_accessible(&store, post))
                        .count() as i64
                })
                .unwrap_or(0);
            let orphan = store
                .monitored_posts
                .iter()
                .filter(|post| post.watch_id.is_none() && post_accessible(&store, post))
                .count() as i64;
            Ok((for_watch, total, orphan))
        })?
    };

    let summary = json!({
        "success": true,
        "watched": stats.watched,
        // 深翻时实际滚动到的条数
        "scanned": scanned_total,
        // 落库诊断：抓到多少条、其中多少是已存在（更新）、多少因域名不符丢弃
        "scrapedPosts": scraped_total,
        "updatedExisting": updated_total,
        "droppedDomain": dropped_total,
        // 深翻上限
        "limit": BACKFILL_MAX,
        // 主页声明的作品总数（进度条分母；未知为 0）
        "profileTotal": profile_total_max,
        // 其中 URL 未见过的 = 真正补进来的历史帖
        "newPosts": stats.new_posts,
        // 因发布年份早于下限（BACKFILL_YEAR_MIN）被丢弃的历史帖数
        "droppedBefore2026": stats.dropped_old_posts,
        "detailEnriched": stats.detail_ok,
        "detailSkipped": stats.detail_skipped,
        "skipNote": stats.skip_note,
        "backfilled": stats.backfilled,
        // 因已有标记被跳过的博主数（手动 force 时恒为 0）
        "alreadyBackfilled": stats.already_backfilled,
        // 服务器未登录平台而跳过的博主数（游客视角只能拿到首屏）
        "notLoggedIn": stats.not_logged_in,
        // 触到 200 上限（历史可能还有更多）的博主数
        "limitReached": stats.limit_reached,
        // 滚动诊断明细（每个博主：滚了多少轮 / 每轮锚点数 / 实际滚动的容器）
        "scrollDiags": scroll_diags
            .iter()
            .map(|(name, diag)| json!({ "name": name, "diag": diag }))
            .collect::<Vec<_>>(),
        // 一句话诊断提示
        "hint": hint,
        // "ok" = 正常（含已抓全）/ "warn" = 需要关注
        "hintLevel": hint_level,
        // 历史视频已转后台线程自动「下载+转写」，这里报启动处理的数量
        "pendingVideos": stats.pending_videos,
        // 视频诊断明细（每个博主：帖子总数 / mediaType=video 的数 / mediaType 缺失的数 / 本次待下载数）
        // 用来分辨「压根没识别出视频帖」还是「识别出来了但下载失败」。
        "videoDiags": video_diag
            .iter()
            .map(|(name, total, typed_video, missing_type, pending)| {
                json!({
                    "name": name,
                    "posts": total,
                    "typedVideo": typed_video,
                    "missingType": missing_type,
                    "pending": pending,
                })
            })
            .collect::<Vec<_>>(),
        "failed": stats.failed,
        // 库内实况：直接读 store，回答"新建了 47 为什么只显示 5 条"
        "fetchRuns": fetch_runs,
        "storePostsForWatch": store_for_watch,
        "storePostsTotal": store_total,
        "storePostsOrphan": store_orphan,
        "ranAt": now,
    });
    println!("[monitor history] backfill summary: {}", summary);
    // 前端列表同样是内存旧快照，补完要刷新才看得到新帖
    if !quiet {
        let _ = app.emit("monitor:prefetch-done", summary.clone());
    }
    Ok(summary)
}

/// 计划任务入口：抓取所有到期的「启用中」监控项（按各自 interval_hours 判断）。
/// 启动对账：进程重启后，所有 `downloading` 都必然是上次被中断的僵尸任务，
/// 复位为 `none` 并记录原因，否则会永远卡在「下载中」且不再重试。
pub fn reconcile_interrupted_video_downloads(state: &State<'_, AppState>) {
    let _ = with_store_mut(state, |store| {
        let mut reset = 0usize;
        for post in store.monitored_posts.iter_mut() {
            if post.video_status == "downloading" {
                post.video_status = "none".to_string();
                if post.video_error.is_none() {
                    post.video_error = Some("上次下载被中断（应用重启），已复位".to_string());
                }
                reset += 1;
            }
        }
        if reset > 0 {
            crate::logging::emit_legacy_line(
                crate::logging::event::LogSource::Host,
                crate::logging::event::LogLevel::Info,
                "monitor",
                "monitor.video.reset_interrupted",
                format!("[monitor] 启动复位移位下载 {reset} 条"),
                json!({ "reset": reset }),
                None,
            );
        }
        Ok(())
    });
}

pub fn run_due_fetches(state: &State<'_, AppState>, now: i64) -> Result<Value, String> {
    let due: Vec<crate::CompetitorWatchRecord> = with_store_mut(state, |store| {
        Ok(store
            .competitor_watchlist
            .iter()
            .filter(|watch| watch.enabled && is_watch_due(watch, now))
            .cloned()
            .collect())
    })?;

    let mut checked = 0i64;
    let mut total_new = 0i64;
    for watch in &due {
        checked += 1;
        match fetch_monitor(state, &json!({ "watchId": watch.id }), None, false) {
            Ok(result) => {
                if let Some(created) = result.get("created").and_then(Value::as_array) {
                    total_new += created.len() as i64;
                }
            }
            Err(error) if error.starts_with("monitor busy") => {
                // 有一轮抓取正在跑（手动补历史等），本轮跳过即可，不算失败。
                continue;
            }
            Err(error) => {
                crate::logging::emit_legacy_line(
                    crate::logging::event::LogSource::Host,
                    crate::logging::event::LogLevel::Warn,
                    "monitor",
                    "monitor.scheduler.fetch_failed",
                    format!("[monitor scheduler] fetch for {} failed: {}", watch.name, error),
                    json!({ "watchId": watch.id, "error": error }),
                    None,
                );
            }
        }
    }
    Ok(json!({ "success": true, "checked": checked, "totalNew": total_new }))
}

pub(super) fn is_watch_due(watch: &crate::CompetitorWatchRecord, now: i64) -> bool {
    match watch
        .last_fetch_at
        .as_ref()
        .and_then(|value| value.parse::<i64>().ok())
    {
        None => true,
        Some(last) => {
            let interval_ms = watch.interval_hours.max(1) * 3_600_000;
            now.saturating_sub(last) >= interval_ms
        }
    }
}

/// 启动对标监控的常驻计划任务（独立于 Redclaw runner，应用启动即生效）。
///
/// 两条常驻循环：
/// 1. 每 5 分钟检查一次，对到期且启用的监控项执行「列表抓取」（保持现有 ad-hoc / 自动逻辑，
///    只落列表、不补全正文，开销低、频次高，负责把新帖尽快纳入监控）。
/// 2. 每天本地 00:00 触发一次「批量预取」：列表(URL 去重)→逐条补全正文→视频帖下载+转写，
///    保证用户次日 09:00 打开任意新帖即可秒见全文、视频已转写（需求 #每日00点预取）。
pub fn start_monitor_scheduler(app: AppHandle) {
    // 循环 1：5 分钟到期检查（列表抓取）
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let mut ticker = interval(std::time::Duration::from_secs(300));
            // 跳过启动时的即时 tick，避免冷启动立即抓全量。
            ticker.tick().await;
            loop {
                ticker.tick().await;
                let app_handle = app.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    let state = app_handle.state::<AppState>();
                    let now = crate::now_i64();
                    if let Err(error) = run_due_fetches(&state, now) {
                        crate::logging::emit_legacy_line(
                            crate::logging::event::LogSource::Host,
                            crate::logging::event::LogLevel::Warn,
                            "monitor",
                            "monitor.scheduler.run_due_failed",
                            format!("[monitor scheduler] run_due_fetches failed: {error}"),
                            json!({ "error": error }),
                            None,
                        );
                    }
                })
                .await;
            }
        });
    }

    // 循环 2：每天本地 00:00 批量预取（列表→补全正文→视频下载+转写）
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            // 受管多用户：由实例管理器集中触发每日任务，实例内不再自己跑，避免重复。
            if std::env::var("BEAV_DISABLE_DAILY_SCHEDULER").ok().as_deref() == Some("1") {
                return;
            }
            loop {
                let now = crate::now_i64();
                // 计算下一个本地 00:00（时区取本机，与 task_policy 的每日调度同源）。
                let next = match crate::scheduler::task_policy::next_daily_timestamp_in_timezone(
                    0, 0, now, None,
                ) {
                    Ok(ts) => ts,
                    Err(error) => {
                        crate::logging::emit_legacy_line(
                            crate::logging::event::LogSource::Host,
                            crate::logging::event::LogLevel::Warn,
                            "monitor",
                            "monitor.scheduler.daily_next_failed",
                            format!("[monitor scheduler] compute next 00:00 failed: {error}"),
                            json!({ "error": error }),
                            None,
                        );
                        // 兜底：1 小时后重试，避免死循环
                        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                        continue;
                    }
                };
                let wait_ms = (next.saturating_sub(now)).max(0) as u64;
                tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;

                let app_handle = app.clone();
                let _ = tauri::async_runtime::spawn_blocking(move || {
                    if let Err(error) = run_daily_prefetch(app_handle) {
                        crate::logging::emit_legacy_line(
                            crate::logging::event::LogSource::Host,
                            crate::logging::event::LogLevel::Warn,
                            "monitor",
                            "monitor.scheduler.daily_failed",
                            format!("[monitor scheduler] run_daily_prefetch failed: {error}"),
                            json!({ "error": error }),
                            None,
                        );
                    }
                })
                .await;
                // 跑完后循环回到顶部，重新计算下一个 00:00（= 次日），天然每天只触发一次。
            }
        });
    }
}
