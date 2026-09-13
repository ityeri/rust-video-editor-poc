//! PoC3 — CubeCL이 "우리 wgpu 디바이스"를 공유할 수 있는가, 그리고 그 결과를 화면까지 옮기는 비용은 얼마인가.
//!
//! 배경(실측 근거)
//! - cubecl-wgpu 0.10 은 wgpu 29 를 요구하고, Slint 1.17 의 `unstable-wgpu-29` 도 wgpu 29 다
//!   → **같은 wgpu 크레이트 인스턴스**를 쓴다(버전 충돌 없음).
//! - CubeCL 공식 예제 `examples/device_sharing` 은 `WgpuSetup { instance, adapter, device, queue, backend }` 를
//!   직접 만들어 `init_device(setup, options) -> WgpuDevice::Existing(_)` 로 등록하는 경로를 제공하며,
//!   문서 주석에 "Useful when you want to share a device between CubeCL and other wgpu-dependent libraries"
//!   라고 명시돼 있다. 이 PoC 는 그 경로를 실제로 밟고, 성능/정확성을 측정한다.
//!
//! 측정 항목
//! - 커널 정확성(CPU 참조 구현과 비교): scale(1D) / blur3_h(2D 인덱싱)
//! - 커널 실행 시간(launch + sync)
//! - `read_one` 비용(= PCIe 왕복, CubeCL 버퍼 → CPU)
//! - 같은 연산을 **WGSL 컴퓨트 셰이더로 텍스처에 직접** 쓸 때의 시간(권장 경로 baseline)
//! - `write_texture` 업로드 비용(CubeCL 결과를 화면 텍스처로 올리는 실제 경로)
//!
//! 실행
//!   cargo run --release -p poc3-cubecl-wgpu-share -- --device existing   # 우리가 만든 디바이스를 CubeCL에 넘김(핵심)
//!   cargo run --release -p poc3-cubecl-wgpu-share -- --device cpu        # GPU 없는 CI(소프트웨어 어댑터)
//!   cargo run --release -p poc3-cubecl-wgpu-share -- --device default    # CubeCL이 고르게 둠(대조군)
//!
//! 산출물: 표준출력 표 + `poc3-report.json`

use std::time::Instant;

use cubecl::prelude::*;
use cubecl::wgpu::{
    init_device, AutoGraphicsApi, GraphicsApi, RuntimeOptions, WgpuDevice, WgpuRuntime, WgpuSetup,
};
use cubecl::Runtime;

/// 실제 영상 프레임 크기(비교 기준). CubeCL 경로는 f32 버퍼, WGSL 경로는 rgba8 텍스처를 쓴다.
const W: u32 = 1920;
const H: u32 = 1080;
const THREADS: u32 = 256;
const ITERS: usize = 20;

const SCALE_SHADER: &str = r#"
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var dst: texture_storage_2d<rgba8unorm, write>;

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let dims = textureDimensions(src);
    if (gid.x >= dims.x || gid.y >= dims.y) { return; }
    let c = textureLoad(src, vec2<i32>(gid.xy), 0);
    let out = clamp(c * 1.1 + vec4<f32>(0.05), vec4<f32>(0.0), vec4<f32>(1.0));
    textureStore(dst, vec2<i32>(gid.xy), out);
}
"#;

// ----------------------------------------------------------------------------
// CubeCL 커널 (순수 Rust)
// ----------------------------------------------------------------------------

/// 픽셀 밝기/게인 연산 — 스텐실 없는 1D 커널의 대표 예.
#[cube(launch)]
fn scale_pixels<F: Float>(input: &Array<F>, output: &mut Array<F>) {
    if ABSOLUTE_POS < input.len() {
        output[ABSOLUTE_POS] = input[ABSOLUTE_POS] * F::new(1.10) + F::new(0.05);
    }
}

/// 가로 3탭 블러 — 2D 인덱싱(pos % width)이 필요하므로 meta 배열로 width/height 를 넘긴다.
#[cube(launch)]
fn blur3_h<F: Float>(input: &Array<F>, meta: &Array<u32>, output: &mut Array<F>) {
    let width = meta[0];
    let height = meta[1];
    let total = width * height;

    if ABSOLUTE_POS < total {
        let x = ABSOLUTE_POS % width;
        let mut left = ABSOLUTE_POS;
        let mut right = ABSOLUTE_POS;

        if x > 0 {
            left = ABSOLUTE_POS - 1;
        }
        if x < width - 1 {
            right = ABSOLUTE_POS + 1;
        }

        output[ABSOLUTE_POS] =
            (input[left] + input[ABSOLUTE_POS] + input[right]) / F::new(3.0);
    }
}

