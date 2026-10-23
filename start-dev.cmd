@echo off
chcp 65001 >nul
cd /d "%~dp0desktop"
echo ============================================
echo   Beav 开发启动器  (pnpm tauri:dev:beav)
echo   关闭此窗口或按 Ctrl+C 可停止应用
echo ============================================
call pnpm tauri:dev:beav
echo.
echo [Beav 已退出] 按任意键关闭窗口...
pause >nul
