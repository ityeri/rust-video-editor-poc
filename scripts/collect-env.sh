#!/usr/bin/env bash
# 3 OS 정보 수집(Linux/macOS). 출력 전체를 복사해서 전달하면 된다.
set -u
echo "=== collect-env ==="
echo "date: $(date -Is 2>/dev/null || date)"
echo "uname: $(uname -a)"
for c in rustc cargo; do printf "%s: " "$c"; ($c -V 2>/dev/null || echo "MISSING"); done

echo "--- session/env ---"
for k in XDG_SESSION_TYPE WAYLAND_DISPLAY DISPLAY XMODIFIERS QT_IM_MODULE GTK_IM_MODULE SDL_IM_MODULE \
         GLFW_IM_MODULE LANG LC_ALL LC_CTYPE XKB_DEFAULT_LAYOUT; do
  printf "%s=%s\n" "$k" "${!k:-<unset>}"
done

echo "--- IME 프로세스 ---"
ps -eo comm 2>/dev/null | grep -Ei 'fcitx|ibus|nimf|uim' | sort -u || echo "없음"

if command -v fcitx5 >/dev/null 2>&1; then echo "fcitx5: $(fcitx5 --version 2>&1 | head -1)"; fi
if command -v ibus >/dev/null 2>&1; then echo "ibus: $(ibus version 2>&1 | head -1)"; fi

echo "--- GPU ---"
if command -v nvidia-smi >/dev/null 2>&1; then nvidia-smi --query-gpu=name,driver_version --format=csv,noheader; fi
if command -v glxinfo >/dev/null 2>&1; then glxinfo -B 2>/dev/null | grep -Ei 'device|version' | head -6; fi
if [ "$(uname)" = "Darwin" ]; then system_profiler SPDisplaysDataType 2>/dev/null | head -20; fi
echo "=== end ==="