// ----------------------------------------------------------------------------
// 유틸
// ----------------------------------------------------------------------------

fn arg_value(flag: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    let prefix = format!("{flag}=");
    for (i, a) in args.iter().enumerate() {
        if a == flag {
            return args.get(i + 1).cloned();
        }
        if let Some(v) = a.strip_prefix(&prefix) {
            return Some(v.to_string());
        }
    }
    None
}

fn time_avg<F: FnMut()>(iters: usize, mut f: F) -> f64 {
    // 워밍업 1회
    f();
    let t0 = Instant::now();
    for _ in 0..iters {
        f();
    }
    t0.elapsed().as_secs_f64() * 1000.0 / iters as f64
}

fn pattern(len: usize) -> Vec<f32> {
    (0..len).map(|i| ((i % 1024) as f32) / 1023.0).collect()
}

struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn create_device() -> Result<Gpu, Box<dyn std::error::Error>> {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .map_err(|e| format!("어댑터 없음(GPU 드라이버 확인): {e}"))?;

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("poc3-app-device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .map_err(|e| format!("디바이스 생성 실패: {e}"))?;

    Ok(Gpu { instance, adapter, device, queue })
}

// ----------------------------------------------------------------------------
// 본체
// ----------------------------------------------------------------------------

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let device_mode = arg_value("--device").unwrap_or_else(|| "existing".to_string());
    println!("[poc3] device mode = {device_mode}   프레임 = {W}x{H}");

    match device_mode.as_str() {
        "existing" => run_existing()?,
        "cpu" => {
            let device = WgpuDevice::Cpu;
            let client = WgpuRuntime::client(&device);
            println!("[poc3] CubeCL CPU(소프트웨어 어댑터) 디바이스로 실행");
            run_cubecl_part(&client, &device, "cpu", None)?;
        }
        "default" => {
            let device = WgpuDevice::DefaultDevice;
            let client = WgpuRuntime::client(&device);
            println!("[poc3] CubeCL 기본 디바이스로 실행(대조군: 디바이스를 넘기지 않음)");
            run_cubecl_part(&client, &device, "default", None)?;
        }
        other => {
            eprintln!("[poc3] 알 수 없는 --device 값: {other} (existing | cpu | default)");
            std::process::exit(2);
        }
    }
    println!("[poc3] 끝. 생성된 poc3-report.json 을 그대로 전달해 주세요.");
    Ok(())
}

/// 핵심 모드: 우리가 만든 instance/adapter/device/queue 를 CubeCL 에 넘긴다.
fn run_existing() -> Result<(), Box<dyn std::error::Error>> {
    let gpu = create_device()?;
    let info = gpu.adapter.get_info();
    let info_line = format!(
        "adapter={} backend={:?} type={:?} driver={} {}",
        info.name, info.backend, info.device_type, info.driver, info.driver_info
    );
    println!("[poc3] 우리 디바이스: {info_line}");

    // ★ 디바이스 공유의 핵심: 우리 wgpu 핸들을 그대로 담은 WgpuSetup 을 CubeCL 에 등록한다.
    let setup = WgpuSetup {
        instance: gpu.instance.clone(),
        adapter: gpu.adapter.clone(),
        device: gpu.device.clone(),
        queue: gpu.queue.clone(),
        backend: AutoGraphicsApi::backend(),
    };
    let cube_device: WgpuDevice = init_device(setup, RuntimeOptions::default());
    let is_existing = matches!(cube_device, WgpuDevice::Existing(_));
    println!("[poc3] CubeCL 디바이스 = {cube_device:?}  (Existing 여부: {is_existing})");
    assert!(is_existing, "init_device 가 WgpuDevice::Existing 을 돌려주지 않았다");

    let client = WgpuRuntime::client(&cube_device);
    println!("[poc3] CubeCL 클라이언트 생성 완료. 우리 디바이스와 동일한 device/queue 를 공유한다.");

    run_cubecl_part(&client, &cube_device, "existing", Some(&gpu))?;
    Ok(())
}

