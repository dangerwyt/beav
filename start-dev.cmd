@echo off
setlocal
title Beav 开发启动器
cd /d "%~dp0desktop"
if not exist "package.json" (
    echo [错误] 未找到 desktop\package.json，请确认本脚本位于项目根目录。
    echo.
    pause
    exit /b 1
)
echo ============================================
echo   Beav 开发启动器  (pnpm tauri:dev:beav)
echo   关闭此窗口或按 Ctrl+C 可停止应用
echo ============================================
call pnpm tauri:dev:beav
echo.
echo [Beav 已退出] 按任意键关闭窗口...
pause >nul
endlocal
