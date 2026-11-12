use super::*;

/// —— 帖子最终产物导出（5 级目录分类）——
///
/// 把一篇帖子的「最终产物」按知识库根目录落盘为物理文件夹树：
///   知识库根 / 平台(抖音) / 日期(YYYYMMDD) / 作者 / 标题前40字[+帖子ID] / 具体内容文件
/// 触发：视频帖 `analyze_video` 完成后、转写补齐(`transcribe_video_only`)后、以及
///       图文/视频帖 `deconstruct` 完成后自动导出；也可经 `monitor:export-post` 手动触发
///       （便于导出未拆解的图文帖）。
/// 内容为：原图(`images`)、抽帧图(`frames`)、正文、转写全文、转写时间轴(SRT)、
///         视频摘要、爆文拆解(六要素 JSON+MD)、帖子信息(JSON 溯源)。
/// 设计为幂等：重复导出只覆盖/补齐，不会重复造目录；标题前 40 字撞名时追加帖子 ID
/// （通过 `.beav-post-id` 标记文件识别「自己的」目录，跨日期也能找回，避免重导出时产生重复目录）。

/// 中文平台名（知识库一级目录用）。
pub(super) fn platform_label(platform: &str) -> String {
    match Platform::parse(platform) {
        Some(p) => p.label().to_string(),
        None => {
            let s = platform.trim();
            if s.is_empty() {
                "未知平台".to_string()
            } else {
                sanitize_fs_name(s)
            }
        }
    }
}

/// 详情页抽到的作者是否允许覆盖记录里已有的作者。
///
/// 背景：列表抓取阶段 `author` 写的是 `watch.name`（该监控项的博主本人），这是权威值。
/// 详情页的 `author` 来自页面兜底链（SSR → `_ROUTER_DATA.item_list[0]` → 作用域 DOM），
/// 会误抓正文里的 `@提及`（如「@抖音科技」「@抖音精选 App官方账号」）或推荐位的其他作者。
/// 一旦让它覆盖，同一个博主会因为不同帖子的 SSR 命中情况不同而裂成多个导出目录
/// （导出路径是 `导出根/平台/日期/作者/标题`）。
///
/// 因此：**有监控归属且当前作者非空时，拒绝详情页覆盖**；
/// 仅「无归属的临时帖（adhoc）」或「当前作者为空」才采用详情页的值。
pub(super) fn detail_author_may_override(watch_id: Option<&str>, current_author: &str) -> bool {
    let has_watch = watch_id.map(str::trim).is_some_and(|value| !value.is_empty());
    let has_author = !current_author.trim().is_empty();
    !(has_watch && has_author)
}

