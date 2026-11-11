use super::*;

pub(super) fn chrome_status(_app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("default");
    let path = headless_browser::detect_chrome_path();
    let port = headless_browser::active_chrome_port(platform);
    // 必须校验端口是否还活着：用户手动关掉 Chrome 后端口号仍留在会话表里，
    // 不校验就会一直显示"已就绪"，实际抓取全失败。
    let alive = port != 0 && headless_browser::chrome_session_alive(platform);
    let ready_platforms = headless_browser::ready_platforms();
    Ok(json!({
        "chromeInstalled": path.is_some(),
        "chromePath": path.as_ref().map(|p| p.display().to_string()),
        "platform": platform,
        "debugPort": if alive { port } else { 0 },
        "debugReady": alive,
        "readyPlatforms": ready_platforms,
    }))
}

/// 扫码登录：打开登录页并截图二维码，返回 data URL 给前端显示。
///
/// 支持小红书 / 抖音 / B 站（都用手机对应 App 扫官方二维码）。
///
/// 保护：服务器上的平台账号是**所有用户共享**的 —— 任何人一扫码就等于把
/// 服务器换成他自己的账号（把原来的会话顶掉）。所以已登录时默认拒绝，
/// 需要显式 `force` 才允许更换。
pub(super) fn chrome_qr_start(app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("xiaohongshu");
    let spec = headless_browser::qr_platform_spec(platform)
        .ok_or_else(|| "暂时只支持小红书 / 抖音 / B 站扫码登录".to_string())?;
    let force = payload
        .get("force")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut port = headless_browser::active_chrome_port(platform);
    if port == 0 {
        chrome_launch(
            app,
            &json!({ "platform": platform, "url": spec.login_url }),
        )?;
        port = headless_browser::active_chrome_port(platform);
    }
    if port == 0 {
        return Err("Chrome 未就绪（启动失败）".to_string());
    }
    let ws_url = tauri::async_runtime::block_on(headless_browser::ws_url_for_port(port))?;

    // 已登录 → 拒绝扫码，避免把服务器上共享的账号顶掉。
    if !force {
        let already = tauri::async_runtime::block_on(headless_browser::login_logged_in(
            &ws_url, platform,
        ))
        .unwrap_or(false);
        if already {
            return Err(
                "服务器已登录该平台（账号对所有用户共享）。如需更换账号，请先在桌面端「登录管理」点「退出登录」后再扫码。"
                    .to_string(),
            );
        }
    }

    let capture = tauri::async_runtime::block_on(headless_browser::capture_login_qr(
        &ws_url, &spec,
    ))?;
    let image = capture
        .data_url
        .ok_or_else(|| "未能获取二维码图像".to_string())?;
    Ok(json!({
        "success": true,
        "image": image,
        "port": port,
        // 诊断信息：便于定位"扫出来是过期码"这类问题
        "kind": capture.kind,
        "refreshed": capture.refreshed,
        "expired": capture.expired,
        "snippet": capture.snippet,
        // 小红书风控态：需要扫弹窗里的"请通过验证"二维码
        "needVerify": capture.need_verify,
    }))
}

/// 查询扫码登录是否完成（看平台会话 cookie）。
pub(super) fn chrome_qr_status(_app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("xiaohongshu");
    // refresh=true：跳过缓存强制重检（用户/管理员刚在服务器上手动登录完，想立即确认）
    let refresh = payload
        .get("refresh")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if refresh {
        headless_browser::invalidate_login_state(platform);
    }
    let port = headless_browser::active_chrome_port(platform);
    if port == 0 {
        return Ok(json!({ "success": true, "loggedIn": false, "note": "Chrome 未启动" }));
    }
    let ws_url = tauri::async_runtime::block_on(headless_browser::ws_url_for_port(port))?;
    let logged_in =
        tauri::async_runtime::block_on(headless_browser::login_logged_in(&ws_url, platform))?;
    Ok(json!({ "success": true, "loggedIn": logged_in }))
}

/// 登录态诊断：把服务器 Chrome 里该平台页面的真实状态取回来（只回 cookie 名，不回值）。
pub(super) fn chrome_login_debug(_app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("xiaohongshu");
    let port = headless_browser::active_chrome_port(platform);
    if port == 0 {
        return Err("Chrome 未启动".to_string());
    }
    let ws_url = tauri::async_runtime::block_on(headless_browser::ws_url_for_port(port))?;
    tauri::async_runtime::block_on(headless_browser::login_debug(&ws_url, platform))
}

