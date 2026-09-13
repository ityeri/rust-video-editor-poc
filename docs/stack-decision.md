# 스택 결정 기록 (2026-09)

요구사항: (1) GPU 처리가 무거운 영상 편집기 (2) 풍부한 한국어 IME (3) Windows/Linux/macOS
(4) HiDPI (5) 미려한 디자인 (6) 셰이더를 가능하면 Rust 로

## 결론

| 레이어 | 선택 | 이유 |
|---|---|---|
| UI 툴킷 | **Slint 1.17 (renderer-femtovg-wgpu)** | wgpu 백엔드 + 외부 텍스처 주입 API(`TryFrom<wgpu::Texture>`)를 가진 3 OS 툴킷 중 IME 이슈가 가장 적고, 선언적 DSL 로 픽셀 퍼펙트 디자인이 가능. 라이선스는 배포 형태에 따라 법무 확인 필요 |
| 미디어 엔진 | FFmpeg(ffmpeg-next 9.0) + 하드웨어 디코드 | 3 OS 하드웨어 디코더(NVDEC/DXVA/VideoToolbox/VA-API) 활용 |
| 프리뷰 표시 | 우리 wgpu 패스 → Slint 에 텍스처 주입 | 복사 0회 (PoC1 검증 대상) |
| 이펙트/합성 | WGSL(+naga_oil 또는 WESL) | 텍스처·샘플러·sRGB 를 다루는 작업은 WGSL 이 사실상 표준. naga 가 Vulkan/HLSL/MSL 로 변환 |
| 무거운 커널 | CubeCL 0.10 (커널 슬롯 한정) | Rust DSL 로 커널 작성. 단 프레임 경로가 아니라 분석/오프라인 작업 위주 배치 (PoC3 검증 대상) |
| Rust 셰이더 | rust-gpu (실험 슬롯) | "not yet production-ready" 명시, SPIR-V 만. 제품 의존 금지 |
| 오디오 | cpal / rodio | — |
| 텍스트 | cosmic-text, 접근성 accesskit | — |

## 근거 (실측)

- **버전 정합**: cubecl-wgpu 0.10 → wgpu **29**, Slint 1.17 `unstable-wgpu-29` → wgpu **29**.
  → 하나의 디바이스로 UI + 컴퓨트를 묶을 수 있다(PoC3 가 검증).
- **디바이스 공유 API**: CubeCL `init_device(WgpuSetup{instance,adapter,device,queue,backend}, options)`
  문서 주석이 "share a device between CubeCL and other wgpu-dependent libraries" 라고 명시.
- **IME 이슈 수(2026-09-12 GitHub 검색 기준)**: egui 64 / winit 93 / iced 11 / Slint 12 / makepad 15 /
  gpui(Zed) 147. winit 는 iced·egui·slint 의 공통 하위 레이어이므로 macOS IME 리스크는 공통 상속된다.
  egui 는 "IME input broken" 과 macOS 한국어 한자 후보창 이슈가 열려 있어 사용자 대상 제품엔 부적합.
- **GPU 인터롭**: egui_wgpu(`register_native_texture`), Slint(`TryFrom<wgpu::Texture>`) 는 공개 API.
  gpui 는 wgpu 가 아니라 Blade(Metal/Vulkan) 기반이라 미디어 파이프라인을 별도로 얹기 어렵다.
  Tauri 는 IME/HiDPI 가 최상급이지만 프리뷰 프레임을 웹뷰까지 옮기는 비용이 크다.

## 배제한 선택과 이유

| 후보 | 배제 이유 |
|---|---|
| egui | 한국어 IME 이슈가 열려 있고(조합/후보창), 장기적 디자인 자유도가 낮다. 내부 툴엔 여전히 최적 |
| iced 0.14 | IME 가 거칠고(winit 의존), 릴리스 주기가 길다 |
| GPUI | 성능·IME 는 최고 수준이나 0.x + 문서 부재 + Blade 종속 |
| Tauri | IME/HiDPI 는 최고, GPU 인터롭이 최악(4K60 프리뷰 부적합) |
| Qt(cxx-qt 0.10) | 업계 표준 경로이지만 "Rust 로 만든다"는 목표와 충돌. 최후의 안전판으로만 고려 |

## 미검증 항목 → 이 저장소의 PoC 로 확인

1. Slint 에 외부 wgpu 텍스처를 제로카피로 붙일 수 있는가 → PoC1
2. 3 OS 한국어 IME 가 실사용 가능한 수준인가 → PoC2
3. CubeCL 을 우리 디바이스에 얹을 수 있고, 인터롭 비용이 얼마인가 → PoC3