fn run_cubecl_part(
    client: &ComputeClient<WgpuRuntime>,
    device: &WgpuDevice,
    mode: &str,
    gpu: Option<&Gpu>,
) -> Result<(), Box<dyn std::error::Error>> {
    let pixels = (W * H) as usize;
    let len = pixels * 4; // RGBA f32

    // ---------- 입력 준비 ----------
    let input_cpu = pattern(len);
    let input_bytes = f32::as_bytes(&input_cpu);
    let handle_in = client.create_from_slice(input_bytes);
    let handle_out = client.empty(len * core::mem::size_of::<f32>());

    // ---------- 정확성 1: scale_pixels ----------
    let cubes = len.div_ceil(THREADS as usize) as u32;
    let t0 = Instant::now();
    scale_pixels::launch::<f32, WgpuRuntime>(
        client,
        CubeCount::Static(cubes, 1, 1),
        CubeDim::new(client, THREADS),
        unsafe { ArrayArg::from_raw_parts(handle_in.clone(), len) },
        unsafe { ArrayArg::from_raw_parts(handle_out.clone(), len) },
    );
    pollster::block_on(client.sync()).unwrap();
    let first_call_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let out_bytes = client.read_one(handle_out.clone())?;
    let out = f32::from_bytes(&out_bytes);
    let mut max_err = 0f32;
    for i in 0..len {
        let expect = (input_cpu[i] * 1.10 + 0.05).clamp(0.0, 1.0);
        let got = out[i];
        let e = (got - expect).abs();
        if e > max_err {
            max_err = e;
        }
    }
    let scale_ok = max_err < 1e-4;
    println!("[poc3] scale_pixels: 최대오차={max_err:.3e} → {}", if scale_ok { "PASS" } else { "FAIL" });

    // ---------- 커널 실행 시간(launch + sync) ----------
    let kernel_ms = time_avg(ITERS, || {
        scale_pixels::launch::<f32, WgpuRuntime>(
            client,
            CubeCount::Static(cubes, 1, 1),
            CubeDim::new(client, THREADS),
            unsafe { ArrayArg::from_raw_parts(handle_in.clone(), len) },
            unsafe { ArrayArg::from_raw_parts(handle_out.clone(), len) },
        );
        pollster::block_on(client.sync()).unwrap();
    });

    // ---------- read_one 비용(PCIe 왕복) ----------
    let readback_ms = time_avg(ITERS, || {
        let _ = client.read_one(handle_out.clone()).unwrap();
    });
    let moved_mib = (len * 4) as f64 / (1024.0 * 1024.0);

    // ---------- 정확성 2: blur3_h (2D 인덱싱) ----------
    let meta_cpu: [u32; 2] = [W, H];
    let meta_bytes = <u32 as CubeElement>::as_bytes(&meta_cpu);
    let handle_meta = client.create_from_slice(meta_bytes);
    let handle_blur = client.empty(len * core::mem::size_of::<f32>());

    blur3_h::launch::<f32, WgpuRuntime>(
        client,
        CubeCount::Static(cubes, 1, 1),
        CubeDim::new(client, THREADS),
        unsafe { ArrayArg::from_raw_parts(handle_in.clone(), len) },
        unsafe { ArrayArg::from_raw_parts(handle_meta, 2) },
        unsafe { ArrayArg::from_raw_parts(handle_blur.clone(), len) },
    );
    pollster::block_on(client.sync()).unwrap();
    let blur_bytes = client.read_one(handle_blur)?;
    let blur = f32::from_bytes(&blur_bytes);

    let mut blur_err = 0f32;
    let w = W as usize;
    let rows = H as usize;
    for y in 0..rows {
        for x in 0..w {
            let base = y * w + x;
            let left = if x == 0 { base } else { base - 1 };
            let right = if x == w - 1 { base } else { base + 1 };
            for c in 0..4 {
                let i = base * 4 + c;
                let expect = (input_cpu[left * 4 + c] + input_cpu[i] + input_cpu[right * 4 + c]) / 3.0;
                let e = (blur[i] - expect).abs();
                if e > blur_err {
                    blur_err = e;
                }
            }
        }
    }
    let blur_ok = blur_err < 1e-4;
    println!("[poc3] blur3_h(2D 인덱싱): 최대오차={blur_err:.3e} → {}", if blur_ok { "PASS" } else { "FAIL" });

    // ---------- WGSL 컴퓨트 baseline (같은 디바이스에서 텍스처에 직접 쓰기) ----------
    let mut wgsl_ms = f64::NAN;
    let mut upload_ms = f64::NAN;
    if let Some(gpu) = gpu {
        let device = &gpu.device;
        let queue = &gpu.queue;

        let mk_tex = |label: &str, usage: wgpu::TextureUsages| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage,
                view_formats: &[],
            })
        };
        let src = mk_tex("poc3-src", wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
        let dst = mk_tex(
            "poc3-dst",
            wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let src_view = src.create_view(&wgpu::TextureViewDescriptor::default());
        let dst_view = dst.create_view(&wgpu::TextureViewDescriptor::default());

        // 입력 텍스처 채우기
        let rgba: Vec<u8> = (0..(W * H * 4) as usize).map(|i| (i % 251) as u8).collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &src,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(W * 4),
                rows_per_image: Some(H),
            },
            wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
        );

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("poc3-scale-wgsl"),
            source: wgpu::ShaderSource::Wgsl(SCALE_SHADER.into()),
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("poc3-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("poc3-bg"),
            layout: &bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&src_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&dst_view) },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("poc3-pl"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("poc3-pipeline"),
            layout: Some(&layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        let dispatch = || {
            let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("poc3-wgsl-encoder"),
            });
            {
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("poc3-wgsl-pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &bind_group, &[]);
                pass.dispatch_workgroups(W.div_ceil(8), H.div_ceil(8), 1);
            }
            queue.submit(Some(enc.finish()));
            let _ = device.poll(wgpu::PollType::Wait);
        };
        wgsl_ms = time_avg(ITERS, dispatch);

        // CubeCL 결과를 화면 텍스처로 올리는 실제 경로 비용(read_one → write_texture)
        let bytes = client.read_one(handle_out.clone())?;
        upload_ms = time_avg(ITERS, || {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &dst,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(W * 4),
                    rows_per_image: Some(H),
                },
                wgpu::Extent3d { width: W, height: H, depth_or_array_layers: 1 },
            );
            let _ = device.poll(wgpu::PollType::Wait);
        });
    }

    // ---------- 요약 ----------
    let wgsl_mib = (W as f64 * H as f64 * 4.0) / (1024.0 * 1024.0);
    println!();
    println!("================ PoC3 요약 (device={mode}) ================");
    println!("프레임 {W}x{H} / CubeCL 연산은 f32 버퍼({moved_mib:.1} MiB), WGSL 연산은 rgba8 텍스처({wgsl_mib:.1} MiB)");
    println!("정확성     : scale={}  blur3_h={}", if scale_ok { "PASS" } else { "FAIL" }, if blur_ok { "PASS" } else { "FAIL" });
    println!("첫 호출(컴파일 포함) : {first_call_ms:.1} ms   ← CubeCL 커널 JIT 컴파일 비용");
    println!("커널 실행  : {kernel_ms:.3} ms/frame   (launch + sync, GPU 시간)");
    println!("read_one   : {readback_ms:.3} ms/frame   (PCIe 왕복 {moved_mib:.1} MiB)  ← CubeCL 버퍼를 CPU 로");
    println!("WGSL 컴퓨트: {wgsl_ms:.3} ms/frame   (텍스처 → 텍스처, 왕복 0)");
    println!("업로드      : {upload_ms:.3} ms/frame   (read_one 결과를 write_texture 로 화면 텍스처에)");
    println!("→ CubeCL 결과를 화면까지 올리는 총비용 ≈ read_one + write_texture = {:.3} ms/frame", readback_ms + upload_ms);
    println!("   커널 자체는 {kernel_ms:.3} ms 이므로, 병목은 커널이 아니라 **인터롭(왕복)** 이다.");
    println!("==========================================================");

    let json = format!(
        concat!(
            "{{\n",
            "  \"device_mode\": \"{}\",\n",
            "  \"frame\": \"{}x{}\",\n",
            "  \"cubecl_bytes_per_frame_mib\": {:.2},\n",
            "  \"scale_ok\": {},\n",
            "  \"scale_max_err\": {:.3e},\n",
            "  \"blur_ok\": {},\n",
            "  \"blur_max_err\": {:.3e},\n",
            "  \"first_call_ms\": {:.3},\n",
            "  \"kernel_ms\": {:.4},\n",
            "  \"read_one_ms\": {:.4},\n",
            "  \"wgsl_compute_ms\": {:.4},\n",
            "  \"upload_ms\": {:.4},\n",
            "  \"iterations\": {}\n",
            "}}\n"
        ),
        mode, W, H, moved_mib, scale_ok, max_err, blur_ok, blur_err, first_call_ms, kernel_ms,
        readback_ms, wgsl_ms, upload_ms, ITERS,
    );
    std::fs::write("poc3-report.json", &json)?;
    println!("[poc3] 리포트 저장: poc3-report.json");

    let _ = device;
    Ok(())
}
