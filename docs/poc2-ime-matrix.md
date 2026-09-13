# PoC2 — 3 OS 한국어 IME

## 두 개의 바이너리

| 바이너리 | 보는 것 | 왜 필요한가 |
|---|---|---|
| `ime-winit` | winit 이 OS IME 에서 받는 **원시 이벤트** | iced/egui/slint 는 모두 이 레이어를 공유한다. 여기서 깨지면 툴킷이 아니라 winit 수준 문제 |
| `ime-slint` | **Slint 위젯**이 그 이벤트를 제대로 소비하는가 | 실제 제품에 쓸 툴킷의 실측. 조합 중간값이 `text` 에 들어오는지 판정 |

## ime-winit 사용법

```bash
cargo run --release -p poc2-ime-probe --bin ime-winit
```

- 방향키: 가상 캐럿 이동 → `set_ime_cursor_area` 갱신(후보창이 따라오는지 눈으로 확인)
- 숫자키 1~6: 아래 6개 테스트 케이스 구분선 기록
- Ctrl+Q: 종료(요약 리포트 생성)

화면에 텍스트를 그리지 않고 **창 제목 + 터미널 출력**으로 상태를 보여준다(GPU 없이도 진단 가능).
제목 표시줄에 `preedit="…"` 가 실시간으로 보이므로 조합 상태를 바로 판정할 수 있다.

## 테스트 케이스 (3 OS 공통)

| # | 케이스 | Windows | Linux(Wayland) | Linux(X11) | macOS |
|---|---|---|---|---|---|
| 1 | 순수 한글 입력(안녕하세요) | | | | |
| 2 | 한자 변환(한자키 / Option+Return) | | | | |
| 3 | 조합 중 포커스 상실 → 복귀 | | | | |
| 4 | 캐럿 이동 중 후보창 위치 추적 | | | | |
| 5 | 영문/숫자/기호 혼합, 스페이스 확정 | | | | |
| 6 | 자모 단독/복합(ㄱ ㄲ ㅘ ㅢ) + Backspace 수정 | | | | |

판정 기준: `Ime::Enabled → Preedit(반복) → Commit → (필요시 Disabled)` 순서가 자연스럽고,
Commit 이후 preedit 가 비워지며, 후보창이 캐럿 위치를 따라오면 OK.

## ime-slint 사용법

```bash
cargo run --release -p poc2-ime-probe --bin ime-slint
```

- 상/중/하 3개 입력창에서 각각 한글을 입력한다(후보창이 각 위치를 따라오는지 확인).
- 100ms 폴링으로 `text` 프로퍼티 변화를 로그에 남긴다.
  **조합 중간값(preedit)이 `text` 에 들어오는지**가 핵심 판정 포인트다(들어오면 조합 중 상태를 앱이 알 수 있다).

| 항목 | 결과 |
|---|---|
| 3개 입력창 모두 한글 입력/확정 가능 | |
| 조합 중간값이 `text` 로 들어옴(로그 확인) | |
| 후보창이 캐럿을 따라옴 | |
| 조합 중 다른 입력창 클릭 시 이전 조합이 어떻게 되는가 | |
| Backspace/방향키가 조합과 충돌하지 않음 | |
| `scale_factor` 값(HiDPI) | |

## OS별 사전 준비

**Windows**: Microsoft 입력기(기본). 한자 변환은 한자 키. `Win+Space` 로 입력기 전환 상태 확인.

**Linux (Wayland, Hyprland)**
- fcitx5 사용 시: `WAYLAND_DISPLAY` 존재 확인, fcitx5 가 `text-input-v3` 로 붙는지 확인.
- ibus 사용 시: `GTK_IM_MODULE`/`QT_IM_MODULE` 은 슬린트/winit 과 무관하고, Wayland text-input 프로토콜 지원 여부가 관건.
- 실행 전 `ime-winit` 가 출력하는 env 스냅샷(`WAYLAND_DISPLAY`, `XMODIFIERS`, `QT_IM_MODULE` 등)을 그대로 남겨둘 것.
- X11 폴백(`DISPLAY` 만 있는 세션)도 같은 표에 기록해 비교한다.

**macOS**: 시스템 설정 → 키보드 → 입력 소스에 "2-Set Korean" 추가. 한자 후보창은 Option+Return.
알려진 이슈: winit 의 macOS IME 는 재작성 진행 중이며(이슈 #4519), 한자 후보창이 열리지 않는
사례가 egui 에도 보고돼 있다(egui #7974). **이 케이스가 실패하면 그 자체가 중요한 결과다.**

## 산출물

- `ime-winit.log` / `ime-winit-report.txt`
- `ime-slint.log` / `ime-slint-report.txt`
