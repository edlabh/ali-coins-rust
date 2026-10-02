@echo off
rem Tarefas diárias (`ali-coins tasks`) — wrapper Windows (CMD).
setlocal
set "ROOT=%~dp0.."
cd /d "%ROOT%"
set "EXE=%ROOT%\target\release\ali-coins.exe"
where ali-coins >nul 2>nul && set "EXE=ali-coins"
"%EXE%" tasks %*
exit /b %ERRORLEVEL%
