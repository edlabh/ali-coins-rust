@echo off
rem Check-in diário (`ali-coins checkin`) — wrapper Windows (CMD).
setlocal
set "ROOT=%~dp0.."
cd /d "%ROOT%"
set "EXE=%ROOT%\target\release\ali-coins.exe"
where ali-coins >nul 2>nul && set "EXE=ali-coins"
"%EXE%" checkin %*
exit /b %ERRORLEVEL%
