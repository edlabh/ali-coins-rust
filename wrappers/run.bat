@echo off
rem Execução unificada (`ali-coins all`) — wrapper Windows (CMD).
setlocal
set "ROOT=%~dp0.."
cd /d "%ROOT%"
set "EXE=%ROOT%\target\release\ali-coins.exe"
where ali-coins >nul 2>nul && set "EXE=ali-coins"
"%EXE%" all %*
exit /b %ERRORLEVEL%
