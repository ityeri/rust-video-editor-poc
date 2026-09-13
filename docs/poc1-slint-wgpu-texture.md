# PoC1 — Slint + 외부 wgpu 텍스처

## 가설

1. 우리가 만든 `wgpu::Device`/`Queue` 를 Slint 에 넘겨 **같은 디바이스**를 쓰게 할 수 있다.
2. 우리 렌더 패스가 그린 텍스처를 **복사 없이** Slint UI 안에 표시할 수 있다.
3. 디바이스를 공유하지 않는 경로(읽기→업로드)의 비용은 얼마인가.

## 구현 요약

| 모드 | Slint 디바이스 | 표시 경로 | 복사 횟수 |
|---|---|---|---|
| `shared` | 우리 디바이스를 `WGPUConfiguration::Manual` 로 주입 | `slint::Image::try_from(우리 텍스처)` | **0회** |
| `copy` | Slint 자체 디바이스 | `copy_texture_to_buffer` → `map_async` → `Image::from_rgba8` | 1회(왕복) |

핵심 API (Slint 1.17.1 소스에서 실측)

```rust
slint::BackendSelector::new()
    .backend_name("winit".into())
    .require_wgpu_29(slint::wgpu_29::WGPUConfiguration::Manual { instance, adapter, device, queue })
    .select()?;

// 제약: format ∈ {Rgba8Unorm, Rgba8UnormSrgb}, usage 에 TEXTURE_BINDING + RENDER_ATTACHMENT 필수
let image = slint::Image::try_from(texture.clone())?;
```

## 함정 (PoC 에서 명시적으로 처리한 것)

- **dirty tracking**: Slint 는 프로퍼티가 안 바뀌면 재렌더하지 않는다. 외부 텍스처 내용만 갱신하면
  화면이 멈춘 것처럼 보일 수 있다 → 매 프레임 `Window::request_redraw()` 호출.
  실무에서는 이 호출이 필요하다는 사실 자체가 설계 제약(프레임 페이싱을 직접 관리해야 함)이다.
- **단일 크레이트 보장**: PoC1 은 `wgpu` 를 직접 의존하지 않고 `slint::wgpu_29::wgpu` 재export 만 쓴다.
  wgpu 를 직접 의존하면 버전이 갈릴 위험이 생긴다.
- **검은 화면이 나오면**: 디바이스 공유 실패(또는 wgpu validation error)일 가능성이 높다.
  `RUST_LOG=warn` 으로 실행하면 naga/wgpu 메시지가 보인다.
- **하이브리드 GPU 노트북**: 우리가 고른 어댑터가 창 표면과 호환되지 않으면 창이 뜨지 않거나
  깜빡일 수 있다. 실행 시 출력되는 어댑터 목록을 보고 `--power low`(내장 GPU) 또는 `--adapter N`
  으로 바꿔 시도한다. 이 실패 자체가 "디바이스를 직접 만들어 넘기는 전략의 실제 제약"이라는
  중요한 결과이므로, 실패하면 그 조합을 기록해 주세요.

## 확인 방법

```bash
cargo run --release -p poc1-slint-wgpu-texture -- --mode shared
```

- [ ] 창에 애니메이션 패턴(파문+격자)이 부드럽게 나온다 → **디바이스 공유 성공**
- [ ] 하단 초록 텍스트의 fps 가 60 근처(모니터 주사율)로 유지된다
- [ ] 리사이즈/최대화/최소화 후에도 계속 렌더된다
- [ ] HiDPI(200% 등)에서 UI 텍스트는 선명하고, 프리뷰는 뷰포트 픽셀 크기로 렌더된다
- [ ] GPU 사용률/전력이 실제로 올라간다(작업관리자·nvidia-smi·Activity Monitor로 확인 → "GPU로 그리고 있다"는 증거)

```bash
cargo run --release -p poc1-slint-wgpu-texture -- --mode copy
```

- [ ] 같은 화면이 나오지만 fps/프레임당 비용이 shared 보다 확실히 나쁘다
- [ ] `poc1-copy-report.txt` / `poc1-shared-report.txt` 가 생성된다

## 기록할 수치

| 항목 | shared | copy |
|---|---|---|
| fps | | |
| 프레임당 CPU 제출 시간(ms) | | |
| 프레임당 이동 바이트 | 0 | ~7.0 MiB (1280×720×4 읽기+쓰기) |
| 눈에 보이는 지연/티어링 | | |
