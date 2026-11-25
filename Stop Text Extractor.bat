@echo off
title Stop Text Extractor
powershell -NoProfile -Command "Stop-Process -Name 'TextExtractor' -Force -ErrorAction SilentlyContinue; Write-Host 'Text Extractor stopped.'"
