@echo off
rem Gera um SESSION_SECRET (32 bytes, base64) para o credentials.env.
powershell -NoProfile -Command "[Convert]::ToBase64String([System.Security.Cryptography.RandomNumberGenerator]::GetBytes(32))"
