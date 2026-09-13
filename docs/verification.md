# 검증 기록

이 문서는 **이 저장소를 만든 환경에서 실제로 확인한 것**과 **확인하지 못한 것**을 구분해 기록한다.
(수치나 실행 결과를 추정으로 채우지 않는다.)

## 1. 작성 환경

| 항목 | 값 |
|---|---|
| OS | Linux 6.12.57-talos (컨테이너), 사용자 `agent` |
| GPU | **없음** (`/dev/dri` 부재, vulkan ICD 없음) |
| 디스플레이 | **없음** (`DISPLAY`, `WAYLAND_DISPLAY` 미설정) |
| GUI 시스템 라이브러리 | **없음** (fontconfig/freetype/xkbcommon/wayland/x11/vulkan 전부 미설치) |
| Rust | 초기에는 없음 → rustup 으로 stable 1.98.1 (musl) 설치하여 `/config/.cargo` 에 배치 |
| 기타 | git 2.54 / python3 3.14 / curl / jq 사용 가능했음 (아래 3절 참조) |

→ 구조적으로 **GUI 실행도, GPU 실행도, IME 실행도 불가능**한 환경이다. 따라서 이 저장소의 실행 결과는
전부 사용자 실기기 검증 항목이다.

## 2. 소스 수준에서 확정한 API (실제 배포 크레이트 소스, 2026-09-13 확인)

추정이 아니라, crates.io 에서 내려받은 실제 크레이트 소스에서 심볼과 필드를 직접 확인했다.

### Slint 1.17.1
| 근거 위치 | 확인 내용 |
|---|---|
| `i-slint-core-1.17.1/graphics/wgpu_29.rs:77` | `enum WGPUConfiguration` · `Manual { instance, adapter, device, queue }` (비공개 필드 없음) |
| `i-slint-core-1.17.1/graphics/wgpu_29.rs:101` | `impl TryFrom<wgpu_29::Texture> for Image` — 단 **format ∈ {Rgba8Unorm, Rgba8UnormSrgb}**, **usage 에 TEXTURE_BINDING + RENDER_ATTACHMENT 필수**(105~115행) |
| `i-slint-core-1.17.1/graphics/image.rs:803` | `Image::from_rgba8(SharedPixelBuffer<Rgba8Pixel>)` (copy 경로용) |
| `i-slint-core-1.17.1/graphics/image.rs:918` | `Image::to_wgpu_29_texture()` (역방향) |
| `i-slint-backend-selector-1.17.1/api.rs:143/252/265` | `BackendSelector::{require_wgpu_29, backend_name, select}` |
| `i-slint-core-1.17.1/timers.rs` | `Timer::start(&self, TimerMode, Duration, impl FnMut() + 'static)` |
| `i-slint-core-1.17.1/api.rs:550/556` | `Window::{request_redraw, scale_factor}` |
| `slint-1.17.1/Cargo.toml` | feature 이름: `backend-winit`, `renderer-femtovg-wgpu`, `unstable-wgpu-29` (`renderer-wgpu` 라는 feature 는 **없다**) |

### wgpu 29.0.4
| 근거 위치 | 확인 내용 |
|---|---|
| `wgpu-types-29.0.4/src/device.rs:12` | `DeviceDescriptor { label, required_features, required_limits, experimental_features, memory_hints, trace }` |
| `wgpu-29.0.4/src/api/instance.rs:167` | `request_adapter(&RequestAdapterOptions) -> Result<Adapter, _>` |
| `wgpu-29.0.4/src/api/adapter.rs:58` | `request_device(&DeviceDescriptor) -> Result<(Device, Queue), _>` |
| `wgpu-29.0.4/src/api/render_pass.rs:603` | `RenderPassColorAttachment { view, depth_slice, resolve_target, ops }` |
| `wgpu-types-29.0.4/src/texture.rs:795/837/851` | `TexelCopyBufferLayout` / `TexelCopyBufferInfo` / `TexelCopyTextureInfo` |

### CubeCL 0.10.0
| 근거 위치 | 확인 내용 |
|---|---|
| `cubecl-wgpu-0.10.0/src/runtime.rs:210` | `pub struct WgpuSetup { instance, adapter, device, queue, backend }` — 전부 공개 필드 |
| `cubecl-wgpu-0.10.0/src/runtime.rs:223-232` | 문서 주석: "Useful when you want to **share a device between CubeCL and other wgpu-dependent libraries**" · `init_device(setup, options) -> WgpuDevice` |
| `cubecl-wgpu-0.10.0/src/device.rs:50` | `WgpuDevice::Existing(u32)` |
| `cubecl-wgpu-0.10.0/Cargo.toml` | `wgpu` 요구 버전 = **29** (Slint `unstable-wgpu-29` 와 동일 → 크레이트 인스턴스 공유 가능) |
| `cubecl-runtime-0.10.0/src/client.rs:136/287/471/805` | `read_one`, `create_from_slice`, `empty`, `sync` |
| `cubecl-core-0.10.0/src/runtime_tests/assign.rs` | `#[cube(launch)]` + `kernel::launch::<F,R>(&client, CubeCount::Static(..), CubeDim::new(&client, n), unsafe { ArrayArg::from_raw_parts(handle, len) })` |

