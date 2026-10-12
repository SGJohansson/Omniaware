@echo off
rem Starts oa-probe with no console window. Right-click its amber tray icon to stop it.
start "" "%~dp0oa-probe.exe" --hidden %*
