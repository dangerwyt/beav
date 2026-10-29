#!/bin/bash
# Beav 服务器一键更新脚本（macOS）
#
# 做四件事：
#   1. git pull（仅快进）
#   2. 按需重建：Rust 源码变了 → cargo build --release；前端变了 → vite build
#   3. 重启 launchd 服务（com.beav.desktop / com.beav.web）
#   4. 健康检查（daemon 31937 / web 8090），失败时打印日志尾部
#
# 用法：
#   bash deploy/update-server.sh              # 完整更新
#   bash deploy/update-server.sh --skip-rust  # 跳过 Rust 重建
#   bash deploy/update-server.sh --skip-web   # 跳过前端构建
#   bash deploy/update-server.sh --no-restart # 只构建，不重启服务
#   bash deploy/update-server.sh --status     # 只看当前状态

# 不用 `set -u`：部署脚本里变量可能因分支/环境差异为空，nounset 会直接中断更新。
# 宁可少一道保险，也别让脚本在服务器上崩在半路。
set -o pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DESKTOP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_DIR="$(cd "$DESKTOP_DIR/.." && pwd)"
LOG_DIR="$HOME/Library/Logs/beav"
LABEL_DESKTOP="com.beav.desktop"
LABEL_WEB="com.beav.web"
UID_NUM="$(id -u)"

SKIP_RUST=0
SKIP_WEB=0
NO_RESTART=0
STATUS_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --skip-rust) SKIP_RUST=1 ;;
    --skip-web) SKIP_WEB=1 ;;
    --no-restart) NO_RESTART=1 ;;
    --status) STATUS_ONLY=1 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "未知参数：$arg" >&2; exit 1 ;;
  esac
done

log()  { printf '\033[36m[update]\033[0m %s\n' "$*"; }
warn() { printf '\033[33m[update]\033[0m %s\n' "$*"; }
err()  { printf '\033[31m[update]\033[0m %s\n' "$*" >&2; }
die()  { err "$*"; exit 1; }

service_loaded() {
  launchctl print "gui/$UID_NUM/$1" >/dev/null 2>&1
}

# 取 HTTP 状态码；失败统一返回 000（curl 自己也会输出 000，不要重复补）
http_code() {
  local out
  out="$(curl -s -o /dev/null -w '%{http_code}' --max-time "$2" "$1" 2>/dev/null || true)"
  [ -n "$out" ] && echo "$out" || echo "000"
}

show_status() {
  log "仓库：$REPO_DIR"
  printf '  git HEAD : %s\n' "$(git -C "$REPO_DIR" rev-parse --short HEAD 2>/dev/null || echo '?')"
  printf '  分支     : %s\n' "$(git -C "$REPO_DIR" rev-parse --abbrev-ref HEAD 2>/dev/null || echo '?')"
  for label in "$LABEL_DESKTOP" "$LABEL_WEB"; do
    if service_loaded "$label"; then
      local_state=$(launchctl print "gui/$UID_NUM/$label" 2>/dev/null | grep -m1 'state = ' | sed 's/.*= //')
      local_pid=$(launchctl print "gui/$UID_NUM/$label" 2>/dev/null | grep -m1 'pid = ' | sed 's/.*= //')
      printf '  %-22s %s (pid %s)\n' "$label" "${local_state:-unknown}" "${local_pid:-?}"
    else
      printf '  %-22s 未加载\n' "$label"
    fi
  done
  printf '  daemon    : %s\n' "$(http_code http://127.0.0.1:31937/api/ipc/health 5)"
  printf '  web       : %s\n' "$(http_code http://127.0.0.1:8090/login 5)"
}

if [ "$STATUS_ONLY" = "1" ]; then
  show_status
  exit 0
fi

cd "$REPO_DIR" || die "找不到仓库目录：$REPO_DIR"

