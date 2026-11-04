#!/bin/bash
# 安装 sccache（编译缓存）到 ~/.local/bin —— 不依赖 Homebrew。
#
# 用途：Beav 的 Rust 依赖有 400+ crate，全量构建约 50-70 分钟；
# sccache 把编译产物缓存到磁盘，之后清缓存/换机器重建能直接命中。
#
# 用法：
#   bash deploy/install-sccache.sh            # 装最新正式版
#   bash deploy/install-sccache.sh --version v0.10.0
#
# 装完记得把 ~/.local/bin 加进 PATH（脚本末尾会提示）。
set -uo pipefail

INSTALL_DIR="${BEAV_SCCACHE_DIR:-$HOME/.local/bin}"
CACHE_DIR="${SCCACHE_DIR:-$HOME/.cache/sccache}"
VERSION="latest"

for arg in "$@"; do
  case "$arg" in
    --version) VERSION="" ;;
    --version=*) VERSION="${arg#*=}" ;;
    -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "未知参数：$arg" >&2; exit 1 ;;
  esac
done

log() { printf '[sccache] %s\n' "$*"; }
die() { printf '[sccache] %s\n' "$*" >&2; exit 1; }

case "$(uname -s)" in
  Darwin) : ;;
  *) die "本脚本只支持 macOS（当前：$(uname -s)）" ;;
esac

case "$(uname -m)" in
  arm64)  ARCH_TOKEN="aarch64-apple-darwin" ;;
  x86_64) ARCH_TOKEN="x86_64-apple-darwin" ;;
  *) die "不支持的架构：$(uname -m)" ;;
esac

mkdir -p "$INSTALL_DIR" "$CACHE_DIR" || die "创建目录失败"

if [ "$VERSION" = "latest" ]; then
  log "查询最新版本…"
  release_json="$(curl -fsSL --max-time 30 https://api.github.com/repos/mozilla/sccache/releases/latest 2>/dev/null)" \
    || die "拉取 GitHub release 失败（服务器可能无法直连 api.github.com）"
  url="$(printf '%s' "$release_json" \
    | grep -o '"browser_download_url": *"[^"]*"' \
    | sed 's/.*"browser_download_url": *"//; s/"$//' \
    | grep "$ARCH_TOKEN" \
    | grep -E '\.tar\.gz$' \
    | head -1)"
  [ -n "$url" ] || die "在 release 里没找到 $ARCH_TOKEN 的包（可能网络受限，或改用 --version 指定）"
else
  url="https://github.com/mozilla/sccache/releases/download/v${VERSION}/sccache-v${VERSION}-${ARCH_TOKEN}.tar.gz"
fi

log "下载：$url"
tmp="$(mktemp -d)" || die "创建临时目录失败"
trap 'rm -rf "$tmp"' EXIT

if ! curl -fL --max-time 300 -o "$tmp/sccache.tar.gz" "$url"; then
  die "下载失败"
fi

tar -xzf "$tmp/sccache.tar.gz" -C "$tmp" || die "解压失败"
# 不要用 `find -perm -u+x`：那是 GNU 语法，macOS 的 BSD find 对符号模式的处理不一致
BIN="$(find "$tmp" -type f -name sccache | head -1)"
[ -n "$BIN" ] || die "压缩包里没找到 sccache 可执行文件"
chmod +x "$BIN" || die "设置可执行权限失败"

install -m 755 "$BIN" "$INSTALL_DIR/sccache" || die "安装失败"

log "已安装：$INSTALL_DIR/sccache"
"$INSTALL_DIR/sccache" --version || true

case ":$PATH:" in
  *":$INSTALL_DIR:"*) log "PATH 已包含 $INSTALL_DIR" ;;
  *) log "请把下面这行加进 ~/.zshrc："
     echo "    export PATH=\"$INSTALL_DIR:\$PATH\""
     log "（本脚本会自动用 $INSTALL_DIR/sccache，无需改 PATH 也能用）" ;;
esac
log "缓存目录：$CACHE_DIR"
