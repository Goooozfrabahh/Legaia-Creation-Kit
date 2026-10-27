@echo off
setlocal
title Legaia Studio LCK - Installed Latest Build
set "LCK_EXE=D:\AstraRimElmProject\legaiasource\legend-of-legaia-re-main\target\release\legaia-studio.exe"
if not exist "%LCK_EXE%" (
    echo The installed LCK release was not found at "%LCK_EXE%".
    echo Ask for a release build and installation before trying again.
    pause
    exit /b 1
)
cd /d "D:\AstraRimElmProject\legaiasource\legend-of-legaia-re-main"
start "" "%LCK_EXE%" %*