### winit 0.30.13
| 근거 위치 | 확인 내용 |
|---|---|
| `winit-0.30.13/src/event.rs:774` | `enum Ime` (Enabled / Preedit(String, Option<(usize,usize)>) / Commit(String) / Disabled) |
| `winit-0.30.13/src/window.rs:1248/1283` | `set_ime_cursor_area(position, size)`, `set_ime_allowed(bool)` |
| `winit-0.30.13/src/platform_impl/linux/wayland/seat/text_input/mod.rs` | Wayland `text-input` 경로 존재 |

## 3. 컴파일 검증 — **완료하지 못함** (정직 기록)

시도한 것과 결과:

1. `cargo check -p poc3-cubecl-wgpu-share` 를 `CARGO_BUILD_JOBS=2` 로 실행.
   약 13분간 진행(naga 컴파일 단계까지 도달, rmeta 97개 생성)했으나 **워크스페이스 컨테이너가 재시작**되어
   중단되었다(당시 노드 메모리: 전체 15.6GB 중 13.9GB 사용 → 압박).
2. 재시작 후에는 컨테이너 이미지가 교체되어 **git/python 이 사라졌고**, `/config/.cargo` 의 Rust 툴체인도
   `libgcc_s.so.1` 부재로 실행 불가(`error loading shared library libgcc_s.so.1`)가 되었다.
   → 이후 `cargo fetch` 조차 불가능해져 **재검증도 못 했다.**

**따라서 "이 저장소의 코드가 컴파일된다"는 사실은 이 환경에서 증명되지 않았다.**
사용자 로컬에서 첫 `cargo build` 시 컴파일 오류가 나올 수 있으며, 그 오류 메시지를 그대로 전달하면
바로 수정한다. 특히 아래 항목이 위험 지점이다(소스 대조는 했지만 컴파일러 검증을 못 한 곳).

| 위험 지점 | 위치 | 비고 |
|---|---|---|
| `Instance::enumerate_adapters` 시그니처 | PoC1 `main.rs` | wgpu 29 에서 인자/반환형 확인 필요 |
| `DeviceDescriptor` 전 필드 명시(Default 미사용) | PoC1/PoC3 | 필드 이름/개수가 다르면 오류 |
| `.slint` 문법(`in-out property`, `edited(t) =>`, `read-only`) | PoC1/PoC2 UI | `slint-build` 가 빌드 시 검증 |
| CubeCL 커널의 `meta: &Array<u32>`, `%`/`/` 인덱싱, `F::new(1.10)` | PoC3 | 매크로 확장 단계에서 검증됨 |
| `CubeDim::new(&client, THREADS)` / `ArrayArg::from_raw_parts` 인자 형태 | PoC3 | 0.10 의 실제 시그니처와 일치해야 함 |
| `BackendSelector` + `Image::try_from` 조합의 런타임 유효성 | PoC1 | 컴파일로는 검증 불가(실행 필요) |

## 4. 로컬 빌드 준비물(예상)

PoC1/PoC2 는 Slint(winit + femtovg-wgpu) 때문에 플랫폼별 dev 패키지가 필요할 수 있다.

- Debian/Ubuntu: `build-essential pkg-config libfontconfig1-dev libfreetype-dev libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libvulkan-dev`
- Fedora: `fontconfig-devel freetype-devel libxkbcommon-devel wayland-devel libX11-devel libXcursor-devel libXrandr-devel libXi-devel vulkan-loader-devel`
- NixOS: `nix-shell -p pkg-config fontconfig freetype libxkbcommon wayland libX11 libXcursor libXrandr libXi vulkan-loader`
- macOS: Xcode Command Line Tools (Metal 사용, 추가 dev 패키지 불필요)
- Windows: MSVC Build Tools (winget: `Microsoft.VisualStudio.2022.BuildTools`)

GPU 드라이버/런타임: Windows=최신 GPU 드라이버, Linux=Vulkan 로더 + 드라이버(Mesa/NVIDIA), macOS=내장(Metal).
