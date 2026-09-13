# 검증 기록

## 이 환경(에이전트 CI)에서의 확인 결과

- 환경: Linux 6.12.57-talos, agent@컨테이너, **rustc/cargo 없음 → 설치함(1.98.1)**, GPU 없음(`/dev/dri` 부재),
  디스플레이 없음(`DISPLAY`/`WAYLAND_DISPLAY` 미설정), fontconfig/freetype/xkbcommon/wayland/x11/vulkan
  시스템 라이브러리 전부 미설치 → **GUI 실행 불가**.
- 따라서 "컴파일이 되는가"까지가 이 환경의 한계이며, 렌더링·IME·성능은 사용자 실기기 검증 항목이다.

| 크레이트 | cargo check (linux) | cargo check (windows 타깃) | 비고 |
|---|---|---|---|
| poc1-slint-wgpu-texture | (아래에 실제 결과 기록) | | Slint는 Linux에서 fontconfig 등 시스템 라이브러리를 요구할 수 있음 |
| poc2-ime-probe | | | |
| poc3-cubecl-wgpu-share | | | |

> 이 표는 이 저장소를 만든 환경의 `cargo check` 실제 출력만 기록한다. 실행(런타임) 결과는 사용자 몫이다.
