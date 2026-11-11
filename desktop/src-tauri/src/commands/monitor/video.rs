use super::*;

// 下载目录：应用数据根下的 monitor-videos/（视频文件与抽帧都落这里）。
pub(super) fn monitor_videos_dir(state: &State<'_, AppState>) -> Result<PathBuf, String> {
    let base = workspace_root(state)?;
    let dir = base.join("monitor-videos");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建视频目录失败：{e}"))?;
    Ok(dir)
}

/// 竞品图片本地缓存目录：`<workspace>/monitor-images/<post_id>/`。
pub(super) fn monitor_images_dir(state: &State<'_, AppState>, post_id: &str) -> Result<PathBuf, String> {
    let dir = workspace_root(state)?.join("monitor-images").join(post_id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建图片缓存目录失败：{e}"))?;
    Ok(dir)
}

/// 单张远程图片落盘（带 Referer/UA）。成功返回本地路径，失败返回 `None`。
///
/// 为什么必须本地化：B 站 `i0.hdslb.com`、小红书 `sns-webpic*.xhscdn.com` 都校验 Referer，
/// WebView2 里 `<img src="https://...">` 不带 Referer → 403 → 详情页/导出全是裂图。
pub(super) fn cache_remote_image(
    client: &reqwest::blocking::Client,
    dir: &Path,
    url: &str,
    name: &str,
    referer: &str,
    ua: &str,
) -> Option<String> {
    let ext = url
        .split('?')
        .next()
        .unwrap_or("")
        .rsplit('.')
        .next()
        .map(|e| e.to_ascii_lowercase())
        .filter(|e| matches!(e.as_str(), "jpg" | "jpeg" | "png" | "webp" | "gif"))
        .unwrap_or_else(|| "jpg".to_string());
    let target = dir.join(format!("{name}.{ext}"));
    if target.is_file() && std::fs::metadata(&target).map(|m| m.len() > 0).unwrap_or(false) {
        return Some(target.display().to_string());
    }
    let resp = client
        .get(url)
        .header("Referer", referer)
        .header("User-Agent", ua)
        .send()
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().ok()?;
    if bytes.is_empty() || std::fs::write(&target, &bytes).is_err() {
        return None;
    }
    Some(target.display().to_string())
}

/// 批量本地化帖子图片；下载失败的条目保留原 URL（至少不丢数据）。
pub(super) fn cache_post_images(
    state: &State<'_, AppState>,
    post_id: &str,
    platform: &str,
    urls: &[String],
    cover: Option<&str>,
) -> (Vec<String>, Option<String>) {
    let dir = match monitor_images_dir(state, post_id) {
        Ok(dir) => dir,
        Err(_) => return (urls.to_vec(), cover.map(ToString::to_string)),
    };
    let referer = Platform::parse(platform)
        .map(|p| p.home_url())
        .unwrap_or("https://www.bilibili.com");
    let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(client) => client,
        Err(_) => return (urls.to_vec(), cover.map(ToString::to_string)),
    };

    let mut local: Vec<String> = Vec::with_capacity(urls.len());
    for (idx, url) in urls.iter().take(12).enumerate() {
        // 协议相对 URL（`//i0.hdslb.com/...`）必须先补 https，否则会被误判成「已是本地路径」，
        // 原样存进记录 → WebView 当成相对路径 → 裂图。
        let normalized = if url.starts_with("//") {
            format!("https:{url}")
        } else {
            url.trim().to_string()
        };
        if !normalized.starts_with("http") {
            // 已经是本地文件才保留；否则（空串/非法）直接丢
            if Path::new(&normalized).is_file() {
                local.push(normalized);
            }
            continue;
        }
        let name = format!("img_{:02}", idx + 1);
        match cache_remote_image(&client, &dir, &normalized, &name, referer, ua) {
            Some(path) => local.push(path),
            // 下载失败就**丢弃**：远程直连对这两家 CDN 必然 403，留着只会显示裂图。
            None => println!("[monitor] cache image failed, dropped: {normalized}"),
        }
    }

    let cover_local = cover.and_then(|c| {
        let normalized = if c.starts_with("//") {
            format!("https:{c}")
        } else {
            c.trim().to_string()
        };
        if !normalized.starts_with("http") {
            return Path::new(&normalized).is_file().then_some(normalized);
        }
        cache_remote_image(&client, &dir, &normalized, "cover", referer, ua)
    });
    (local, cover_local)
}