pub(super) fn chrome_launch(app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("default");
    let login_url = payload
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("https://www.xiaohongshu.com");
    let chrome_path = headless_browser::detect_chrome_path()
        .ok_or_else(|| "未检测到 Chrome / Edge / Brave，请先安装 Google Chrome".to_string())?;
    let profile = headless_browser::profile_dir_for(app, platform)?;
    let port = headless_browser::launch_chrome_for_login(&chrome_path, &profile, platform, login_url)?;
    Ok(json!({
        "success": true,
        "platform": platform,
        "debugPort": port,
        "profileDir": profile.display().to_string(),
        "note": format!("已为你打开登录页，请在弹出的 Chrome 中完成登录。每个平台有独立 profile（cookie 隔离）。{} 不会保存你的密码，只复用这次登录的 cookie。", app_brand_display_name()),
    }))
}

pub(super) fn chrome_test(_app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("default");
    // 同步壳：进入 tokio runtime 执行 CDP 测试（ipc_invoke 已在 spawn_blocking 内，安全）。
    let port = headless_browser::active_chrome_port(platform);
    if port == 0 {
        return Err(format!(
            "「{platform}」的 Chrome 调试端口未启动，请先在该平台点「打开 Chrome 登录」完成登录"
        ));
    }
    let target_url = payload
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or("https://www.xiaohongshu.com");
    // 默认测试脚本：取页面标题，并用页面文本粗判是否仍处于「未登录」态（登录页通常有「登录/扫码」字样）。
    let expression = payload
        .get("expression")
        .and_then(Value::as_str)
        .unwrap_or(
            r#"
            (() => {
                const t = document.title;
                const txt = (document.body && document.body.innerText) || '';
                const loggedOut = /登录|扫码|sign[\s_-]?in|未登录|扫码登录/i.test(txt)
                    && !/(个人主页|消息|创作中心|creator|我的主页)/i.test(txt);
                return { title: t, loggedOut: loggedOut };
            })()
            "#,
        );
    let port_arg = port as u16;
    let result = tauri::async_runtime::block_on(async move {
        let ws_url = headless_browser::ws_url_for_port(port_arg).await?;
        headless_browser::navigate_and_eval(&ws_url, target_url, expression).await
    });
    result.map(|value| json!({ "success": true, "platform": platform, "result": value }))
}

/// 导出该平台的登录 cookie（跨机器迁移登录态用，绕开平台风控验证）。
pub(super) fn chrome_export_cookies(_app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("xiaohongshu");
    let port = headless_browser::active_chrome_port(platform);
    if port == 0 {
        return Err("Chrome 未启动：请先「打开 Chrome 登录」并完成登录".to_string());
    }
    let ws_url = tauri::async_runtime::block_on(headless_browser::ws_url_for_port(port))?;
    tauri::async_runtime::block_on(headless_browser::export_login_cookies(&ws_url, platform))
}

/// 导入登录 cookie（把另一台机器导出的 JSON 写进服务器 Chrome）。
pub(super) fn chrome_import_cookies(_app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .unwrap_or("xiaohongshu");
    let cookies = payload
        .get("cookies")
        .and_then(Value::as_array)
        .ok_or_else(|| "缺少 cookies 数组".to_string())?;
    let port = headless_browser::active_chrome_port(platform);
    if port == 0 {
        return Err("Chrome 未启动：请先「打开 Chrome 登录」把 Chrome 拉起来再导入".to_string());
    }
    let ws_url = tauri::async_runtime::block_on(headless_browser::ws_url_for_port(port))?;
    let count =
        tauri::async_runtime::block_on(headless_browser::import_login_cookies(&ws_url, cookies))?;
    Ok(json!({ "success": true, "imported": count }))
}

pub(super) fn chrome_shutdown(app: &AppHandle, payload: &Value) -> Result<Value, String> {
    let platform = payload
        .get("platform")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    headless_browser::shutdown_chrome();
    // 真·退出登录：删掉该平台的 Chrome profile（cookie 就存在里面）。
    // 只关进程不删 profile 的话，下次启动仍是登录状态 —— 按钮名不副实。
    let mut cleared = false;
    if let Some(platform) = platform {
        headless_browser::invalidate_login_state(platform);
        if let Ok(dir) = headless_browser::profile_dir_for(app, platform) {
            // 给进程一点时间释放文件句柄（Windows 上尤其需要）
            std::thread::sleep(std::time::Duration::from_millis(700));
            let mut removed = false;
            for _ in 0..3 {
                if std::fs::remove_dir_all(&dir).is_ok() {
                    removed = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            cleared = removed;
        }
    }
    Ok(json!({
        "success": true,
        "cleared": cleared,
        "note": if cleared {
            "已退出登录：该平台的 Chrome 登录态（profile）已清除，下次抓取需重新登录"
        } else {
            "已关闭 Chrome 进程；未指定平台或 profile 删除失败，登录态可能仍保留"
        }
    }))
}
