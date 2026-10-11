@echo off
cd /d "%~dp0"
echo ========================================================
echo   Pushing code to GitHub...
echo   Target: https://github.com/Anson-sen/ProxyDistribution.git
echo ========================================================
echo.

git push -u origin main

echo.
echo ========================================================
echo   GitHub Actions URL:
echo   https://github.com/Anson-sen/ProxyDistribution/actions
echo ========================================================
echo.
pause