/// 把文件名非法字符（Windows：`\ / : * ? " < > |` 及控制字符）替换为 `_` 并压缩连续下划线。
pub(super) fn sanitize_fs_name(s: &str) -> String {
    const ILLEGAL: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let mut out: String = s
        .chars()
        .map(|c| {
            if ILLEGAL.contains(&c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    while out.contains("__") {
        out = out.replace("__", "_");
    }
    out.trim().trim_matches('_').to_string()
}

/// 导出四级目录取标题前多少个字符（按 Unicode 字符计，空标题回退「未命名」）。
///
/// 历史：最早写死 10 —— 中文场景下明显不够，`【抖音独家】` 这类栏目前缀就占 6 个字符，
/// 剩 4 个只装得下 `Open`，于是 `【抖音独家】OpenAI Images 2.5 重磅发布` 与
/// `【抖音独家】OpenAI 为何紧急暂停 Astra 测试？` 会撞成同一个目录名，
/// 后者只能靠 `_{postId}` 后缀区分，目录树可读性很差。
///
/// 之后调到 20 缓解了碰撞，但实测仍然会切在词中间
/// （`【抖音独家】别再被骗了！AI根本没有在思`），Finder / Spotlight 里都难辨认。
/// 现在取 40：存量老数据本来就是 40 字上下的完整句式，新数据保持同一风格；
/// 40 个汉字 = 120 字节，离文件系统单段 255 字节上限仍很远。
/// 真撞了还有 `_{postId}` 后缀兜底（见 `try_export_post_product`）。
///
/// 注意：**会**影响已导出的目录 —— 复用旧目录时按当前前缀就地改名
/// （`try_export_post_product` 的 rename 分支），调这个值等于一次惰性重命名：
/// 只在该帖下次被自动导出时发生，不扫全库、不复制内容；目标名被占用则跳过改名。
pub(super) const EXPORT_TITLE_PREFIX_LEN: usize = 40;

pub(super) fn title_export_prefix(title: &str, n: usize) -> String {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return "未命名".to_string();
    }
    let prefix: String = trimmed.chars().take(n).collect();
    let sanitized = sanitize_fs_name(&prefix);
    if sanitized.is_empty() {
        "未命名".to_string()
    } else {
        sanitized
    }
}

/// 从文本里抠出第一个形如 `YYYY-MM-DD` / `YYYY/MM/DD` / `YYYYMMDD` 的日期，规范为 `YYYYMMDD`。
pub(super) fn first_date_in(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        if chars[i].is_ascii_digit() {
            // YYYYMMDD
            if i + 8 <= n && chars[i..i + 8].iter().all(|c| c.is_ascii_digit()) {
                let s: String = chars[i..i + 8].iter().copied().collect();
                // 年份只做格式校验（月份/日期才有取值范围）
                if let (Ok(_y), Ok(m), Ok(d)) = (
                    s[0..4].parse::<u32>(),
                    s[4..6].parse::<u32>(),
                    s[6..8].parse::<u32>(),
                ) {
                    if (1..=12).contains(&m) && (1..=31).contains(&d) {
                        return Some(s);
                    }
                }
            }
            // YYYY-MM-DD / YYYY/MM/DD
            if i + 10 <= n {
                let b: Vec<char> = chars[i..i + 10].iter().copied().collect();
                if b[4] == '-' || b[4] == '/' {
                    let y: String = b[0..4].iter().copied().collect();
                    let m: String = b[5..7].iter().copied().collect();
                    let d: String = b[8..10].iter().copied().collect();
                    if let (Ok(yy), Ok(mm), Ok(dd)) =
                        (y.parse::<u32>(), m.parse::<u32>(), d.parse::<u32>())
                    {
                        if (1..=12).contains(&mm) && (1..=31).contains(&dd) {
                            return Some(format!("{yy:04}{mm:02}{dd:02}"));
                        }
                    }
                }
            }
        }
        i += 1;
    }
    None
}

/// 毫秒时间戳 → 本地时区 `YYYYMMDD`。
pub(super) fn millis_to_yyyymmdd(ms: i64) -> Option<String> {
    let secs = ms / 1000;
    let dt = match Utc.timestamp_opt(secs, 0) {
        chrono::LocalResult::Single(dt) => dt,
        _ => return None,
    };
    let local = dt.with_timezone(&Local);
    Some(local.format("%Y%m%d").to_string())
}

pub(super) fn today_yyyymmdd() -> String {
    Local::now().format("%Y%m%d").to_string()
}

/// 归档目录的日期：取**发布时间**，不是导出时间。
///
/// 原来恒返回「今天」：同一篇 11/4 的帖子，11/5 导一次落 20261105、11/6 再导落 20261106，
/// 博主没更新的日子每天都会漂出一个新日期目录（实测 11/6 凌晨重抓 11/4 的更新）。
/// 内容只该有一个归档日期；抓不到发布时间才退回当天。
pub(super) fn export_date_yyyymmdd(post: &MonitoredPostRecord) -> String {
    post.published_at
        .as_deref()
        .and_then(first_date_in)
        .unwrap_or_else(today_yyyymmdd)
}

/// 在三级目录（作者）下找「自己的」产物目录（依据 `.beav-post-id` 标记）。
pub(super) fn find_own_export_dir(l3: &Path, post_id: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(l3).ok()?;
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Ok(content) = std::fs::read_to_string(p.join(".beav-post-id")) {
                if content.trim() == post_id {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// 跨日期 / 跨作者名，在「平台」一级目录下找这篇帖子已经导出过的目录。
///
/// 为什么需要它：导出目录里的日期是**导出当天**，同一篇帖子在不同日子被再次自动导出
/// （转写重试成功、重新下载、重新抽帧、重新分析都会触发 `export_post_product`）时会落到
/// 另一个 `日期/作者` 分支，`find_own_export_dir(l3)` 按定义找不到，于是又建一份 ——
/// 素材库里就出现同一篇帖子的两个目录（内容各写一半，谁也删不掉谁）。
/// 作者名被清洗过（老数据 `@抖音科技` → 正确昵称）也会换作者目录，所以这里不按作者名
/// 定位，而是把每个日期目录下的作者目录都翻一遍，只认 `.beav-post-id`。
pub(super) fn find_export_dir_for_post(l1: &Path, post_id: &str) -> Option<PathBuf> {
    let dates = std::fs::read_dir(l1).ok()?;
    for date_entry in dates.flatten() {
        let date_dir = date_entry.path();
        if !date_dir.is_dir() {
            continue;
        }
        let Ok(authors) = std::fs::read_dir(&date_dir) else {
            continue;
        };
        for author_entry in authors.flatten() {
            let author_dir = author_entry.path();
            if !author_dir.is_dir() {
                continue;
            }
            if let Some(found) = find_own_export_dir(&author_dir, post_id) {
                return Some(found);
            }
        }
    }
    None
}

/// 显式导出把写入路径和错误一起返回给调用者。
pub(super) fn write_export_file(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> Result<(), String> {
    let path = path.as_ref();
    std::fs::write(path, contents).map_err(|error| format!("写入 {} 失败：{error}", path.display()))
}

/// 仅在目标缺失或大小变化时才拷贝，避免重复导出时反复复制大文件。
pub(super) fn copy_if_changed(src: &Path, dst: &Path) -> Result<(), String> {
    if let (Ok(sm), Ok(dm)) = (std::fs::metadata(src), std::fs::metadata(dst)) {
        if sm.len() == dm.len() { return Ok(()); }
    }
    std::fs::copy(src, dst).map(|_| ()).map_err(|error|
        format!("复制 {} 到 {} 失败：{error}", src.display(), dst.display()))
}

/// A directory link installed by the migration script opts in to video-only external storage.
pub(super) fn exported_video_path(root: &Path, l4: &Path, post_id: &str, name: &str) -> Result<PathBuf, String> {
    let videos = root.join("导出视频");
    match std::fs::symlink_metadata(&videos) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(l4.join(name)),
        Err(error) => return Err(format!("读取导出视频目录失败：{error}")),
        Ok(meta) if !meta.file_type().is_symlink() => return Ok(l4.join(name)),
        Ok(_) => {}
    }
    // Do not create a replacement directory on the internal disk when the drive is disconnected.
    if !videos.is_dir() {
        return Err("导出视频硬盘未连接或目录不可访问，请连接硬盘后重试".into());
    }
    if post_id.is_empty() || post_id == "." || post_id == ".." || post_id.contains(['/', '\\', ':', '[', ']', '|', '#']) {
        return Err("无效的导出帖子标识".into());
    }
    Ok(videos.join(post_id).join(name))
}

pub(super) fn export_video_embed(root: &Path, l4: &Path, post_id: &str, name: &str, vault: &str) -> Result<String, String> {
    let path = exported_video_path(root, l4, post_id, name)?;
    if path.parent() == Some(l4) {
        return Ok(format!("![[{vault}{name}]]"));
    }
    // The post id makes this vault-relative suffix unique, including hub notes outside the export root.
    Ok(format!("![[导出视频/{post_id}/{name}]]"))
}

/// 真实落盘一篇帖子的「最终产物」到 5 级目录树。best-effort：任何失败都只记日志不阻断主流程。
pub(super) fn export_post_product(state: &State<'_, AppState>, post_id: &str) {
    if let Err(e) = try_export_post_product(state, post_id) {
        println!("[monitor] export_post_product failed for {post_id}: {e}");
    }
}

/// 解析「帖子最终产物」的导出根目录（**始终跟随导出根目录**，不再按平台选知识库）。
///
/// - `monitor_export_root` 非空 → 用它（**不要求目录已存在**，导出时会 `create_dir_all`）；
/// - 否则回退 `workspace_root`（保留平台级一级目录）。
///
/// 注意：早期实现用 `is_dir()` 判断，用户清理过导出目录后（目录被删）会**静默回退到工作区**，
/// 表现为「设置了导出根目录但产物仍落在工作区」。这里改为只认配置值，目录由导出流程按需创建。
pub(super) fn resolve_export_target(state: &State<'_, AppState>) -> Result<PathBuf, String> {
    // 受管模式（Web 多实例）：导出根由实例启动参数固定，忽略 store 里的用户设置。
    if let Ok(raw) = std::env::var("BEAV_EXPORT_ROOT") {
        let raw = raw.trim();
        if !raw.is_empty() {
            return Ok(PathBuf::from(raw));
        }
    }
    let export_root = with_store(state, |store| Ok(store.monitor_export_root.clone()))?;
    let export_root = export_root.trim().to_string();
    if !export_root.is_empty() {
        return Ok(PathBuf::from(&export_root));
    }
    workspace_root(state)
}

/// 拆出标题尾部的 hashtag（抖音标题常把话题直接贴在末尾，如 `…决策者#财务 #cfo`）。
///
/// 返回（清洗后标题, 标签列表）。只有末尾形如 `#token`（token 内无空白）才会被剥离，
/// 因此不会误伤正文中出现的 `#`。
/// 用详情页抓到的干净文案纠正列表页留下的脏标题。
///
/// 背景：抖音**列表页**的 title 是「播放量 + 文案 + 文案重复一遍」拼出来的
/// （`1.8万日本资产泡沫… 日本资产泡沫…`），而**详情页**抓到的 desc 是干净的
/// （`日本资产泡沫…`）。`post_detail` 原本从不更新 title，于是脏标题永久留存，
/// 只能靠事后清洗脚本回洗 —— 而新抓的帖子还会持续产生脏标题。
///
/// 规则（保守，宁可不改）：
/// - 仅当**现有 title 以数字开头**（疑似播放量前缀）才考虑修正；
/// - 详情文案里若**也出现**同一个数字（可隔 ≤4 个非数字字符）→
///   说明那个数字属于文案（`4000亿…` / `30+亿…`），**不能剥**；
/// - 详情文案里**没有**这个数字 → 它是列表页粘上来的播放量前缀，该剥；
///   但剥掉后剩下的文案必须**出现在详情文案里**，否则说明判断错了，不改。
///
/// 详情文案里的「`第8集 | `」这类集数前缀会被剥掉：它属于正文编排，不属于标题。
pub(super) fn corrected_title_from_detail(list_title: &str, detail_text: &str) -> Option<String> {
    let raw = list_title.trim();
    if raw.is_empty() {
        return None;
    }
    // 只处理「以数字开头」的疑似播放量前缀
    let lead: String = raw.chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '+').collect();
    if lead.is_empty() {
        return None;
    }
    // 剥掉数字（及其后的单位）后的剩余文案
    let mut rest = raw[lead.len()..].trim_start();
    for unit in ["万", "w", "W", "k", "K"] {
        if let Some(stripped) = rest.strip_prefix(unit) {
            rest = stripped.trim_start();
            break;
        }
    }
    if rest.len() < 6 {
        return None;
    }
    // 详情文案（剥掉集数前缀）里若**也出现**同一个数字 → 那个数字属于文案
    // （`4000亿…` / `30+亿…` / `2025车圈…`），不能剥。
    let detail = strip_episode_prefix(detail_text);
    if detail.is_empty() {
        return None;
    }
    let digits: String = lead.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
    if digits.is_empty() {
        return None;
    }
    if detail_contains_number_near(&detail, &digits) {
        return None;
    }
    // 详情里**没有**这个数字 → 它是列表页粘上来的播放量前缀，该剥。
    // 但剥掉后剩余文案必须**出现在详情里**，否则说明剥错了 → 不改。
    if !detail.contains(rest) {
        return None;
    }
    Some(rest.to_string())
}

/// 剥掉抖音文案开头的集数前缀：`第8集 | ` / `第 3 期｜` / `EP12 ` 等。
pub(super) fn strip_episode_prefix(raw: &str) -> String {
    let text = raw.trim();
    let bytes: Vec<char> = text.chars().collect();
    // 「第 + 数字 + 集/期 + 可选分隔符」
    if bytes.first().map(|c| *c == '第').unwrap_or(false) {
        let mut idx = 1usize;
        // 数字与「集/期」之间可能有空格：`第 3 期｜…`
        while idx < bytes.len() && (bytes[idx].is_ascii_digit() || bytes[idx].is_whitespace()) {
            idx += 1;
        }
        if idx > 1 && idx < bytes.len() && (bytes[idx] == '集' || bytes[idx] == '期') {
            idx += 1;
            // 跳过可选的分隔符与空白
            while idx < bytes.len() && (bytes[idx].is_whitespace() || matches!(bytes[idx], '|' | '｜' | '·' | '・')) {
                idx += 1;
            }
            return bytes[idx..].iter().collect::<String>().trim().to_string();
        }
    }
    text.to_string()
}

/// 详情文案里是否「紧邻着」（可隔 ≤4 个非数字字符）出现指定数字。
pub(super) fn detail_contains_number_near(detail: &str, digits: &str) -> bool {
    if digits.is_empty() {
        return false;
    }
    let chars: Vec<char> = detail.chars().collect();
    let dchars: Vec<char> = digits.chars().collect();
    if dchars.len() > chars.len() {
        return false;
    }
    for start in 0..=chars.len().saturating_sub(dchars.len()) {
        // 前面最多允许 4 个非数字字符
        let mut back = 0usize;
        while back <= 4 && start > back {
            let c = chars[start - back - 1];
            if c.is_ascii_digit() {
                break;
            }
            back += 1;
        }
        if back > 4 {
            continue;
        }
        if chars[start..start + dchars.len()] == dchars[..] {
            return true;
        }
    }
    false
}

pub(super) fn split_title_tags(raw: &str) -> (String, Vec<String>) {
    let mut tags: Vec<String> = Vec::new();
    let mut head = raw.trim().to_string();
    while let Some(pos) = head.rfind('#') {
        let tail = &head[pos + 1..];
        if tail.is_empty() || tail.contains(char::is_whitespace) || tail.contains('#') {
            break;
        }
        let tag = tail.trim_matches(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'));
        if tag.is_empty() {
            break;
        }
        tags.push(tag.to_string());
        head.truncate(pos);
        head = head.trim_end().to_string();
    }
    tags.reverse();
    (head.trim().to_string(), tags)
}

/// 生成 YAML 双引号标量（转义 `"` 与 `\`，换行压成空格）。
pub(super) fn yaml_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' => out.push(' '),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// 生成 Obsidian 知识卡 `knowledge.md`（旧名 `Beav知识卡.md`，写新版时会顺手删掉旧的）。
///
/// 设计要点（2026-10-27 重写）：
/// - **正文 / 语音转写直接内联**：Obsidian 不渲染 `.txt` / `.srt` / `.json`，
///   早期用 `![[转写全文]]` 会变成死链，因此改为把文本写进笔记本身。
/// - **关键帧用 `![](文件)` 图片引用而非 `![[...]]`**：只显示、不生成图谱边，
///   避免一篇帖子往图谱里连出上百条边。
/// - 元数据进 YAML frontmatter（作者/平台/赞藏评/来源/标签），hashtag 从标题剥离后转成 `tags`。
///
/// - 始终在产物目录 `l4` 写一份（prefix 为空，同目录相对引用，整包可移植）。
/// - 若配置了 `monitor_obsidian_vault` 且产物目录在其内，另在
///   `<vault>/Beav导出/<l4 相对导出根的路径>/` 写一份（prefix 为 vault 相对路径）。
///
/// best-effort：任何失败只记日志，不阻断导出主流程。
pub(super) fn write_obsidian_card(
    state: &State<'_, AppState>,
    root: &Path,
    l4: &Path,
    post: &MonitoredPostRecord,
    analysis: &Option<PostVideoAnalysisRecord>,
    decon: &Option<crate::PostDeconstructionRecord>,
) -> Result<(), String> {
    let (title, tags) = split_title_tags(post.title.trim());
    let title = if title.is_empty() { "未命名".to_string() } else { title };

    let ext_of = |p: &str| -> String {
        Path::new(p)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg")
            .to_lowercase()
    };

    // 只列实际存在的产物（导出时对缺失源会 continue），避免断链
    let mut images: Vec<String> = Vec::new();
    for (idx, img) in post.images.iter().enumerate() {
        if Path::new(img).exists() {
            images.push(format!("图片_{}.{}", idx + 1, ext_of(img)));
        }
    }
    let mut frames: Vec<String> = Vec::new();
    if let Some(a) = analysis {
        for (idx, fr) in a.frames.iter().enumerate() {
            if Path::new(fr).exists() {
                frames.push(format!("抽帧_{:03}.{}", idx + 1, ext_of(fr)));
            }
        }
    }
    let video_src = post
        .video_preview_path
        .as_ref()
        .filter(|p| Path::new(p).exists())
        .or_else(|| post.video_local_path.as_ref().filter(|p| Path::new(p).exists()));
    let video_name = video_src.map(|p| format!("视频.{}", ext_of(p)));
    let external_video_embed = video_name.as_ref()
        .map(|name| export_video_embed(root, l4, &post.id, name, ""))
        .transpose()?
        .filter(|embed| embed.starts_with("![[导出视频/"));

    let body = post.content.trim();
    let transcript = analysis
        .as_ref()
        .map(|a| a.transcript.trim().to_string())
        .unwrap_or_default();
    let summary = analysis
        .as_ref()
        .map(|a| a.summary.trim().to_string())
        .unwrap_or_default();
    let srt = analysis
        .as_ref()
        .map(|a| a.transcript_srt.trim().to_string())
        .unwrap_or_default();

    if body.is_empty()
        && transcript.is_empty()
        && frames.is_empty()
        && images.is_empty()
        && decon.is_none()
        && video_name.is_none()
    {
        return Ok(());
    }

    let platform = platform_label(&post.platform);
    let published = post.published_at.as_deref().unwrap_or("").trim().to_string();
    let url = post.post_url.trim().to_string();

    // `rel`：相对当前笔记所在目录的路径前缀（HTML `<img>` / Markdown 链接用）
    // `vault`：vault 根相对路径前缀（`![[wikilink]]` 用，Obsidian 的 wikilink 始终从库根解析）
    // 产物目录那份两者都是空串；库内 hub 那份 `rel` 是 `../../…/`、`vault` 是库内路径。
    let render = |rel: &str, vault: &str| -> String {
        let path_of = |name: &str| format!("{rel}{name}");
        // 配图 / 关键帧都走原生 `![|200](path)`：Live Preview 下点击只打开预览、不露源码
        // （旧的内联 HTML `<img>` 会露源码）；附件默认不进图谱，不产生图谱边。
        // 关键帧用「默认折叠的 callout」承载，替代 `<details>`（HTML 块内 markdown 不解析）。
        let md_img = |name: &str| format!("![|200]({})", path_of(name));

        let mut md = String::new();

        // ---- YAML frontmatter ----
        md.push_str("---\n");
        md.push_str(&format!("title: {}\n", yaml_str(&title)));
        md.push_str(&format!(
            "author: {}\n",
            yaml_str(&sanitize_fs_name(&post.author))
        ));
        md.push_str(&format!("platform: {}\n", yaml_str(&platform)));
        md.push_str(&format!("post_id: {}\n", yaml_str(&post.id)));
        if !url.is_empty() {
            md.push_str(&format!("url: {}\n", yaml_str(&url)));
        }
        if !published.is_empty() {
            md.push_str(&format!("published: {}\n", yaml_str(&published)));
        }
        md.push_str(&format!("likes: {}\n", post.likes));
        md.push_str(&format!("collects: {}\n", post.collects));
        md.push_str(&format!("comments: {}\n", post.comments));
        if !tags.is_empty() {
            let list: Vec<String> = tags.iter().map(|t| yaml_str(t)).collect();
            md.push_str(&format!("tags: [{}]\n", list.join(", ")));
        }
        md.push_str("source: beav-monitor\n");
        md.push_str("---\n\n");

        // ---- 摘要行 ----
        md.push_str(&format!("# {title}\n\n"));
        md.push_str(&format!(
            "> **{}** · {} · 赞 {} / 藏 {} / 评 {}\n",
            sanitize_fs_name(&post.author),
            platform,
            post.likes,
            post.collects,
            post.comments
        ));
        if !url.is_empty() {
            md.push_str(&format!("> [查看原帖]({url})\n"));
        }
        md.push('\n');

        // ---- 正文（内联） ----
        if !body.is_empty() {
            md.push_str("## 正文\n\n");
            md.push_str(body);
            md.push_str("\n\n");
        }

        // ---- 语音转写（内联，折叠） ----
        if !transcript.is_empty() {
            md.push_str("## 语音转写\n\n");
            md.push_str(&format!(
                "<details>\n<summary>展开全文（{} 字）</summary>\n\n",
                transcript.chars().count()
            ));
            // 与 转写全文.txt 同一套归一化：ASR 的逐句换行直接内联会断在词中间
            md.push_str(&normalize_transcript_text(&transcript));
            md.push_str("\n\n</details>\n\n");
        }
        // ---- 逐句时间轴：直接展开显示 ----
        // `.srt` 不是 Obsidian 支持的嵌入格式，原来的链接点开只会甩给外部播放器；
        // 这里把文本内联进代码块（折叠），一眼能看，也不产生图谱边。
        if !srt.is_empty() {
            let cues = srt.lines().filter(|line| line.contains("-->")).count();
            md.push_str("### 逐句时间轴\n\n");
            md.push_str(&format!(
                "<details>\n<summary>展开全文（{cues} 条）</summary>\n\n```\n"
            ));
            md.push_str(&srt);
            md.push_str("\n```\n\n</details>\n\n");
        }
        if !summary.is_empty() {
            md.push_str("## 视频摘要\n\n");
            md.push_str(&summary);
            md.push_str("\n\n");
        }

        // ---- 关键帧（折叠 + 图片引用，不产生图谱边） ----
        // 用「默认折叠的 callout」而非 `<details>`：callout 内容仍是 markdown，图片走原生
        // `![|200]()`（Live Preview 点击只开预览、不露源码）；`<details>` 是 HTML 块，块内
        // markdown 不解析，只能用 HTML `<img>`，点开即露源码。折叠 = 默认不展开，保持轻量。
        if !frames.is_empty() {
            md.push_str(&format!("## 关键帧（{} 张）\n\n", frames.len()));
            md.push_str(&format!(
                "> [!abstract]- 展开全部关键帧（{} 张）\n",
                frames.len()
            ));
            for chunk in frames.chunks(4) {
                let row: Vec<String> = chunk.iter().map(|f| md_img(f)).collect();
                md.push_str(&format!("> {}\n", row.join(" ")));
            }
            md.push('\n');
        }

        // ---- 配图 ----
        if !images.is_empty() {
            md.push_str("## 配图\n\n");
            for chunk in images.chunks(4) {
                let row: Vec<String> = chunk.iter().map(|f| md_img(f)).collect();
                md.push_str(&row.join(" "));
                md.push_str("\n\n");
            }
        }

        // ---- 爆文拆解（.md，wikilink 可正常嵌入） ----
        if decon.is_some() {
            md.push_str("## 爆文拆解\n\n");
            md.push_str(&format!("![[{vault}爆文拆解]]\n\n"));
        }

        // ---- 视频（折叠，避免一开笔记就加载大文件） ----
        if let Some(v) = &video_name {
            // 视频嵌入**不能**放进 `<details>`：Obsidian 不解析 HTML 块里的 wikilink，
            // 会把 `![[视频.mp4]]` 当纯文本原样显示（实测）。图片用 HTML `<img>` 没事，
            // 但 `![[...]]` 是 Obsidian 自己的语法，必须裸放在正文里。
            md.push_str("## 视频\n\n");
            let local_embed = format!("![[{vault}{v}]]");
            let embed = external_video_embed.as_deref().unwrap_or(&local_embed);
            md.push_str(&format!("{embed}\n\n"));
        }

        md.push_str("---\n\n");
        md.push_str(&format!(
            "- 完整字段：[帖子信息.json]({})\n",
            path_of("帖子信息.json")
        ));
        md
    };

    // 1) 始终写在产物目录（portable，同目录相对引用）
    let note_path = l4.join("knowledge.md");
    // 旧版本叫 `Beav知识卡.md`：改名后清掉旧文件，避免同一目录出现两张卡（都是生成物，可安全删）
    let legacy_note = l4.join("Beav知识卡.md");
    if legacy_note.is_file() {
        let _ = std::fs::remove_file(&legacy_note);
    }
    write_export_file(&note_path, render("", ""))?;

    // 2) 配置了资料库：产物在库内 → 额外在库内 hub 再写一份（vault 相对引用）
    let vault = with_store(state, |s| Ok(s.monitor_obsidian_vault.clone()))
        .unwrap_or_default();
    let vault = vault.trim().to_string();
    if vault.is_empty() {
        return Ok(());
    }
    let vault_path = PathBuf::from(&vault);
    if !vault_path.is_dir() {
        return Err(format!("Obsidian 资料库不是有效目录：{vault}"));
    }
    let (l4_vault_rel, l4_root_rel) =
        match (l4.strip_prefix(&vault_path), l4.strip_prefix(root)) {
            (Ok(v), Ok(r)) => (v, r),
            _ => {
                println!(
                    "[monitor] 导出目录不在 Obsidian 资料库内（vault={vault}），无法在库内生成可解析的笔记；\
                     建议把「监控导出目标」设为该资料库内的目录。"
                );
                return Ok(());
            }
        };
    let hub_dir = vault_path.join("Beav导出").join(l4_root_rel);
    std::fs::create_dir_all(&hub_dir).map_err(|error| format!("创建 Obsidian 目录失败：{error}"))?;
    // hub 笔记在 <vault>/Beav导出/<相对路径>/，产物在 <vault>/<导出根相对路径>/<相对路径>/
    // wikilink 用库根相对路径；HTML/Markdown 相对引用要回退到库根再下去
    let vault_prefix = format!("{}/", l4_vault_rel.to_string_lossy().replace('\\', "/"));
    let rel_prefix = {
        let depth = 1 + l4_root_rel.components().count();
        let mut p = "../".repeat(depth);
        p.push_str(&l4_vault_rel.to_string_lossy().replace('\\', "/"));
        p.push('/');
        p
    };
    let legacy_hub = hub_dir.join("Beav知识卡.md");
    if legacy_hub.is_file() {
        let _ = std::fs::remove_file(&legacy_hub);
    }
    let hub_note = hub_dir.join("knowledge.md");
    write_export_file(&hub_note, render(&rel_prefix, &vault_prefix))?;
    Ok(())
}

/// 把 ASR 转写里的「按句硬换行」折叠掉，只保留空行分段。
///
/// ASR 每句都带 `\n`，直接落盘成 `转写全文.txt` 后，在编辑器/Markdown 里渲染
/// 就是断在词中间的锯齿（实测 "Global⏎Bot"、"OPS⏎5.5"）。
/// 段内合并：中文直接相连、西文补一个空格；`转写时间轴.srt` 不动（换行是结构）。
pub(super) fn normalize_transcript_text(text: &str) -> String {
    let is_cjk = |ch: char| matches!(ch, '\u{2e80}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}');
    text.replace("\r\n", "\n")
        .split("\n\n")
        .map(|paragraph| {
            let lines: Vec<&str> = paragraph
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect();
            let mut merged = String::new();
            for line in lines {
                let need_space = match (merged.chars().last(), line.chars().next()) {
                    (Some(last), Some(first)) => !is_cjk(last) && !is_cjk(first),
                    _ => false,
                };
                if need_space {
                    merged.push(' ');
                }
                merged.push_str(line);
            }
            merged
        })
        .filter(|paragraph| !paragraph.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(super) fn try_export_post_product(
    state: &State<'_, AppState>,
    post_id: &str,
) -> Result<(), String> {
    let post = with_store(state, |store| {
        Ok(store
            .monitored_posts
            .iter()
            .find(|p| p.id == post_id)
            .cloned())
    })?;
    let post = match post {
        Some(p) => p,
        None => return Err("动态不存在".to_string()),
    };
    let analysis = with_store(state, |store| {
        Ok(store
            .post_video_analyses
            .iter()
            .find(|a| a.post_id == post_id)
            .cloned())
    })?;
    let decon = with_store(state, |store| {
        Ok(store
            .post_deconstructions
            .iter()
            .find(|d| d.post_id == post_id)
            .cloned())
    })?;

    // 导出根 → 平台 → 日期 → 作者 → 标题前 20 字
    //
    // 注意 `date` 取的是**导出当天**（不是发布时间），所以同一篇帖子在不同日子被
    // 自动导出（转写重试成功、重新下载、重新抽帧都会触发）会算出一组全新的 l3。
    // 只用 `find_own_export_dir(l3)` 去重的话跨天必然找不到 → 再建一份同名素材。
    // 所以这里按 `.beav-post-id` 在**整个平台目录**下横向找一遍，先复用再说。
    let root = resolve_export_target(state)?;
    let date = export_date_yyyymmdd(&post);
    let author = {
        let a = sanitize_fs_name(&post.author);
        if a.is_empty() {
            "未知作者".to_string()
        } else {
            a.chars().take(32).collect()
        }
    };
    let l1 = root.join(platform_label(&post.platform));
    let l2 = l1.join(&date);
    let l3 = l2.join(&author);

    let title_prefix = title_export_prefix(&post.title, EXPORT_TITLE_PREFIX_LEN);
    // 先看当天当天作者（快路径），再横向扫平台目录下的历史日期 / 历史作者名。
    let l4 = match find_own_export_dir(&l3, post_id)
        .or_else(|| find_export_dir_for_post(&l1, post_id))
    {        Some(existing) => {
            // 复用旧目录时标题可能已被清洗/纠正：沿用旧名会让素材库一直顶着脏标题，
            // 所以在同一个作者目录内就地改名。目标名已被占用（别的帖子同名前缀）时保持不动，
            // 宁可留旧名也不覆盖别人的目录。
            let renamed = match existing.file_name().and_then(|name| name.to_str()) {
                Some(current) if current != title_prefix => existing
                    .parent()
                    .map(|parent| parent.join(&title_prefix))
                    .filter(|target| !target.exists())
                    .and_then(|target| std::fs::rename(&existing, &target).ok().map(|_| target)),
                _ => None,
            };
            renamed.unwrap_or(existing)
        }
        None => {
            let candidate = l3.join(&title_prefix);
            if candidate.exists() {
                l3.join(format!("{title_prefix}_{post_id}"))
            } else {
                candidate
            }
        }
    };
    std::fs::create_dir_all(&l4).map_err(|e| format!("创建产物目录失败：{e}"))?;
    write_export_file(l4.join(".beav-post-id"), post_id)?;


    // 原图
    for (idx, img) in post.images.iter().enumerate() {
        let src = Path::new(img);
        if !src.exists() {
            continue;
        }
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg")
            .to_lowercase();
        let dst = l4.join(format!("图片_{}.{}", idx + 1, ext));
        copy_if_changed(src, &dst)?;
    }

    // 抽帧图 + 视频分析文本
    if let Some(a) = &analysis {
        for (idx, fr) in a.frames.iter().enumerate() {
            let src = Path::new(fr);
            if !src.exists() {
                continue;
            }
            let ext = src
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("jpg")
                .to_lowercase();
            let dst = l4.join(format!("抽帧_{:03}.{}", idx + 1, ext));
            copy_if_changed(src, &dst)?;
        }
        if !a.transcript.trim().is_empty() {
            write_export_file(l4.join("转写全文.txt"), normalize_transcript_text(&a.transcript))?;
        }
        if !a.transcript_srt.trim().is_empty() {
            write_export_file(l4.join("转写时间轴.srt"), &a.transcript_srt)?;
        }
        if !a.summary.trim().is_empty() {
            write_export_file(l4.join("视频摘要.txt"), &a.summary)?;
        }
    }

    // 正文
    if !post.content.trim().is_empty() {
        write_export_file(l4.join("正文.txt"), &post.content)?;
    }

    // 视频本体：只导出一份 `视频.mp4`。
    // 优先轻量预览（720p/H.264/faststart）；但**预览只在需要转码时才生成**——
    // H.264 ≤1080p 的原片直接可播，`video_preview_path` 为空，此时回退原片，
    // 否则这类帖子会导出成「没有视频」。
    let video_src = post
        .video_preview_path
        .as_ref()
        .filter(|p| Path::new(p).exists())
        .or(post.video_local_path.as_ref());
    if let Some(vp) = video_src {
        let src = Path::new(vp);
        if src.exists() {
            let ext = src
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("mp4")
                .to_lowercase();
            let dst = exported_video_path(&root, &l4, post_id, &format!("视频.{ext}"))?;
            if dst.parent() != Some(l4.as_path()) {
                let dir = dst.parent().ok_or("导出视频路径无效")?;
                std::fs::create_dir_all(dir).map_err(|error| format!("创建导出视频目录失败：{error}"))?;
                write_export_file(dir.join(".beav-post-id"), post_id)?;
            }
            copy_if_changed(src, &dst)?;
        }
    }

    // 爆文拆解（六要素 + 结构骨架）
    if let Some(d) = &decon {
        let json = serde_json::to_string_pretty(d).unwrap_or_default();
        write_export_file(l4.join("爆文拆解.json"), json)?;
        let bullets = |items: &[String]| -> String {
            if items.is_empty() {
                "（无）".to_string()
            } else {
                items
                    .iter()
                    .map(|item| format!("- {item}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        };
        // 结构骨架单独成表：时间码是拆解里最有用、也最容易被编造的字段，
        // 摆在明面上才能一眼看出模型有没有照着真实字幕走。
        let structure_md = if d.structure.is_empty() {
            "（本条没有可用的时间轴结构）".to_string()
        } else {
            let mut table =
                String::from("| 时间码 | 段落功能 | 占比 | 这一段做了什么 |\n| --- | --- | --- | --- |\n");
            for segment in &d.structure {
                table.push_str(&format!(
                    "| {} | {} | {} | {} |\n",
                    if segment.start.trim().is_empty() {
                        "—"
                    } else {
                        segment.start.trim()
                    },
                    segment.role,
                    segment
                        .weight
                        .map(|weight| format!("{weight:.0}%"))
                        .unwrap_or_else(|| "—".to_string()),
                    segment.gist
                ));
            }
            table
        };
        let md = format!(
            "# 爆文拆解 · {}\n\n作者：{}\n\n> 拆解依据：{}\n\n## 选题角度\n{}\n\n## 文案结构\n{}\n\n## 结构骨架\n{}\n\n## 开场白 / 前3秒钩子\n{}\n\n## 标题句式\n{}\n\n## 金句 / 钩子\n{}\n\n## 可复用方法论\n{}\n\n---\n\n### 拆解所用原文\n{}\n",
            d.post_title,
            d.author,
            d.source_evidence,
            d.topic_angle,
            d.copy_structure,
            structure_md,
            d.opening,
            bullets(&d.title_patterns),
            bullets(&d.hooks),
            d.reusable_takeaway,
            d.raw_content
        );
        write_export_file(l4.join("爆文拆解.md"), md)?;
    }

    // 二创产出（分镜脚本 / 各平台改写）：落到「二创/」子目录，和拆解并列
    let remixes: Vec<crate::PostRemixRecord> = with_store(state, |store| {
        Ok(store
            .post_remixes
            .iter()
            .filter(|item| item.post_id == post_id)
            .cloned()
            .collect())
    })?;
    if !remixes.is_empty() {
        let remix_dir = l4.join("二创");
        std::fs::create_dir_all(&remix_dir).map_err(|error| format!("创建二创目录失败：{error}"))?;
        {
            for item in &remixes {
                let file_name = if item.kind == "rewrite" {
                    format!(
                        "改写-{}.md",
                        if item.target.trim().is_empty() {
                            "default"
                        } else {
                            item.target.trim()
                        }
                    )
                } else {
                    "分镜脚本.md".to_string()
                };
                let label = if item.kind == "rewrite" { "多平台改写" } else { "分镜脚本" };
                let body = format!(
                    "# {label} · {}\n\n> 生成依据：{}\n\n{}\n",
                    item.post_title, item.source_evidence, item.content
                );
                write_export_file(remix_dir.join(file_name), body)?;
            }
        }
    }

    // 帖子信息（溯源）
    let meta = json!({
        "postId": post.id,
        "platform": post.platform,
        "author": post.author,
        "title": post.title,
        "postUrl": post.post_url,
        "mediaType": post.media_type,
        "publishedAt": post.published_at,
        "createdAt": post.created_at,
        "likes": post.likes,
        "comments": post.comments,
        "collects": post.collects,
        "deconstructed": post.deconstructed,
        "videoLocalPath": post.video_local_path,
        "videoPreviewPath": post.video_preview_path,
        "hasVideoAnalysis": analysis.is_some(),
        "hasDeconstruction": decon.is_some(),
        "exportedAt": now_i64(),
    });
    write_export_file(
        l4.join("帖子信息.json"),
        serde_json::to_string_pretty(&meta).unwrap_or_default(),
    )?;

    // 生成 Obsidian 聚合笔记（「知识卡」），用 ![[嵌入]] 关联所有产物
    write_obsidian_card(state, &root, &l4, &post, &analysis, &decon)?;

    println!("[monitor] exported post product -> {}", l4.display());
    Ok(())
}

/// 读取全局「导出根目录」（`monitor:get-export-root`）。
///
/// 返回 `{ success, root }`；`root` 为空表示跟随工作区根。
pub(super) fn get_export_root(state: &State<'_, AppState>) -> Result<Value, String> {
    let root = with_store(state, |store| Ok(store.monitor_export_root.clone()))?;
    let workspace = workspace_root(state)?;
    Ok(json!({ "success": true, "root": root,
        "effectiveRoot": resolve_export_target(state)?,
        "videoCacheRoot": workspace.join("monitor-videos"),
        "externalVideoStorageVersion": 1 }))
}

/// 设置全局「导出根目录」（`monitor:set-export-root`）。
///
/// 入参：`root`（绝对目录；空串 / null 表示清空、回退工作区根）。
pub(super) fn set_export_root(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    if crate::managed_dirs_locked() {
        return Err("导出目录由服务器统一管理（Web 多用户模式），不能在客户端修改".to_string());
    }
    let root = str_field(payload, "root")
        .or_else(|| str_field(payload, "dir"))
        .unwrap_or_default();
    let root = root.trim().to_string();
    // 目录不存在时直接创建（用户可能选了一个稍后被删掉/尚未创建的目录），
    // 创建失败才报错，避免「存了设置但导出静默回退工作区」。
    if !root.is_empty() {
        std::fs::create_dir_all(&root).map_err(|e| format!("创建导出根目录失败：{root}（{e}）"))?;
    }
    with_store_mut(state, |store| {
        store.monitor_export_root = root.clone();
        Ok(json!({ "success": true, "root": root }))
    })
}

/// 弹出系统目录选择器，返回所选目录（`monitor:pick-export-root`）。
pub(super) fn pick_export_root() -> Result<Value, String> {
    if crate::managed_dirs_locked() {
        return Err("导出目录由服务器统一管理（Web 多用户模式），不能在客户端选择".to_string());
    }
    let selected = crate::pick_files_native("选择监控导出根目录", true, false)?;
    let path = selected.first().map(|item| item.display().to_string());
    Ok(json!({
        "success": path.is_some(),
        "canceled": path.is_none(),
        "path": path,
    }))
}

/// 读取全局「Obsidian 资料库路径」（`monitor:get-obsidian-vault`）。
///
/// 返回 `{ success, vault }`；`vault` 为空表示未配置（导出只生成可移植的聚合笔记，
/// 需用户自行把导出目录拖进 vault 才会产生图谱边）。
pub(super) fn get_obsidian_vault(state: &State<'_, AppState>) -> Result<Value, String> {
    with_store(state, |store| {
        Ok(json!({ "success": true, "vault": store.monitor_obsidian_vault.clone() }))
    })
}

/// 设置全局「Obsidian 资料库路径」（`monitor:set-obsidian-vault`）。
///
/// 入参：`vault`（绝对目录；空串 / null 表示清空）。非空时必须存在且为目录。
pub(super) fn set_obsidian_vault(state: &State<'_, AppState>, payload: &Value) -> Result<Value, String> {
    let vault = str_field(payload, "vault")
        .or_else(|| str_field(payload, "dir"))
        .unwrap_or_default();
    let vault = vault.trim().to_string();
    if !vault.is_empty() && !Path::new(&vault).is_dir() {
        return Err(format!("Obsidian 资料库路径不存在：{vault}"));
    }
    with_store_mut(state, |store| {
        store.monitor_obsidian_vault = vault.clone();
        Ok(json!({ "success": true, "vault": vault }))
    })
}

/// 弹出系统目录选择器，返回所选 Obsidian 资料库目录（`monitor:pick-obsidian-vault`）。
pub(super) fn pick_obsidian_vault() -> Result<Value, String> {
    let selected = crate::pick_files_native("选择 Obsidian 资料库（.obsidian 所在目录）", true, false)?;
    let path = selected.first().map(|item| item.display().to_string());
    Ok(json!({
        "success": path.is_some(),
        "canceled": path.is_none(),
        "path": path,
    }))
}

/// 「分析/转写」完成后删除 monitor-videos 里的原片，仅保留预览。**默认行为，不再提供开关**。
///
/// 唯一守卫：**必须存在独立预览文件**且与原片不是同一路径 —— 否则删掉就没视频了
/// （H.264 ≤1080p 的帖子不生成预览，直接复用原片播放）。
///
/// 删后把 `video_local_path` 与分析的 `local_path` 都改指预览，保证播放与「重新分析」仍可用。
pub(super) fn delete_original_after_analysis(state: &State<'_, AppState>, post_id: &str) {
    let post = match with_store(state, |store| {
        Ok(store
            .monitored_posts
            .iter()
            .find(|p| p.id == post_id)
            .cloned())
    }) {
        Ok(Some(p)) => p,
        _ => return,
    };
    let (Some(local), Some(preview)) = (
        post.video_local_path.clone(),
        post.video_preview_path.clone(),
    ) else {
        return;
    };
    if local == preview {
        return;
    }
    let (lp, pp) = (Path::new(&local), Path::new(&preview));
    if !lp.exists() || !pp.exists() {
        return;
    }
    let preview_size = std::fs::metadata(pp).map(|m| m.len() as i64).unwrap_or(0);
    match std::fs::remove_file(lp) {
        Ok(()) => {
            let _ = with_store_mut(state, |store| {
                if let Some(slot) = store.monitored_posts.iter_mut().find(|p| p.id == post_id) {
                    slot.video_local_path = Some(preview.clone());
                    if preview_size > 0 {
                        slot.video_size = Some(preview_size);
                    }
                    slot.updated_at = now_i64().to_string();
                }
                if let Some(rec) = store
                    .post_video_analyses
                    .iter_mut()
                    .find(|a| a.post_id == post_id)
                {
                    rec.local_path = preview.clone();
                    rec.updated_at = now_i64();
                }
                Ok(())
            });
            println!("[monitor] deleted original video for {post_id}, kept preview: {preview}");
        }
        Err(e) => println!("[monitor] delete original video failed for {post_id}: {e}"),
    }
}

/// 手动触发单篇帖子产物导出（`monitor:export-post`）。
pub(super) fn export_post_command(
    state: &State<'_, AppState>,
    payload: &Value,
) -> Result<Value, String> {
    let post_id = str_field(payload, "postId")
        .or_else(|| str_field(payload, "post_id"))
        .ok_or_else(|| "缺少 postId".to_string())?;
    try_export_post_product(state, &post_id)?;
    Ok(json!({ "success": true, "postId": post_id }))
}
