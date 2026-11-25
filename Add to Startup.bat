@echo off
title Add Text Extractor to Windows Startup
powershell -NoProfile -Command "$ws = New-Object -ComObject WScript.Shell; $s = $ws.CreateShortcut([Environment]::GetFolderPath('Startup') + '\Text Extractor.lnk'); $s.TargetPath = '%~dp0TextExtractor.exe'; $s.WorkingDirectory = '%~dp0'; $s.Description = 'Text Extractor (Rust Standalone)'; $s.Save(); Write-Host 'Added to Windows Startup successfully!'"
pause
