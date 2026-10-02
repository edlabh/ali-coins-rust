@echo off
rem Execução unificada (check-in + tarefas) — wrapper Windows (CMD).
rem Retenta uma vez (--no-delay) quando o exit é 1, como o oráculo.
setlocal
set "ROOT=%~dp0.."
cd /d "%ROOT%"
set "EXE=%ROOT%\target\release\ali-coins.exe"
where ali-coins >nul 2>nul && set "EXE=ali-coins"

"%EXE%" all %*
set CODE=%ERRORLEVEL%
if not "%CODE%"=="1" goto fim
echo [run_all] execucao falhou ^(exit 1^); retentando uma vez com --no-delay... 1>&2
timeout /t 10 /nobreak >nul
"%EXE%" all --no-delay %*
set CODE=%ERRORLEVEL%

:fim
echo [run_all] exit=%CODE% fim: %DATE% %TIME%
exit /b %CODE%