/// 在帖子页抽取视频直链的 JS 模板。
///
/// `__XHS__` / `__DY__` 由 Rust 替换为 `true`/`false` 后注入平台分支；其余为 JS 原样执行。
///
/// # 设计要点（2026-10-21 真机联调后重写）
///
/// 首版只赌 `window.__INITIAL_STATE__` / `_ROUTER_DATA` 的固定字段，实机全部落空
/// （`vtags:2, found:0`）：抖音已改用 MSE 流式播放，`<video>.src` 是 `blob:`，页面全局对象里
/// 也读不到裸直链。因此改为**不依赖站点内部结构**的广撒网：
/// 1. `performance.getEntriesByType('resource')` —— 浏览器记录的真实网络请求，平台再怎么改版都还在，
///    且带完整签名参数。这是主力手段。
/// 2. 深层遍历 `window` 里的已知状态容器（只作为补充）。
/// 3. `<video>` / `<source>` 的 `currentSrc`、`src`（含 blob: 之外的直链）。
///
/// 最后按「视频特征打分」排序：命中 `.mp4` / `.m3u8` / `video/tos` / `aweme` / `playwm` 等特征越强的排前面，
/// 优先选无水印（`wm=`）的高清地址。
pub(super) const VIDEO_EXTRACT_EXPRESSION: &str = r#"
(() => {
    const XHS = __XHS__;
    const DY = __DY__;
    const candidates = [];
    // 抖音网页版播放器走 DASH：视频轨与音频轨是**两个独立直链**。`video.play_addr` 系列
    // 只含视频（下载下来 ffprobe 查不到 audio stream），音频在 `music.play_url` / `.../audio/tos/...`。
    // 不识别音频候选 → 下载到的 mp4 永远没声音 → 语音转写必然失败（历史 bug）。
    const looksAudio = (u) => {
        if (/\.(?:mp3|m4a|aac|wav|ogg)(?:[?#]|$)/i.test(u)) return true;
        if (/mime_type=[^&]*audio/i.test(u)) return true;
        if (/\/audio\/|\/music\/|media-audio|[.-]audio[.-]/i.test(u)) return true;
        return false;
    };
    const push = (u, src) => {
        if (typeof u !== 'string') return;
        const t = u.trim();
        if (!t) return;
        if (!/^https?:\/\//i.test(t)) return;
        // 去掉 blob: / data: / 明显的图片、静态资源
        if (/^blob:|^data:/i.test(t)) return;
        if (/\.(?:jpg|jpeg|png|gif|webp|avif|svg|css|js|woff2?)(?:[?#]|$)/i.test(t)) return;
        candidates.push({ url: t, src: src || 'unknown', audio: looksAudio(t) });
    };

    // —— 1) 主力：performance resource timing（真实网络请求，含完整签名）——
    let perfTotal = 0;
    try {
        const entries = performance.getEntriesByType('resource') || [];
        perfTotal = entries.length;
        for (const e of entries) push(e.name, 'perf');
    } catch (e) {}

    // —— 2) 补充：平台状态容器深遍历 ——
    const walk = (o, depth) => {
        if (!o || typeof o !== 'object' || depth > 6) return;
        for (const key in o) {
            let v;
            try { v = o[key]; } catch (e) { continue; }
            if (typeof v === 'string') {
                const k = key.toLowerCase();
                // 只看名字里带"播放/流"语义的键，避免把头像、封面等图片地址当视频
                if (/play|stream|video|src|url_list|download_addr|bit_rate/.test(k)) push(v, 'state:' + key);
            } else if (v && typeof v === 'object') {
                walk(v, depth + 1);
            }
        }
    };
    try {
        if (XHS) walk(window.__INITIAL_STATE__, 0);
        if (DY) {
            walk(window._ROUTER_DATA, 0);
            walk(window.__INITIAL_STATE__, 0);
            walk(window.__pace_f, 0);
        }
    } catch (e) {}

    // —— 3) 补充：DOM 上的 video / source ——
    let vtags = 0;
    try {
        const els = document.querySelectorAll('video, video source, source');
        vtags = document.querySelectorAll('video').length;
        for (const el of els) push(el.currentSrc, 'dom');
        for (const el of els) push(el.getAttribute && el.getAttribute('src'), 'dom-attr');
    } catch (e) {}

    // —— 打分排序：视频特征越强越靠前 ——
    const score = (c) => {
        const u = c.url;
        let s = 0;
        if (/\.m3u8(?:[?#]|$)/i.test(u)) s += 100;              // HLS：ffmpeg 可直接合并
        else if (/\.mp4(?:[?#]|$)/i.test(u)) s += 90;           // 直链 mp4：最理想
        if (/\/video\/tos\//i.test(u)) s += 40;                 // 抖音 TOS 路径
        if (/(aweme|douyinvod|xhscdn|snssdk|byteimg)/i.test(u)) s += 30;
        if (/video|play|stream/i.test(u)) s += 15;
        if (/[\?&]wm=\d/i.test(u)) s -= 20;                     // 水印地址降权
        if (/[\?&]ratio=(\d{3,4})p/i.test(u)) s += 5;           // 明确分辨率
        if (/\.(?:jpg|jpeg|png|webp)(?:[?#]|$)/i.test(u)) s -= 200; // 兜底再杀一次图片
        return s;
    };

    const uniq = [];
    const seen = new Set();
    for (const c of candidates) {
        if (seen.has(c.url)) continue;
        seen.add(c.url);
        uniq.push(c);
    }
    uniq.sort((a, b) => score(b) - score(a));

    // 音频候选单列（不受视频打分/截断影响）：抖音 DASH 分离流要靠它补音轨。
    const audios = uniq
        .filter((c) => c.audio)
        .slice(0, 6)
        .map((c) => c.url);

    // 优先非水印、非图片
    const best = uniq.find((c) => score(c) > 20 && !c.audio) || uniq.find((c) => !c.audio) || uniq[0] || null;
    return {
        url: best ? best.url : '',
        urls: uniq.filter((c) => !c.audio).slice(0, 12).map((c) => c.url),
        audios: audios,
        srcOf: best ? best.src : '',
        debug: JSON.stringify({
            xhs: !!window.__INITIAL_STATE__,
            dy: !!window._ROUTER_DATA,
            vtags,
            perfTotal,
            found: uniq.length,
            audios: audios.length,
            top: uniq.slice(0, 3).map((c) => ({ src: c.src, u: c.url.slice(0, 120) })),
        }),
    };
})()
"#;

/// 页面抽取结果：视频直链候选 + 音频直链候选。
///
/// 分开返回是必须的 —— 抖音 DASH 把音视频拆成两条流，只下视频轨会得到**无声 mp4**，
/// 后续语音转写必然失败（且失败原因极难定位）。音频候选用于下载后补音轨。
pub(super) struct VideoStreamCandidates {
    videos: Vec<String>,
    audios: Vec<String>,
}

/// B 站取流：页面内调 `view`（拿 cid）+ `playurl`（拿 DASH 音视频直链）。
///
/// 不复用通用「扫 performance 资源」那套：B 站播放器用 **m4s + Range 分段**，
/// performance 里全是带 `&range=` 的分片地址，ffmpeg 只下一段就完事；且 CDN 校验 Referer。
/// 走官方 playurl 拿整段 `baseUrl` 才是完整文件。
pub(super) const BILI_VIDEO_EXTRACT_EXPRESSION: &str = r#"
(async () => {
    const out = { url: '', urls: [], audios: [], debug: '' };
    const m = location.pathname.match(/\/video\/(BV[0-9A-Za-z]+)/);
    const bv = m ? m[1] : '';
    if (!bv) { out.debug = 'no-bv'; return out; }
    const getJson = async (u) => {
        try {
            const r = await fetch(u, { credentials: 'include' });
            return await r.json();
        } catch (e) { return null; }
    };
    const view = await getJson('https://api.bilibili.com/x/web-interface/view?bvid=' + bv);
    if (!view || view.code !== 0 || !view.data) {
        out.debug = 'view-fail:' + (view && view.code);
        return out;
    }
    const cid = view.data.cid || (view.data.pages && view.data.pages[0] && view.data.pages[0].cid) || 0;
    if (!cid) { out.debug = 'no-cid'; return out; }
    const pu = await getJson('https://api.bilibili.com/x/player/playurl?bvid=' + bv + '&cid=' + cid + '&qn=80&fnval=4048&fourk=1');
    if (!pu || pu.code !== 0 || !pu.data) {
        out.debug = 'playurl-fail:' + (pu && pu.code);
        return out;
    }
    const pick = (x) => x && (x.baseUrl || x.base_url || '');
    const dash = pu.data.dash || null;
    if (dash) {
        const vids = (dash.video || []).filter((x) => pick(x));
        // 优先 avc(h264, codecid=7)：WebView 直接能播；再按清晰度 id 从高到低
        vids.sort((a, b) => {
            const ca = a.codecid === 7 ? 1 : 0;
            const cb = b.codecid === 7 ? 1 : 0;
            if (ca !== cb) return cb - ca;
            return (b.id || 0) - (a.id || 0);
        });
        out.urls = vids.slice(0, 4).map(pick);
        const au = (dash.audio || []).filter((x) => pick(x));
        au.sort((a, b) => (b.bandwidth || 0) - (a.bandwidth || 0));
        out.audios = au.slice(0, 2).map(pick);
    } else if (pu.data.durl && pu.data.durl[0]) {
        // 老格式（番剧/部分版权视频）：整段 mp4
        out.urls = (pu.data.durl || []).slice(0, 4).map((d) => d.url).filter(Boolean);
    }
    out.url = out.urls[0] || '';
    out.debug = JSON.stringify({ bv: bv, cid: cid, videos: out.urls.length, audios: out.audios.length });
    return out;
})()
"#;

pub(super) async fn extract_video_stream_url_async(
    ws_url: &str,
    post_url: &str,
    platform: &str,
) -> Result<VideoStreamCandidates, String> {
    let is_bili = Platform::parse(platform) == Some(Platform::Bilibili);
    let (expression, await_promise) = if is_bili {
        (BILI_VIDEO_EXTRACT_EXPRESSION.to_string(), true)
    } else {
        let mut expr = VIDEO_EXTRACT_EXPRESSION.to_string();
        expr = expr
            .replace("__XHS__", if Platform::parse(platform) == Some(Platform::Xiaohongshu) { "true" } else { "false" })
            .replace("__DY__", if Platform::parse(platform) == Some(Platform::Douyin) { "true" } else { "false" });
        (expr, false)
    };

    let result = headless_browser::navigate_and_eval_settle(
        ws_url,
        post_url,
        &expression,
        8_000,
        await_promise,
    )
    .await?;
    // navigate_and_eval 返回 JS 求值结果；要么是对象，要么是字符串化的对象，做兼容解析。
    let parsed = if result.is_string() {
        serde_json::from_str::<Value>(result.as_str().unwrap_or("{}"))
            .unwrap_or_else(|_| json!({}))
    } else {
        result.clone()
    };
    let debug = parsed
        .get("debug")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // 候选直链（已按视频特征打分排序，首选在前）；交给调用方逐个尝试 —— 签名可能个别失效。
    let mut urls: Vec<String> = Vec::new();
    if let Some(list) = parsed.get("urls").and_then(Value::as_array) {
        for item in list {
            if let Some(s) = item.as_str() {
                if !s.is_empty() {
                    urls.push(s.to_string());
                }
            }
        }
    }
    if urls.is_empty() {
        if let Some(one) = parsed.get("url").and_then(Value::as_str) {
            if !one.is_empty() {
                urls.push(one.to_string());
            }
        }
    }
    if urls.is_empty() {
        return Err(format!(
            "未能从页面抽取到视频直链（debug：{debug}）。取流逻辑可能已变，请按 debug 回填抽取字段。"
        ));
    }
    let audios: Vec<String> = parsed
        .get("audios")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default();
    println!(
        "[monitor] extract_video_stream_url debug: {debug}（video={} audio={}）",
        urls.len(),
        audios.len()
    );
    Ok(VideoStreamCandidates {
        videos: urls,
        audios,
    })
}

/// 探测本地视频是否含音轨。
///
/// 抖音 DASH 的 `play_addr` 只给**视频轨**，下下来是个无声 mp4。不探测的话，后续语音转写会在
/// ffmpeg 抽音频阶段报 `Output file does not contain any stream`，而错误又被上层吞掉，
/// 表现成「转写永远为空」且原因不明。探测失败（ffprobe 缺）时保守返回 `true`，
/// 避免误伤本来正常的文件。
pub(super) fn video_has_audio(app: &AppHandle, path: &str) -> bool {
    let Ok(probe) = ffprobe_program(Some(app)) else {
        return true;
    };
    let Ok(out) = Command::new(&probe)
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("a")
        .arg("-show_entries")
        .arg("stream=codec_name")
        .arg("-of")
        .arg("csv=p=0")
        .arg(path)
        .output()
    else {
        return true;
    };
    !String::from_utf8_lossy(&out.stdout).trim().is_empty()
}

/// 用 ffprobe 读容器时长（秒）。探测不到返回 `None`，不编造默认值。
pub(super) fn probe_duration_secs(app: &AppHandle, path: &str) -> Option<f64> {
    let probe = ffprobe_program(Some(app)).ok()?;
    let out = Command::new(&probe)
        .arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("format=duration")
        .arg("-of")
        .arg("csv=p=0")
        .arg(path)
        .output()
        .ok()?;
    let raw = String::from_utf8_lossy(&out.stdout);
    let value = raw.trim().trim_end_matches(',').parse::<f64>().ok()?;
    if value.is_finite() && value > 0.0 {
        Some(value)
    } else {
        None
    }
}

/// 为无声视频补音轨（抖音 DASH 音视频分离的补偿）。
///
/// 视频轨已从 `play_addr` 下好，这里再抓一条音频流（`music.play_url` / `.../audio/tos/...`），
/// 用 `-c copy` 合并成带声音的 mp4 并原地替换。合并走「临时文件 + rename」，
/// 中途失败不会破坏已下好的视频轨 —— 没声音也比没画面好。
pub(super) fn merge_audio_into_video(
    app: &AppHandle,
    video_path: &std::path::Path,
    audio_url: &str,
    referer: &str,
    ua: &str,
) -> Result<(), String> {
    let ff = ffmpeg_program(Some(app))?;
    let headers = format!("Referer: {referer}\r\nUser-Agent: {ua}");
    let tmp_audio = video_path.with_extension("audio.m4a");
    let _ = std::fs::remove_file(&tmp_audio);
    let mut cmd = Command::new(&ff);
    cmd.arg("-y")
        .arg("-headers")
        .arg(&headers)
        .arg("-i")
        .arg(audio_url)
        .arg("-rw_timeout")
        .arg("20000000")
        .arg("-vn")
        .arg(&tmp_audio);
    let out = run_command_with_timeout(&mut cmd, std::time::Duration::from_secs(180))
        .map_err(|e| format!("下载音频轨失败：{e}"))?;
    if !out.status.success() || std::fs::metadata(&tmp_audio).map(|m| m.len()).unwrap_or(0) == 0 {
        let _ = std::fs::remove_file(&tmp_audio);
        return Err(format!(
            "音频轨下载失败：{}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .rev()
                .take(2)
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }

    let merged = video_path.with_extension("merged.mp4");
    let _ = std::fs::remove_file(&merged);
    let out = Command::new(&ff)
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg("-i")
        .arg(&tmp_audio)
        .arg("-map")
        .arg("0:v:0")
        .arg("-map")
        .arg("1:a:0")
        .arg("-c")
        .arg("copy")
        .arg("-movflags")
        .arg("+faststart")
        .arg(&merged)
        .output()
        .map_err(|e| format!("启动 ffmpeg 合并失败：{e}"))?;
    let _ = std::fs::remove_file(&tmp_audio);
    if !out.status.success() || std::fs::metadata(&merged).map(|m| m.len()).unwrap_or(0) == 0 {
        let _ = std::fs::remove_file(&merged);
        return Err(format!(
            "音视频合并失败：{}",
            String::from_utf8_lossy(&out.stderr)
                .lines()
                .rev()
                .take(2)
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }
    std::fs::rename(&merged, video_path).map_err(|e| format!("替换原视频失败：{e}"))?;
    Ok(())
}

/// 判断原片是否需要转码成 H.264 代理视频才能内联播放。
///
/// WebView2（Windows）多数环境**没有 HEVC/VP9/AV1 软硬解码器**，且 Tauri asset 协议不支持 Range 分段，
/// 超过 1080p 的原片即使编码可解，整文件拉取也很慢。所以：
/// - 视频流编码不是 `h264` → 必须转（HEVC/VP9/AV1 播不了）
/// - 高度 > 1080 → 转成 720p 轻量版（内联播放更稳）
/// - 其余（H.264 ≤1080p）→ 用 faststart 后的原片直接播，省下数分钟软转码。
///
/// 探测失败（ffprobe 缺/异常）时**保守转码**，宁慢勿放不出来。
pub(super) fn needs_playback_transcode(app: &AppHandle, path: &str) -> bool {
    let probe = match ffprobe_program(Some(app)) {
        Ok(p) => p,
        Err(_) => return true,
    };
    let out = Command::new(&probe)
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("v:0")
        .arg("-show_entries")
        .arg("stream=codec_name,height")
        .arg("-of")
        .arg("json")
        .arg(path)
        .output();
    let parsed = match out {
        Ok(o) if o.status.success() => match serde_json::from_slice::<Value>(&o.stdout) {
            Ok(v) => v,
            Err(_) => return true,
        },
        _ => return true,
    };
    let streams = parsed.get("streams").and_then(Value::as_array);
    match streams.and_then(|s| s.first()) {
        Some(s) => {
            let codec = s.get("codec_name").and_then(Value::as_str).unwrap_or("").to_lowercase();
            let height = s.get("height").and_then(Value::as_i64).unwrap_or(0);
            let not_h264 = codec != "h264" && !codec.is_empty();
            let too_tall = height > 1080;
            not_h264 || too_tall
        }
        None => true,
    }
}

/// 下载竞品视频到本地（仅小红书 / 抖音，需登录态 Chrome）。
///
/// 流程：CDP 现抓现下视频直链（签名短链过期快）→ 用 ffmpeg 落盘（同时支持直链 mp4 与 HLS m3u8 合并）。
/// 下载成功写回 `video_local_path` / `video_status=done`；失败写 `video_status=failed` + `video_error`。
/// 带看门狗的进程执行：网络类 ffmpeg（拉 B站/抖音流）可能在「连接已建立但不再给数据」
/// 或 URL 过期时挂死，`-rw_timeout` 并不总能生效。这里按进程级超时兜底：到点强杀并返回
/// `TimedOut`，让上层按失败处理、队列继续推进，而不是整批卡死。
pub(super) fn run_command_with_timeout(
    command: &mut Command,
    timeout: std::time::Duration,
) -> std::io::Result<std::process::Output> {
    use std::io::Read;
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let stdout_handle = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(pipe) = stdout_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buffer);
        }
        buffer
    });
    let stderr_handle = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(pipe) = stderr_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buffer);
        }
        buffer
    });
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("进程超过 {}s 无响应，已强制结束", timeout.as_secs()),
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    };
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

pub(super) fn download_video(app: &AppHandle, state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    let record = with_store_mut(state, |store| {
        Ok(store.monitored_posts.iter().find(|p| p.id == id).cloned())
    })?
    .ok_or_else(|| "动态不存在".to_string())?;
    if !Platform::parse(&record.platform).map(|p| p.supports_video()).unwrap_or(false) {
        return Ok(json!({
            "success": false,
            "note": "目前仅支持小红书 / 抖音 / B 站的视频下载（需登录态 Chrome）。",
        }));
    }
    let port = headless_browser::active_chrome_port(&record.platform);
    if port == 0 {
        return Ok(json!({
            "success": false,
            "note": "该平台 Chrome 未连接（应用重启后会话失效）。请到「登录管理」点「打开 Chrome 登录」重新接管后再下载。",
        }));
    }

    let dir = monitor_videos_dir(state)?;
    let dest = dir.join(format!("{id}.mp4"));
    let dest_str = dest.display().to_string();
    let platform = record.platform.clone();
    let post_url = record.post_url.clone();
    let referer = Platform::parse(&platform)
        .map(|p| p.home_url())
        .unwrap_or(Platform::Douyin.home_url())
        .to_string();
    let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36".to_string();

    // 标记下载中，便于前端展示状态
    with_store_mut(state, |store| {
        if let Some(p) = store.monitored_posts.iter_mut().find(|x| x.id == id) {
            p.video_status = "downloading".to_string();
        }
        Ok(())
    })?;

    // 1) CDP 现抓现下视频直链（签名短链，必须同会话内完成）
    let candidates = tauri::async_runtime::block_on(async {
        let ws_url = headless_browser::ws_url_for_port(port).await?;
        extract_video_stream_url_async(&ws_url, &post_url, &platform).await
    })?;

    // 2) ffmpeg 落盘（直链 mp4 与 HLS m3u8 都支持；带 Referer 过防盗链）。
    //    签名链偶发失效 / 返回 403，按候选顺序逐个重试，取第一个成功的。
    //
    // `patchAudio` 模式：视频轨已在本地且完好时**跳过视频重下**，只走后面的补音轨逻辑。
    // 存量 54 条抖音视频都是「有画面无声音」，重下要再拉 2GB+，而补音轨只下几十 KB。
    let patch_only = payload
        .get("patchAudio")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let existing_video_ok =
        dest.is_file() && std::fs::metadata(&dest).map(|m| m.len() > 0).unwrap_or(false);
    let skip_video_download = patch_only && existing_video_ok;

    let ff = ffmpeg_program(Some(app))?;
    let headers = format!("Referer: {referer}\r\nUser-Agent: {ua}");
    let mut last_stderr = String::new();
    let mut ok = skip_video_download;
    if skip_video_download {
        println!("[monitor] patchAudio: 跳过视频重下，直接补音轨 {id}");
    }
    for stream_url in candidates.videos.iter().take(4).filter(|_| !skip_video_download) {
        let mut cmd = Command::new(&ff);
        cmd.arg("-y")
            .arg("-headers")
            .arg(&headers)
            .arg("-i")
            .arg(stream_url)
            .arg("-c")
            .arg("copy")
            // faststart：把 moov 索引原子搬到文件头。不加的话 moov 在 500MB+ 的文件末尾，
            // 而 Tauri asset 协议不支持 Range 请求，WebView 只能整文件拉完才能解码 → 视频卡住播不出。
            .arg("-movflags")
            .arg("+faststart")
            .arg("-rw_timeout")
            .arg("20000000")
            .arg(&dest_str);
        let output = run_command_with_timeout(&mut cmd, std::time::Duration::from_secs(600))
            .map_err(|e| format!("启动 ffmpeg 失败：{e}"))?;
        if output.status.success() {
            ok = true;
            break;
        }
        last_stderr = String::from_utf8_lossy(&output.stderr).to_string();
        println!("[monitor] ffmpeg candidate failed: {}", &last_stderr.lines().rev().take(3).collect::<Vec<_>>().join(" | "));
    }
    if !ok {
        // 失败时删掉可能残留的半截文件，避免下次「已下载」判定出错
        let _ = std::fs::remove_file(&dest);
        let brief: String = last_stderr
            .lines()
            .rev()
            .filter(|l| !l.trim().is_empty())
            .take(3)
            .collect::<Vec<_>>()
            .join(" | ");
        let msg = if brief.is_empty() {
            "ffmpeg 下载失败（无输出，可能是链接已过期，重试即可）".to_string()
        } else {
            format!("ffmpeg 下载失败：{brief}")
        };
        with_store_mut(state, |store| {
            if let Some(p) = store.monitored_posts.iter_mut().find(|x| x.id == id) {
                p.video_status = "failed".to_string();
                p.video_error = Some(msg.clone());
            }
            Ok(())
        })?;
        return Ok(json!({ "success": false, "error": msg }));
    }

    // 2.5) 补音轨：抖音走 DASH，音视频是**两条独立流**，`play_addr` 只含视频轨。
    //      不补的话拿到的 mp4 全程无声 → 语音转写必然失败（这正是此前 56 次转写
    //      全部抽音频失败、转写恒空的根因）。补失败不阻断下载：无声音仍可抽帧、可播放。
    let has_audio_before = video_has_audio(app, &dest_str);
    let mut audio_patched = false;
    let mut audio_note: Option<String> = None;
    if !has_audio_before {
        if candidates.audios.is_empty() {
            audio_note = Some("源视频无音轨，且页面未抽取到音频候选（抖音可能已改版）".to_string());
        } else {
            let mut last_err = String::new();
            for audio_url in candidates.audios.iter().take(3) {
                match merge_audio_into_video(app, &dest, audio_url, &referer, &ua) {
                    Ok(()) => {
                        audio_patched = true;
                        println!("[monitor] audio track merged for {id}");
                        break;
                    }
                    Err(e) => {
                        println!("[monitor] audio merge failed for {id}: {e}");
                        last_err = e;
                    }
                }
            }
            if !audio_patched {
                audio_note = Some(format!("源视频无音轨，补音轨失败：{last_err}"));
            }
        }
    }
    let has_audio = has_audio_before || audio_patched;

    let size = std::fs::metadata(&dest).map(|m| m.len() as i64).unwrap_or(0);

    // 3) 按需生成**轻量代理视频**用于前端播放。
    //
    // 原片若是 H.264 且 ≤1080p，WebView 配 faststart 后直接内联播放即可，**不必转码**（省下数分钟）。
    // 仅当原片是 HEVC/VP9/AV1（WebView2 多数环境无解码器）或超过 1080p 时才转成 720p/H.264/CRF30——
    // 实测 4K·HEVC·14 分钟 533MB 全软转码要 4-5 分钟，这是 CPU 软编码的固有成本，不能更快，
    // 故前端超时放宽到 600s 覆盖它；原片始终保留供抽帧与转写。
    let preview_path = dest.with_file_name(format!("{id}.preview.mp4"));
    let _ = std::fs::remove_file(&preview_path); // 上次失败的半截产物先清掉
    let need_transcode = needs_playback_transcode(app, &dest_str);
    // 默认不转码：复用已 faststart 的原片作播放源（H.264 ≤1080p 直接可播）。
    let mut preview_str: Option<String> = None;
    if need_transcode {
        let transcode = Command::new(&ff)
            .arg("-y")
            .arg("-i")
            .arg(&dest_str)
            .arg("-vf")
            // 最长边不超过 1280，保持偶数宽高（h264 yuv420p 要求）
            .arg("scale='min(1280,iw)':-2")
            .arg("-c:v")
            .arg("libx264")
            .arg("-preset")
            .arg("veryfast")
            .arg("-crf")
            .arg("30")
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg("-c:a")
            .arg("aac")
            .arg("-b:a")
            .arg("96k")
            .arg("-movflags")
            .arg("+faststart")
            .arg(&preview_path)
            .output()
            .map_err(|e| format!("启动 ffmpeg 转码失败：{e}"))?;
        if transcode.status.success()
            && std::fs::metadata(&preview_path).map(|m| m.len()).unwrap_or(0) > 0
        {
            preview_str = Some(preview_path.display().to_string());
        } else {
            println!(
                "[monitor] preview transcode failed: {}",
                String::from_utf8_lossy(&transcode.stderr)
                    .lines()
                    .rev()
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
        }
    }
    // 转码失败不阻断下载：前端退回原片，原片若是 H.264 仍可播


    with_store_mut(state, |store| {
        if let Some(p) = store.monitored_posts.iter_mut().find(|x| x.id == id) {
            p.video_local_path = Some(dest_str.clone());
            p.video_status = "done".to_string();
            p.video_size = Some(size);
            p.video_preview_path = preview_str.clone();
            p.video_downloaded_at = Some(now_i64());
            p.video_error = None;
            p.video_has_audio = Some(has_audio);
        }
        Ok(())
    })?;

    Ok(json!({
        "success": true,
        "localPath": dest_str,
        "previewPath": preview_str,
        "size": size,
        "hasAudio": has_audio,
        "audioWarning": audio_note,
    }))
}

/// 跑一次语音转写，返回 `(全文, 状态, 错误说明)`。
///
/// 状态取值：`ok` / `no-audio`（源视频无音轨）/ `not-configured`（设置里没配 ASR）/ `failed`。
///
/// 返回体契约（务必对齐 `execute_media_transcribe`）：`transcript`（全文）、`transcriptPath`
/// （字幕文件路径）、`contentPreview`（**已被截断到 4000 字**）。早期实现按 `text`/`subtitles`/
/// `content` 取值 —— 这些键根本不存在，于是转写成功也恒为空串。改动此处前先确认契约。
pub(super) fn run_transcription(
    app: &AppHandle,
    state: &State<'_, AppState>,
    local_path: &str,
) -> (String, String, Option<String>, String) {
    if !video_has_audio(app, local_path) {
        return (
            String::new(),
            "no-audio".to_string(),
            Some(
                "源视频没有音轨（抖音 DASH 音视频分离，只下到了视频轨），无法转写。\
                 重新下载该视频会自动补音轨后再分析。"
                    .to_string(),
            ),
            String::new(),
        );
    }
    match execute_media_transcribe(app, state, None, &json!({ "sourcePath": local_path })) {
        Ok(v) => {
                let text = v
                .get("transcript")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    v.get("contentPreview")
                        .and_then(Value::as_str)
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or_default();
            // 时间轴：单独取转写服务产出的 SRT 文件（带每句起止时间），供前端做「时间轴」展示。
            // 与 transcript 分离，绝不把 `00:00:00,800 -->` 塞进 clean 全文（会污染 LLM / 详情页）。
            let srt = v
                .get("transcriptPath")
                .and_then(Value::as_str)
                .and_then(|p| std::fs::read_to_string(p).ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_default();
            if text.is_empty() {
                // 没有干净全文时，从 SRT 抽掉序号 / 时间轴行作为降级全文，保证 transcript 不为空且无时间戳噪音。
                let plain = if srt.is_empty() {
                    String::new()
                } else {
                    strip_srt_timestamps(&srt)
                };
                if plain.is_empty() {
                    (
                        String::new(),
                        "failed".to_string(),
                        Some("转写服务返回了空内容（整段无口播，或音频被判成纯音乐）。".to_string()),
                        srt,
                    )
                } else {
                    (plain, "ok".to_string(), None, srt)
                }
            } else {
                (text, "ok".to_string(), None, srt)
            }
        }
        Err(e) => {
            println!("[monitor] video transcript failed: {e}");
            let status = if e.contains("未配置音频转写服务") {
                "not-configured"
            } else {
                "failed"
            };
            (
                String::new(),
                status.to_string(),
                Some(e.chars().take(800).collect()),
                String::new(),
            )
        }
    }
}

/// 把 SRT 文本去掉序号行与时间轴行（`00:00:01,200 --> 00:00:04,500`），仅保留字幕正文。
/// 用于转写服务只回传 SRT、没有干净 `transcript` 全文时的降级处理。
pub(super) fn strip_srt_timestamps(srt: &str) -> String {
    srt.lines()
        .map(|l| l.trim())
        .filter(|l| {
            if l.is_empty() {
                return false;
            }
            // 跳过纯序号行
            if l.chars().all(|c| c.is_ascii_digit()) {
                return false;
            }
            // 跳过时间轴行
            if l.contains("-->") {
                return false;
            }
            true
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 分析已下载的竞品视频：抽帧（ffmpeg）+ 语音转写（复用 media_transcribe）+ 落库。
///
/// 视频本体需先经 `monitor:download-video` 落盘；转写需在设置里配置 transcription 服务，
/// 未配置时 transcript 留空（不报错，仅跳过）。LLM 摘要（`summary`）暂未接入，可后续复用 deconstruct 链路。
/// 抽帧结果能否直接复用（= 视频没变过）。
///
/// 判据：目录里已有帧，且最新一帧的时间不早于「视频下载完成时间」。
/// `downloaded_at_secs == 0` 表示老数据没有该字段，此时只要目录里有帧就复用
/// （没有更可靠的判据，重抽一次的代价远大于用旧帧的风险）。
pub(super) fn frames_are_reusable(
    has_frames: bool,
    latest_frame_secs: Option<i64>,
    downloaded_at_secs: i64,
) -> bool {
    if !has_frames {
        return false;
    }
    match latest_frame_secs {
        Some(secs) => downloaded_at_secs == 0 || secs >= downloaded_at_secs,
        None => false,
    }
}

/// 新抓到的正文能否覆盖旧正文。
///
/// - 旧内容是**脏数据**（含「举报发布时间」这类界面噪音）→ 无条件覆盖，
///   哪怕新内容更短。否则垃圾因为"更长"会一直锁在库里（71 字噪音 vs 48 字干净文案）。
/// - 否则要求新内容不短于旧内容：平台改版后提取器可能只拿得到标题级文本，
///   无条件覆盖会把已存好的完整正文冲成标题，而拆解/摘要依赖全文。
pub(super) fn should_replace_content(old: &str, new: &str) -> bool {
    let old_is_noisy = old.contains("举报发布时间") || old.contains("举报\n发布时间");
    old_is_noisy || new.chars().count() >= old.chars().count()
}

pub(super) fn analyze_video(app: &AppHandle, state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    let record = with_store_mut(state, |store| {
        Ok(store.monitored_posts.iter().find(|p| p.id == id).cloned())
    })?
    .ok_or_else(|| "动态不存在".to_string())?;
    let local_path = record
        .video_local_path
        .clone()
        .ok_or_else(|| "该视频尚未下载，请先「下载视频」".to_string())?;

    let dir = monitor_videos_dir(state)?;
    let frames_dir = dir.join(&id).join("frames");
    std::fs::create_dir_all(&frames_dir).map_err(|e| format!("创建帧目录失败：{e}"))?;

    let read_frames = || -> Vec<String> {
        std::fs::read_dir(&frames_dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|e| {
                        let p = e.path();
                        if p.extension().map(|x| x == "jpg").unwrap_or(false) {
                            Some(p.display().to_string())
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    };

    let existing_frames = read_frames();

    // 帧还能不能直接复用：目录里有帧，且最新一帧不早于「视频下载完成时间」。
    //
    // 为什么要这个判断：抽帧是 `fps=1/5` 解全片，46 分钟的视频要抽 550 帧，
    // 每次都重跑纯属白烧 CPU —— 尤其是「转写失败后点重新分析」这种场景，
    // 本来只想重跑 ASR，却连抽帧一起重来。重新下载会刷新 video_downloaded_at，
    // 旧帧自动失效、照常重抽，所以不需要额外的 force 开关。
    let latest_frame_secs = existing_frames
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok()?.modified().ok())
        .filter_map(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .max();
    let downloaded_at_secs = record.video_downloaded_at.map(|ms| ms / 1000).unwrap_or(0);
    let video_exists = Path::new(&local_path).exists();
    let frames_reusable = frames_are_reusable(
        !existing_frames.is_empty(),
        latest_frame_secs,
        downloaded_at_secs,
    );

    let frames: Vec<String> = if frames_reusable {
        existing_frames
    } else if !video_exists {
        // 分析完会 `delete_original_after_analysis` 删掉原片、只留代理，
        // 而 video_local_path 仍指向已删的原片 —— 此时抽帧必然失败（且错误被
        // `let _ =` 吞掉）。没有源文件就老老实实用现有帧，别做无用功。
        existing_frames
    } else {
        let ff = ffmpeg_program(Some(app))?;
        let frame_pattern = frames_dir.join("frame_%03d.jpg").display().to_string();
        let _ = Command::new(&ff)
            .arg("-y")
            .arg("-i")
            .arg(&local_path)
            .arg("-vf")
            .arg("fps=1/5")
            .arg(&frame_pattern)
            .output();
        read_frames()
    };

    // 语音转写：默认在抽帧后同步跑；`skipTranscript` 时为「只抽帧」模式（下载 worker 用），
    // 转写交给 `spawn_transcribe_workers` 的后置队列，避免 ASR 等待阻塞下载并行度。
    let local_path_str = local_path.clone();
    let skip_transcript = payload
        .get("skipTranscript")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let force_transcript = payload
        .get("force")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // 已有可用转写时直接复用：重新分析通常只想刷新抽帧/摘要，
    // 不能因为一次网络抖动或服务商故障，把之前成功的结果覆盖成空。
    let existing_analysis = with_store(state, |store| {
        Ok(store
            .post_video_analyses
            .iter()
            .find(|a| a.post_id == id)
            .cloned())
    })?;
    let existing_opt = existing_analysis
        .as_ref()
        .filter(|a| a.transcript_status == "ok" && !a.transcript.trim().is_empty());

    let (mut transcript, mut transcript_status, mut transcript_error, mut transcript_srt) = if skip_transcript {
        (String::new(), "pending".to_string(), None, String::new())
    } else if !force_transcript {
        if let Some(a) = existing_opt {
            (a.transcript.clone(), a.transcript_status.clone(), a.transcript_error.clone(), a.transcript_srt.clone())
        } else {
            run_transcription(app, state, &local_path_str)
        }
    } else {
        run_transcription(app, state, &local_path_str)
    };

    // 本次转写失败/为空、但之前有成功结果 → 保留旧结果（含时间轴）
    if !skip_transcript && transcript.trim().is_empty() && transcript_status != "ok" {
        if let Some(a) = existing_opt {
            transcript = a.transcript.clone();
            transcript_status = a.transcript_status.clone();
            transcript_error = a.transcript_error.clone();
            transcript_srt = a.transcript_srt.clone();
        }
    }
    let duration_secs = probe_duration_secs(app, &local_path_str);

    // 按 post_id upsert 到视频分析库
    let now = now_i64();
    let analysis = with_store_mut(state, |store| {
        let existing = store.post_video_analyses.iter().find(|a| a.post_id == id).cloned();
        let mut rec = existing.unwrap_or_else(PostVideoAnalysisRecord::default);
        if rec.id.is_empty() {
            rec.id = make_id("video-analysis");
        }
        rec.post_id = id.clone();
        rec.platform = record.platform.clone();
        rec.author = record.author.clone();
        rec.post_title = record.title.clone();
        rec.post_url = record.post_url.clone();
        rec.local_path = local_path.clone();
        rec.transcript = transcript.clone();
        rec.transcript_status = transcript_status.clone();
        rec.transcript_error = transcript_error.clone();
        rec.transcript_srt = transcript_srt.clone();
        rec.duration_secs = duration_secs;
        rec.frames = frames.clone();
        rec.created_at = if rec.created_at == 0 { now } else { rec.created_at };
        rec.updated_at = now;
        if let Some(slot) = store.post_video_analyses.iter_mut().find(|a| a.post_id == id) {
            *slot = rec.clone();
        } else {
            store.post_video_analyses.push(rec.clone());
        }
        Ok(serde_json::to_value(&rec).unwrap_or_else(|_| json!({})))
    })?;

    // 分析完成后自动导出「帖子最终产物」到知识库目录树
    export_post_product(state, &id);
    // 清理原片，仅保留预览（存在独立预览时才删）
    delete_original_after_analysis(state, &id);

    Ok(json!({ "success": true, "analysis": analysis }))
}

/// 重试失败只更新尝试状态，保留上一次成功的正文和字幕。
pub(super) fn apply_transcription_result(rec: &mut PostVideoAnalysisRecord, transcript: &str, status: &str, error: Option<String>, srt: &str) {
    if status == "ok" || (rec.transcript.is_empty() && rec.transcript_srt.is_empty()) {
        rec.transcript = transcript.to_string();
        rec.transcript_srt = srt.to_string();
    }
    rec.transcript_status = status.to_string();
    rec.transcript_error = error;
}

/// 只补语音转写：不重新抽帧、不动已有帧，仅跑 ASR 并回填记录。
///
/// 用于「下载阶段只抽帧，转写排队后置」的流程（见 `spawn_transcribe_workers`），
/// 也供前端「只重新转写」按钮使用。记录不存在时退化成完整分析（先建记录再说）。
pub(super) fn transcribe_video_only(
    app: &AppHandle,
    state: &State<'_, AppState>,
    payload: &Value,
) -> Result<Value, String> {
    let id = str_field(payload, "id").ok_or_else(|| "缺少 id".to_string())?;
    let has_record = with_store(state, |store| {
        Ok(store
            .post_video_analyses
            .iter()
            .any(|a| a.post_id == id))
    })?;
    if !has_record {
        return analyze_video(app, state, &json!({ "id": id }));
    }
    let record = with_store(state, |store| {
        Ok(store.monitored_posts.iter().find(|p| p.id == id).cloned())
    })?
    .ok_or_else(|| "动态不存在".to_string())?;
    let local_path = record
        .video_local_path
        .clone()
        .ok_or_else(|| "该视频尚未下载，请先「下载视频」".to_string())?;

    let (transcript, status, error, srt) = run_transcription(app, state, &local_path);
    let now = now_i64();
    let analysis = with_store_mut(state, |store| {
        let Some(rec) = store.post_video_analyses.iter_mut().find(|a| a.post_id == id) else {
            return Ok(json!({}));
        };
        apply_transcription_result(rec, &transcript, &status, error.clone(), &srt);
        rec.updated_at = now;
        Ok(serde_json::to_value(&*rec).unwrap_or_else(|_| json!({})))
    })?;

    // 转写补齐后自动导出「帖子最终产物」到知识库目录树
    export_post_product(state, &id);
    delete_original_after_analysis(state, &id);

    Ok(json!({ "success": true, "analysis": analysis, "status": status }))
}
