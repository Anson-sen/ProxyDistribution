@echo off
chcp 65001 >nul
title 推送代码到 GitHub 并触发云端打包
cd /d "%~dp0"

echo ========================================================
echo   正在推送代码到 GitHub (https://github.com/Anson-sen/ProxyDistribution.git)
echo ========================================================
echo.

git push -u origin main

if %errorlevel% equ 0 (
    echo.
    echo ========================================================
    echo  [成功] 代码已成功推送到 GitHub！
    echo  GitHub Actions 正在云端 Mac 服务器为您自动编译打包！
    echo.
    echo  请前往查看构建进度与下载 macOS DMG 安装包：
    echo  https://github.com/Anson-sen/ProxyDistribution/actions
    echo ========================================================
) else (
    echo.
    echo ========================================================
    echo  [提示] 推送遇到错误，可能需要先完成 GitHub 账号授权登录。
    echo ========================================================
)

echo.
pause
