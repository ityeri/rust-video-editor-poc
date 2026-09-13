# PoC3 — CubeCL + wgpu 디바이스 공유와 인터롭 비용

## 가설

1. CubeCL 이 **우리 wgpu 디바이스를 공유**할 수 있다(별도 디바이스/컨텍스트 생성 없이).
2. 그 결과를 화면까지 옮기는 비용이 실제 병목이며, 얼마인지 수치로 알 수 있다.

## 근거 (실측)

- `cubecl-wgpu` 0.10 의 `wgpu` 의존성 = **29**, Slint 1.17 `unstable-wgpu-29` = **29**
  → 동일 크레이트 → **하나의 디바이스로 UI + 컴퓨트를 묶을 수 있다.**
- CubeCL 0.10 소스의 공식 문서 주석:

```rust
/// Create a [`WgpuDevice`] on an existing [`WgpuSetup`].
/// Useful when you want to share a device between `CubeCL` and other wgpu-dependent libraries.
pub fn init_device(setup: WgpuSetup, options: RuntimeOptions) -> WgpuDevice
```

- `WgpuSetup { instance, adapter, device, queue, backend }` 는 **전부 공개 필드**라 우리 핸들을 그대로 넣는다.
- 단, **원시 wgpu::Buffer/Texture 를 CubeCL 커널 입출력으로 직접 꽂는 공개 API 는 없다**(`WgpuResource` 는
  내부용, `WgpuStorage` 도 마찬가지). 따라서 "디코드 텍스처 → CubeCL 커널" 사이에는 여전히
  버퍼 변환/복사가 필요하고, 그 비용을 이 PoC 가 측정한다.

## 실행

```bash
cargo run --release -p poc3-cubecl-wgpu-share -- --device existing   # 핵심: 우리 디바이스를 넘긴다
cargo run --release -p poc3-cubecl-wgpu-share -- --device default    # 대조군: CubeCL 이 스스로 고름
cargo run --release -p poc3-cubecl-wgpu-share -- --device cpu        # GPU 없는 CI/노트북(소프트웨어 어댑터)
```

## 출력 예시(항목 설명)

```
첫 호출(컴파일 포함) : ... ms     ← CubeCL 커널 JIT 컴파일. 앱 시작 시 1회 지불(사전 워밍업 필요)
커널 실행            : ... ms/frame
read_one             : ... ms/frame   (PCIe 왕복 33.2 MiB)   ← CubeCL 버퍼 → CPU
WGSL 컴퓨트          : ... ms/frame   (텍스처 → 텍스처, 왕복 0)
업로드               : ... ms/frame   (read_one 결과 → write_texture)
```

## 왜 이 수치가 의사결정에 중요한가

- `kernel` 이 작고 `read_one + upload` 가 크면 → **CubeCL 을 프레임 경로에 쓰면 손해**다.
  이 경우 CubeCL 은 "오프라인 분석(색변환 테이블 생성, 모션 추정, 노이즈 프로파일)"처럼
  결과를 매 프레임 왕복시키지 않는 작업에 배치하는 것이 맞다.
- WGSL 컴퓨트와의 차이가 크면 → 합성/이펙트는 WGSL 로, 커널은 CubeCL 로 분리하는 하이브리드가 정답.

## 확인 체크리스트

- [ ] `--device existing` 에서 `CubeCL 디바이스 = Existing(0)` 이 출력된다
- [ ] `scale_pixels` PASS
- [ ] `blur3_h(2D 인덱싱)` PASS ← 커널에서 2D 인덱싱(pos % width)이 실제로 동작하는지
- [ ] 첫 호출 시간(컴파일)이 이후 실행보다 훨씬 길다
- [ ] `read_one` 이 커널 시간보다 유의미하게 크다(예상: PCIe 대역폭 한계)
- [ ] `poc3-report.json` 이 생성된다
- [ ] CUDA 백엔드와 비교하고 싶다면 `cubecl` 에 `cuda` feature 를 켜고 `--device` 를 CUDA 로 확장(미구현)

## 알려진 한계

- CubeCL 경로는 f32 버퍼(33.2 MiB/frame), WGSL 경로는 rgba8 텍스처(8.3 MiB/frame)를 다룬다.
  **바이트 수가 달라 절대 수치를 그대로 비교하면 안 된다**. 비교해야 할 것은
  "왕복이 있는가/없는가"와 그 크기다.
- 커널은 의도적으로 단순하다(게인/블러). 목적은 성능 최적화가 아니라 **구조적 비용 측정**이다.
