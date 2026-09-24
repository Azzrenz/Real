@echo off
setlocal
title Real - Build and Run

rem ============================================================
rem  Real: one-click build and run
rem
rem  Prerequisites: Node.js 18+ and Rust 1.75+.
rem  Everything else is automatic -- frontend dependencies are
rem  installed on first run, server\.env is seeded from the
rem  template, then the backend and the desktop shell start.
rem
rem  FIRST RUN TAKES SEVERAL MINUTES. cargo compiles the whole
rem  backend, then Tauri compiles the desktop shell. That is the
rem  compiler working, not a hang -- let it finish.
rem ============================================================

echo ============================================
echo   Real - Build and Run
echo ============================================

echo [1/5] Checking toolchain...
where node >nul 2>nul
if errorlevel 1 goto no_node
where cargo >nul 2>nul
if errorlevel 1 goto no_cargo
node --version
cargo --version

echo [2/5] Preparing configuration...
if exist "%~dp0server\.env" goto env_ready
copy /y "%~dp0server\.env.example" "%~dp0server\.env" >nul 2>&1
if not exist "%~dp0server\.env" goto fail
echo        created server\.env from the template
echo        (no API key yet -- Real runs with its built-in mock model)
goto env_done
:env_ready
echo        server\.env already present
:env_done

echo [3/5] Installing frontend dependencies...
if exist "%~dp0client\node_modules" goto deps_ready
cd /d "%~dp0client"
call npm install
if errorlevel 1 goto fail
goto deps_done
:deps_ready
echo        already installed
:deps_done

echo [4/5] Compiling backend (first run: a few minutes)...
cd /d "%~dp0server"
rem Kill any stale backend first: a running real-server.exe locks the exe on
rem Windows so cargo build cannot overwrite it (os error 5 denied access).
rem Ignore errors if it is not running.
taskkill /F /IM real-server.exe >nul 2>&1
timeout /t 1 /nobreak >nul
call cargo build
if errorlevel 1 goto fail

echo [5/5] Starting backend and desktop shell...
if not exist "%APPDATA%\real-agent\logs" mkdir "%APPDATA%\real-agent\logs"
set "LOG=%APPDATA%\real-agent\logs\backend.log"
rem Log rotation. backend.log can be left locked by a stray process that
rem inherited the handle from an earlier run: cmd then cannot open it, the
rem whole command dies, and the server never starts. Test file-system state
rem instead of errorlevel, because cmd does not always set errorlevel when a
rem redirect target cannot be opened.
if not exist "%LOG%" goto log_ready
move /y "%LOG%" "%APPDATA%\real-agent\logs\backend-prev.log" >nul 2>&1
if not exist "%LOG%" goto log_ready
for /f %%i in ('powershell -NoProfile -Command "Get-Date -Format yyyyMMdd-HHmmss"') do set "STAMP=%%i"
set "LOG=%APPDATA%\real-agent\logs\backend-%STAMP%.log"
echo        [WARN] backend.log is locked by a stray process, writing to:
echo               %LOG%
:log_ready
rem /B keeps this window as the only console: the backend shares it and writes
rem to the log file. A failed start is still visible, because the probe below
rem prints the log tail when the port never opens.
start "" /B cmd /c "target\debug\real-server.exe >> %LOG% 2>&1"
set /a tries=0
:wait_backend
ping -n 2 127.0.0.1 >nul 2>&1
rem Probe the port, not the HTTP body: no dependency on curl.exe being on PATH.
powershell -NoProfile -Command "try{$c=New-Object Net.Sockets.TcpClient;$c.Connect('127.0.0.1',8943);$c.Close();exit 0}catch{exit 1}"
if not errorlevel 1 goto backend_ok
set /a tries+=1
if %tries% lss 15 goto wait_backend
echo.
echo [ERROR] Backend did not come up after 30s. Last 25 log lines:
echo ------------------------------------------------------------
powershell -NoProfile -Command "Get-Content -Tail 25 '%LOG%'"
echo ------------------------------------------------------------
echo Fix the error above, then run this script again.
pause
goto eof
:backend_ok
echo        backend is up (log: %LOG%)

echo        starting the Real window -- Tauri compiles it on first run,
echo        which can take a few minutes too.
cd /d "%~dp0client"
call npm run tauri dev
goto eof

:no_node
echo.
echo [ERROR] Node.js not found.
echo         Install the LTS version from https://nodejs.org/
echo         then run this script again.
echo.
pause
exit /b 1

:no_cargo
echo.
echo [ERROR] Rust not found.
echo         Install it from https://rustup.rs/  (the default install is fine)
echo         then run this script again.
echo.
pause
exit /b 1

:fail
echo.
echo Build failed. Check the output above.
pause
exit /b 1

:eof
endlocal
