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

# 文件最后修改时间（unix 秒）；不存在返回 0
# GNU stat 用 -c %Y，BSD(macOS) 用 -f %m —— 两者语法不同且**都会成功返回**，
# 所以不能靠退出码判断，必须校验输出是不是纯数字。
file_mtime() {
  if [ ! -e "$1" ]; then echo 0; return; fi
  local ts
  ts="$(stat -c %Y "$1" 2>/dev/null || true)"
  case "$ts" in
    ''|*[!0-9]*) ts="$(stat -f %m "$1" 2>/dev/null || true)" ;;
  esac
  case "$ts" in
    ''|*[!0-9]*) echo 0 ;;
    *) echo "$ts" ;;
  esac
}

# 启用编译缓存（sccache）：把编译产物缓存到磁盘。
#
# 解决的是「首次/清缓存构建」——依赖有 400+ 个 crate，全量构建约 50-70 分钟；
# 装了 sccache 后，同一份依赖在不同机器/不同 target 目录之间也能复用。
# 注意：cargo 自身的 incremental 缓存解决的是「重复构建」，两者互补。
#
# 环境变量：
#   BEAV_USE_SCCACHE=0      关闭（默认开启，找不到 sccache 就自动跳过）
#   BEAV_INSTALL_SCCACHE=1  没装时自动 brew install sccache（默认不装）
setup_sccache() {
  if [ "${BEAV_USE_SCCACHE:-1}" = "0" ]; then
    log "sccache：已按 BEAV_USE_SCCACHE=0 跳过"
    return 0
  fi
  local bin=""
  # 优先用 PATH 里的；再找 ~/.local/bin（install-sccache.sh 装的默认位置）
  if command -v sccache >/dev/null 2>&1; then
    bin="$(command -v sccache)"
  elif [ -x "$HOME/.local/bin/sccache" ]; then
    bin="$HOME/.local/bin/sccache"
  elif [ "${BEAV_INSTALL_SCCACHE:-0}" = "1" ]; then
    # 服务器可能没装 Homebrew → 优先用官方二进制安装器，其次 brew
    if bash "$SCRIPT_DIR/install-sccache.sh" >/dev/null 2>&1 && [ -x "$HOME/.local/bin/sccache" ]; then
      bin="$HOME/.local/bin/sccache"
    elif command -v brew >/dev/null 2>&1; then
      log "安装 sccache（brew install sccache）…"
      if brew install sccache >/dev/null 2>&1 && command -v sccache >/dev/null 2>&1; then
        bin="$(command -v sccache)"
      else
        warn "sccache 安装失败，继续用默认编译器"
      fi
    else
      warn "sccache 安装失败（无 brew），继续用默认编译器"
    fi
  fi
  if [ -z "$bin" ]; then
    warn "未找到 sccache，本次构建不走编译缓存（可选：bash deploy/install-sccache.sh）"
    return 0
  fi
  export RUSTC_WRAPPER="$bin"
  export SCCACHE_DIR="${SCCACHE_DIR:-$HOME/.cache/sccache}"
  mkdir -p "$SCCACHE_DIR" 2>/dev/null || true
  # 路径单独一行、用 ASCII 标签输出：全角括号里夹变量在部分终端/编码下会显示成 ??
  log "sccache 已启用"
  printf '        bin   = %s\n' "$bin"
  printf '        cache = %s\n' "$SCCACHE_DIR"
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
  if command -v sccache >/dev/null 2>&1; then
    printf '  sccache   : %s\n' "$(sccache --show-stats 2>/dev/null | grep -E 'Hits|Misses|Cache size' | tr '\n' ' ' | sed 's/  */ /g')"
  fi
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
  warn "无法解析 git HEAD（构建判断改用产物时间戳，不受影响）"
elif [ "$BEFORE_HEAD" = "$AFTER_HEAD" ]; then
  log "代码无新提交（HEAD=${AFTER_HEAD:0:8}）"
else
  log "已更新到 ${AFTER_HEAD:0:8}"
  git log --oneline "${BEFORE_HEAD}..${AFTER_HEAD}" 2>/dev/null | sed 's/^/    /'
fi

# ---------- 2. 依赖 ----------
# 同样不用 pull 区间（用户可能已手动 pull）：直接看 package.json / lock 是否比 node_modules 新
pkg_ts="$(file_mtime "$DESKTOP_DIR/package.json")"
lock_ts="$(file_mtime "$DESKTOP_DIR/pnpm-lock.yaml")"
mods_ts="$(file_mtime "$DESKTOP_DIR/node_modules")"
if [ "$mods_ts" -eq 0 ] || [ "$pkg_ts" -gt "$mods_ts" ] || [ "$lock_ts" -gt "$mods_ts" ]; then
  log "依赖有变化，安装中…"
  (cd "$DESKTOP_DIR" && pnpm install --frozen-lockfile) || die "pnpm install 失败"
else
  log "依赖已是最新，跳过 pnpm install"
fi

# ---------- 3. 构建（按"产物时间 vs 最后一次相关提交"判断，而不是 pull 区间） ----------
setup_sccache
#
# 为什么不用 pull 区间：如果用户已经手动 git pull 过，区间就是空的 → 会误判"无变化"
# → 跳过构建 → 新功能根本没编译进去。这里改成对比**产物**与**最后一次相关提交**。
RUST_PATHS="desktop/src-tauri"
WEB_PATHS="desktop/src desktop/vite.config.ts desktop/index.html"
RUST_BIN="$DESKTOP_DIR/src-tauri/target/release/beav"
WEB_OUT="$DESKTOP_DIR/dist/index.html"
STAMP_DIR="$HOME/.beav-deploy"
mkdir -p "$STAMP_DIR"

# 某路径最后一次被改动的提交时间（unix 秒）
last_commit_ts() {
  git log -1 --format=%ct -- "$@" 2>/dev/null | head -1
}

# 产物是否落后于源码：stamp 记录上次构建时的 commit，不一致就重建
# 注意：源码路径要**逐个**传给 git（"$@"），不能打包成一个字符串再引号包住 ——
# 后者会被当成单个 pathspec，git 返回空 → 永远判定"已是最新" → 前端永不重建。
needs_build() {
  local stamp_file="$1" artifact="$2"
  shift 2
  local head_commit last_ts artifact_ts
  head_commit="$(git rev-list -1 HEAD -- "$@" 2>/dev/null | head -1)"
  [ -z "$head_commit" ] && return 1
  if [ -f "$stamp_file" ] && [ "$(cat "$stamp_file" 2>/dev/null)" = "$head_commit" ]; then
    return 1
  fi
  # 没有 stamp（首次使用脚本）→ 退化为比较文件时间
  last_ts="$(last_commit_ts "$@")"
  artifact_ts="$(file_mtime "$artifact")"
  if [ -z "$last_ts" ] || [ "$last_ts" -eq 0 ]; then return 1; fi
  [ "$artifact_ts" -lt "$last_ts" ]
}

need_rust=0
need_web=0
needs_build "$STAMP_DIR/rust.commit" "$RUST_BIN" $RUST_PATHS && need_rust=1
needs_build "$STAMP_DIR/web.commit" "$WEB_OUT" $WEB_PATHS && need_web=1

# 前端变了就必须重编 Rust：桌面端二进制在**编译期**把 dist/ 嵌进去
# （tauri.conf.json 的 frontendDist）。只重建 dist 不重编二进制的话，
# 桌面端仍在用旧的嵌入资源 —— 用户会遇到"改了前端但界面没变"。
if [ "$need_web" = "1" ]; then
  need_rust=1
fi

# 分支切换检测：checkout 会把所有文件时间刷成"现在"，时间戳判断会误判成"最新"
# → 明明换了分支却不重建，产物和代码对不上（例如 main ↔ web）。
# 这里记录上次构建时的 HEAD，HEAD 变了就强制全量重建。
HEAD_FILE="$STAMP_DIR/head.commit"
CURRENT_HEAD="$(git -C "$REPO_DIR" rev-parse HEAD 2>/dev/null || true)"
LAST_HEAD="$(cat "$HEAD_FILE" 2>/dev/null || true)"
if [ -n "$CURRENT_HEAD" ] && [ -n "$LAST_HEAD" ] && [ "$CURRENT_HEAD" != "$LAST_HEAD" ]; then
  warn "HEAD 变了（${LAST_HEAD:0:8} → ${CURRENT_HEAD:0:8}，可能切了分支）→ 强制重建 Rust 与前端"
  need_rust=1
  need_web=1
fi
# 注意：HEAD 基线在**构建成功后**才写（见下方），否则构建失败也会被记成"已建"，
# 下次就不会重建了。

# 自检：二进制里必须含有前端产物，否则 Tauri 会以 dev 模式启动（去连 devUrl）→ 白屏。
# 详见 tauri/build.rs：`dev = !has_feature("custom-protocol")`，该 feature 只有
# tauri-cli 构建时才会加；裸 `cargo build --release` 编出来的二进制**不嵌前端**。
assets_embedded() {
  local bin="$1"
  [ -f "$bin" ] || return 1
  local marker
  marker="$(ls -t "$DESKTOP_DIR"/dist/assets/*.js 2>/dev/null | head -1)"
  [ -n "$marker" ] || return 1
  local base
  base="$(basename "$marker")"
  # 二进制里应能搜到该资源文件名（Tauri 把资源路径当 key 嵌进去）
  if grep -a -q "$base" "$bin" 2>/dev/null; then return 0; fi
  return 1
}

if ! assets_embedded "$RUST_BIN"; then
  warn "当前二进制**未嵌入前端资源**（会白屏）→ 强制重建"
  need_rust=1
fi

if [ "$SKIP_WEB" = "1" ]; then
  log "跳过前端构建（--skip-web）"
elif [ "$need_web" = "1" ]; then
  # 用完整的 pnpm build（sync-brand + sync-version + tsc + vite build），
  # 不是 pnpm web:build —— 后者会跳过品牌/版本同步。
  log "前端产物落后于源码 → pnpm build…"
  (cd "$DESKTOP_DIR" && node ./scripts/run-with-brand.mjs beav pnpm build) || die "前端构建失败"
  git rev-list -1 HEAD -- $WEB_PATHS 2>/dev/null | head -1 > "$STAMP_DIR/web.commit"
  log "前端构建完成：$WEB_OUT"
else
  log "前端产物已是最新，跳过构建"
fi

if [ "$SKIP_RUST" = "1" ]; then
  log "跳过 Rust 构建（--skip-rust）"
elif [ "$need_rust" = "1" ]; then
  log "Rust 产物落后于源码 → cargo build --release（首次全量较慢，请耐心）…"
  # ⚠️ 必须带 `--features tauri/custom-protocol`：
  #   tauri/build.rs 里 `dev = !has_feature("custom-protocol")`。不开这个 feature 时
  #   dev=true → Tauri 不嵌入前端资源、改为去连 devUrl(http://localhost:1420)
  #   → 服务器上没人监听 → **桌面端白屏**。
  #   （只有 tauri-cli 构建时才会自动加这个 feature，裸 cargo build 不会。）
  # 必须在前端构建**之后**执行：dist 会被编译进二进制。
  (cd "$DESKTOP_DIR/src-tauri" \
    && cargo build --release --bin beav --features tauri/custom-protocol) \
    || die "cargo build 失败"
  git rev-list -1 HEAD -- "$RUST_PATHS" 2>/dev/null | head -1 > "$STAMP_DIR/rust.commit"
  log "Rust 构建完成：$RUST_BIN"
  if assets_embedded "$RUST_BIN"; then
    log "已确认前端资源已嵌入二进制 ✅"
  else
    err "⚠️ 未检测到前端资源嵌入 —— 桌面端可能白屏。"
    err "   排查：1) dist/ 是否生成  2) 构建是否带了 --features tauri/custom-protocol"
  fi
else
  log "Rust 产物已是最新，跳过构建"
fi

# 构建都成功了 → 记录本次构建的 HEAD（下次切分支能识别出来强制重建）
if [ -n "$CURRENT_HEAD" ]; then
  echo "$CURRENT_HEAD" > "$HEAD_FILE"
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
