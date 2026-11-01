//! 请求级身份。
//!
//! - 桌面端直接调用 IPC：没有 `X-Beav-User` 头 → 视为**管理员**，可见全部数据；
//! - Web 网关反代时注入 `X-Beav-User: <username>` → 该请求只可见自己的数据。
//!
//! 用线程局部保存：daemon 每个连接一个线程，`handle_channel` 同步执行，
//! 请求结束即清除，不会串到下一个请求。

use std::cell::RefCell;

thread_local! {
    static CURRENT_OWNER: RefCell<Option<String>> = RefCell::new(None);
}

pub(crate) fn set_request_owner(owner: Option<String>) {
    let normalized = owner
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    CURRENT_OWNER.with(|cell| *cell.borrow_mut() = normalized);
}

pub(crate) fn clear_request_owner() {
    CURRENT_OWNER.with(|cell| *cell.borrow_mut() = None);
}

/// `None` = 管理员（桌面端）。
pub(crate) fn current_owner() -> Option<String> {
    CURRENT_OWNER.with(|cell| cell.borrow().clone())
}

pub(crate) fn is_admin() -> bool {
    current_owner().is_none()
}

pub(crate) fn owner_or_admin() -> String {
    current_owner().unwrap_or_else(|| "admin".to_string())
}

fn effective(owner: &str) -> &str {
    if owner.trim().is_empty() {
        "admin"
    } else {
        owner
    }
}

/// 当前身份能否访问该 owner 的数据（旧数据空 owner 归 admin）。
pub(crate) fn owner_matches(owner: &str) -> bool {
    match current_owner() {
        None => true,
        Some(current) => current == effective(owner),
    }
}

/// 两个归属是否指向同一个用户（空 owner 归 admin）。
///
/// 与 `owner_matches` 的区别：那个问「**当前请求身份**能不能访问这条数据」，
/// 这个问「这两条数据是不是同一个用户的」。做去重/归属判定时必须用后者 ——
/// 定时任务线程没有请求身份（= admin），拿它去比会把各用户的帖子混成一堆。
pub(crate) fn owner_equals(left: &str, right: &str) -> bool {
    effective(left) == effective(right)
}

/// 当前身份能否访问「归属 `owner`、且被分配给 `shared_to` 这些人」的数据。
///
/// 在 `owner_matches` 之上多放行一条：**被分配**的博主。
/// 用途限于「看」和「对已有数据做增量处理」（抓取、补历史、下载）；
/// 删除类操作必须继续用 `owner_matches`，否则被分配者能反过来删掉别人的东西。
pub(crate) fn can_access(owner: &str, shared_to: &[String]) -> bool {
    if owner_matches(owner) {
        return true;
    }
    let Some(current) = current_owner() else {
        return true;
    };
    let me = current.trim();
    shared_to.iter().any(|user| user.trim() == me)
}
