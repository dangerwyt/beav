#!/bin/bash
# Beav macOS 常驻服务安装脚本（launchd）
#
# 安装后：
#   - com.beav.desktop：桌面端（BEAV_SKIP_OFFICIAL_LOGIN=1），登录自启、崩溃重启
#   - com.beav.web    ：web 网关（BEAV_SINGLE_DAEMON=1，8090），登录自启、常驻
#
# 用法：
#   cd desktop && bash deploy/install-macos-services.sh            # 安装/更新
#   cd desktop && bash deploy/install-macos-services.sh --uninstall
#
# 日志：
#   ~/Library/Logs/beav/desktop.log / desktop.err.log
#   ~/Library/Logs/beav/web.log     / web.err.log

set -euo pipefail

LABEL_DESKTOP="com.beav.desktop"
LABEL_WEB="com.beav.web"
LA_DIR="$HOME/Library/LaunchAgents"
LOG_DIR="$HOME/Library/Logs/beav"
UID_NUM="$(id -u)"

DESKTOP_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$DESKTOP_DIR/src-tauri/target/release/beav"
NODE_BIN="$(command -v node || true)"
PATH_VALUE="$(dirname "${NODE_BIN:-/usr/bin/node}"):/usr/local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin"

mkdir -p "$LA_DIR" "$LOG_DIR"

if [ "${1:-}" = "--uninstall" ]; then
  for label in "$LABEL_DESKTOP" "$LABEL_WEB"; do
    launchctl bootout "gui/$UID_NUM/$label" 2>/dev/null || true
    rm -f "$LA_DIR/$label.plist"
    echo "已卸载 $label"
  done
  exit 0
fi

if [ -z "$NODE_BIN" ]; then
  echo "找不到 node（请确认 nvm/homebrew 的 node 在当前 shell PATH 中）" >&2
  exit 1
fi
if [ ! -x "$BIN" ]; then
  echo "找不到桌面端可执行文件：$BIN" >&2
  echo "请先构建：cd \"$DESKTOP_DIR\" && pnpm tauri:build:beav" >&2
  exit 1
fi
if [ ! -f "$DESKTOP_DIR/dist/index.html" ]; then
  echo "警告：$DESKTOP_DIR/dist/index.html 不存在，web 网关会 404；请先 pnpm web:build" >&2
fi

echo "desktop : $BIN"
echo "node    : $NODE_BIN"
echo "root    : $DESKTOP_DIR"

cat > "$LA_DIR/$LABEL_DESKTOP.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL_DESKTOP</string>
  <key>ProgramArguments</key>
  <array><string>$BIN</string></array>
  <key>WorkingDirectory</key><string>$DESKTOP_DIR</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>BEAV_SKIP_OFFICIAL_LOGIN</key><string>1</string>
    <key>PATH</key><string>$PATH_VALUE</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key>
  <dict><key>SuccessfulExit</key><false/></dict>
  <key>LimitLoadToSessionType</key><string>Aqua</string>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>StandardOutPath</key><string>$LOG_DIR/desktop.log</string>
  <key>StandardErrorPath</key><string>$LOG_DIR/desktop.err.log</string>
</dict>
</plist>
EOF

cat > "$LA_DIR/$LABEL_WEB.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$LABEL_WEB</string>
  <key>ProgramArguments</key>
  <array>
    <string>$NODE_BIN</string>
    <string>$DESKTOP_DIR/scripts/web-instances.mjs</string>
  </array>
  <key>WorkingDirectory</key><string>$DESKTOP_DIR</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>BEAV_SINGLE_DAEMON</key><string>1</string>
    <key>WEB_HOST</key><string>0.0.0.0</string>
    <key>WEB_PORT</key><string>8090</string>
    <key>PATH</key><string>$PATH_VALUE</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>5</integer>
  <key>StandardOutPath</key><string>$LOG_DIR/web.log</string>
  <key>StandardErrorPath</key><string>$LOG_DIR/web.err.log</string>
</dict>
</plist>
EOF

for label in "$LABEL_DESKTOP" "$LABEL_WEB"; do
  launchctl bootout "gui/$UID_NUM/$label" 2>/dev/null || true
  launchctl bootstrap "gui/$UID_NUM" "$LA_DIR/$label.plist"
  echo "已加载 $label"
done

echo
echo "状态："
launchctl print "gui/$UID_NUM/$LABEL_DESKTOP" 2>/dev/null | grep -E "state|pid" | head -3 || true
launchctl print "gui/$UID_NUM/$LABEL_WEB" 2>/dev/null | grep -E "state|pid" | head -3 || true
echo
echo "日志：tail -f $LOG_DIR/web.log"
