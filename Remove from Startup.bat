@echo off
title Remove Text Extractor from Windows Startup
del "%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\Text Extractor.lnk" 2>nul
del "%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\Text Extractor Standalone.lnk" 2>nul
echo Removed from Windows Startup.
pause
