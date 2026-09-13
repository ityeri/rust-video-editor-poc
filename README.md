# rust-video-editor-poc

Rust 영상 편집기 스택 결정을 위한 3종 PoC. **로컬(사용자 장비)에서 실행하는 것을 전제**로 작성되었다.
이 저장소를 만든 CI/에이전트 환경은 headless·GPU 없음·디스플레이 없음이므로, 여기서 검증한 것과
사용자가 직접 검증해야 하는 것을 아래에 명시적으로 분리했다.

## PoC 목록

| # | 크레이트 | 검증하려는 가설 |
|---|---|---|
| 1 | `poc1-slint-wgpu-texture` | Slint(wgpu 백엔드)에 **외부 wgpu 텍스처를 제로카피로** 꽂아 표시할 수 있고, **디바이스/큐를 공유**할 수 있는가 (공유 실패 시 비용은 얼마인가) |
| 2 | `poc2-ime-probe` | Windows/Linux/macOS 3종에서 **한국어 IME(조합/preedit/후보창/포커스 전환)** 가 실제로 동작하는가 |
| 3 | `poc3-cubecl-wgpu-share` | CubeCL이 **우리 wgpu 디바이스를 공유**하고, CubeCL 커널 결과를 화면 파이프라인까지 옮기는 비용이 얼마인가 |

## 실행

```bash
# 공통: Rust stable + 플랫폼별 GPU 드라이버
cargo build --release

# PoC1 — 외부 텍스처 주입 (shared = 디바이스 공유 / copy = 읽기-업로드 baseline)
cargo run --release -p poc1-slint-wgpu-texture -- --mode shared
cargo run --release -p poc1-slint-wgpu-texture -- --mode copy

# PoC2 — IME (raw winit 진단 + Slint 위젯 실측)
cargo run --release -p poc2-ime-probe --bin ime-winit
cargo run --release -p poc2-ime-probe --bin ime-slint

# PoC3 — CubeCL 디바이스 공유 + 인터롭 비용
cargo run --release -p poc3-cubecl-wgpu-share -- --device existing
cargo run --release -p poc3-cubecl-wgpu-share -- --device cpu      # GPU 없는 CI용
```

## 검증 상태 (정직 고지)

### 이 저장소를 만든 환경에서 실제로 확인한 것
- **API 실측**: 사용한 모든 외부 API를 실제 배포 크레이트 소스에서 확인했다.
  - Slint 1.17.1: `BackendSelector::{backend_name, require_wgpu_29, select}`,
    `WGPUConfiguration::Manual { instance, adapter, device, queue }`,
    `Image: TryFrom<wgpu_29::Texture>` (제약: `Rgba8Unorm`/`Rgba8UnormSrgb` + `TEXTURE_BINDING|RENDER_ATTACHMENT`),
    `Image::from_rgba8(SharedPixelBuffer<Rgba8Pixel>)`, `Window::request_redraw/scale_factor`, `Timer/TimerMode`.
  - wgpu 29.0.4: `DeviceDescriptor`(label/features/limits/memory_hints/trace/experimental_features),
    `RenderPassColorAttachment{depth_slice}`, `TexelCopy*`, `PollType`, `TextureDescriptor`.
  - CubeCL 0.10.0: `WgpuSetup{instance,adapter,device,queue,backend}`,
    `init_device(setup, RuntimeOptions) -> WgpuDevice::Existing(_)` (공식 예제 `examples/device_sharing` 이
    "**share a device between CubeCL and other wgpu-dependent libraries**" 용도라고 명시),
    `WgpuRuntime::client(&device)`, `#[cube(launch)]`, `client.create_from_slice/read_one/empty`.
  - winit 0.30.13: `Ime` 이벤트, `set_ime_allowed`, `set_ime_cursor_area`, `ImePurpose`.
- **버전 정합**: cubecl-wgpu 0.10 ↔ wgpu 29, Slint 1.17(`unstable-wgpu-29`) ↔ wgpu 29
  → **세 PoC가 같은 wgpu 29 크레이트를 공유**한다. 하나의 디바이스로 Slint UI + CubeCL 커널 +
  자체 렌더 패스를 묶는 구성이 버전 충돌 없이 가능하다.
- **컴파일 검증**: `docs/verification.md` 참고 (여기서 `cargo check` 로 확인한 결과와 미확인 항목을 그대로 기록).

### 사용자가 직접 확인해야 하는 것 (이 환경에서는 불가능)
- GPU가 없고(`/dev/dri` 없음) 디스플레이도 없어 **실제 렌더 결과/프레임레이트/프레임 드랍**은 미검증.
- IME는 **OS 입력기·디스플레이 서버가 필요**하므로 3 OS 실측은 사용자 몫이다.
- HiDPI(fractional scaling), 웨이랜드/맥/윈도우별 렌더러 차이는 실기기에서만 판정 가능.

## 결과 기록

1. 각 PoC는 실행하면 표준출력 + `*.log` / `*-report.txt` / `poc3-report.json` 을 남긴다.
2. `docs/test-matrix.md` 의 체크리스트를 채운다.
3. 리포트 파일과 체크리스트를 그대로 전달하면 항목별로 해석/수정한다.
4. 플랫폼 환경 수집: `scripts/collect-env.sh` (Linux/macOS) 또는 `scripts/collect-env.ps1` (Windows).

## 설계 메모

- PoC1 은 `wgpu` 를 직접 의존하지 않고 `slint::wgpu_29::wgpu` 재export 만 사용한다.
  → "내가 넘긴 텍스처와 Slint가 쓰는 wgpu가 서로 다른 크레이트 인스턴스"라는 사고를 원천 차단.
- `shared` 모드가 성공한다는 것 자체가 **디바이스 공유의 증거**다. 만약 Slint가 별도 디바이스를 만들었다면
  다른 디바이스의 텍스처를 샘플링하는 순간 wgpu validation error 가 발생한다.
- Slint는 dirty-tracking 렌더러라 **외부 텍스처 내용만 바뀌어도 자동 재렌더되지 않을 수 있다**.
  PoC1 은 `Window::request_redraw()` 를 매 프레임 호출해 이 문제를 명시적으로 다룬다(중요한 실무 함정).

## 빌드 전제: C 컴파일러가 필요합니다

`wgpu-hal` 의 빌드 스크립트가 C 컴파일러(`cc`)를 호출한다. 다음 환경에서는 **`cc` 없이 빌드가 실패**한다
(`error: linker 'cc' not found`).

- 최소 컨테이너 이미지(alpine/busybox, distroless, CI 러너 축소 이미지)
- dev 패키지를 설치하지 않은 Linux

해결: Debian/Ubuntu `apt install build-essential pkg-config ...`, Fedora `dnf groupinstall "Development Tools"`,
alpine `apk add build-base pkgconf ...`. `docs/verification.md` 4절에 플랫폼별 패키지 목록이 있다.