# ---------- 1. 拉取 ----------
log "拉取最新代码…"
BEFORE_HEAD="$(git rev-parse HEAD 2>/dev/null || true)"
BEFORE_HEAD="${BEFORE_HEAD:-}"
if ! git pull --ff-only; then
  err "git pull 失败（可能有本地改动或冲突）"
  git status --short | head -20
  die "已中止，未做任何构建"
fi
AFTER_HEAD="$(git rev-parse HEAD 2>/dev/null || true)"
AFTER_HEAD="${AFTER_HEAD:-}"
if [ -z "$AFTER_HEAD" ] || [ -z "$BEFORE_HEAD" ]; then
  warn "无法解析 git HEAD，按全量重建处理"
  CHANGED="desktop/src-tauri/
desktop/src/"
elif [ "$BEFORE_HEAD" = "$AFTER_HEAD" ]; then
  log "代码无变化（HEAD=$AFTER_HEAD）"
  CHANGED=""
else
  log "已更新到 ${AFTER_HEAD:0:8}"
  git log --oneline "${BEFORE_HEAD}..${AFTER_HEAD}" 2>/dev/null | sed 's/^/    /'
  CHANGED="$(git diff --name-only "${BEFORE_HEAD}" "${AFTER_HEAD}" 2>/dev/null || true)"
fi

# ---------- 2. 依赖 ----------
if [ -n "$(git diff --name-only "${BEFORE_HEAD:-HEAD}" "${AFTER_HEAD:-HEAD}" -- desktop/package.json desktop/pnpm-lock.yaml 2>/dev/null || true)" ]; then
  log "依赖有变化，安装中…"
  (cd "$DESKTOP_DIR" && pnpm install --frozen-lockfile) || die "pnpm install 失败"
else
  log "依赖未变化，跳过 pnpm install"
fi

# ---------- 3. 构建 ----------
if [ "$SKIP_RUST" = "1" ]; then
  log "跳过 Rust 构建"
elif echo "$CHANGED" | grep -q '^desktop/src-tauri/'; then
  log "Rust 源码有变化 → cargo build --release（首次全量较慢，请耐心）…"
  (cd "$DESKTOP_DIR/src-tauri" && cargo build --release) || die "cargo build 失败"
  log "Rust 构建完成：$DESKTOP_DIR/src-tauri/target/release/beav"
else
  log "Rust 源码未变化，跳过构建"
fi

if [ "$SKIP_WEB" = "1" ]; then
  log "跳过前端构建"
elif echo "$CHANGED" | grep -qE '^desktop/src/|^desktop/src-tauri/tauri.conf.json|^desktop/vite'; then
  log "前端有变化 → vite build…"
  (cd "$DESKTOP_DIR" && pnpm web:build) || die "vite build 失败"
  log "前端构建完成：$DESKTOP_DIR/dist"
else
  log "前端未变化，跳过构建（dist 保持原样）"
fi

# ---------- 4. 重启 ----------
if [ "$NO_RESTART" = "1" ]; then
  log "按要求不重启服务"
  show_status
  exit 0
fi

for label in "$LABEL_DESKTOP" "$LABEL_WEB"; do
  if service_loaded "$label"; then
    log "重启 $label …"
    launchctl kickstart -k "gui/$UID_NUM/$label" || warn "$label 重启失败"
  else
    warn "$label 未加载（首次部署请执行：bash deploy/install-macos-services.sh）"
  fi
done

# ---------- 5. 健康检查 ----------
sleep 3
daemon_code="$(http_code http://127.0.0.1:31937/api/ipc/health 8)"
web_code="$(http_code http://127.0.0.1:8090/login 8)"

if [ "$daemon_code" = "200" ] && [ "$web_code" = "200" ]; then
  log "更新完成 ✅ daemon=$daemon_code web=$web_code"
else
  err "健康检查未通过：daemon=$daemon_code web=$web_code"
  for f in "$LOG_DIR/desktop.err.log" "$LOG_DIR/web.err.log"; do
    if [ -f "$f" ]; then
      echo "----- $f (tail 40) -----"
      tail -40 "$f"
    fi
  done
  exit 1
fi

show_status
