//! PoC1 — Slint(wgpu 백엔드) 안에 "우리가 직접 GPU로 그린 텍스처"를 표시한다.
//!
//! 두 모드를 제공한다.
//!
//! * `--mode shared` : 우리가 만든 `wgpu::Instance/Adapter/Device/Queue` 를 Slint 에 그대로 넘기고
//!   (`WGPUConfiguration::Manual`), 우리 렌더 패스가 그 텍스처를 `slint::Image::try_from` 으로 UI 에
//!   붙인다. 텍스처 복사 0회.
//! * `--mode copy`   : Slint 는 자기 디바이스를 쓰게 두고, 우리 디바이스에서 렌더한 뒤 CPU 로 읽어
//!   (`copy_texture_to_buffer` + `map_async`) `slint::Image::from_rgba8` 으로 업로드한다.
//!   "디바이스를 공유하지 않으면 얼마를 내는가"의 baseline.
//!
//! 확인 포인트
//! - shared 모드가 화면에 정상 출력되면 = **디바이스 공유 성공의 증거**.
//!   Slint 가 별도 디바이스를 만들었다면 다른 디바이스의 텍스처를 샘플링하는 순간 wgpu validation
//!   error 가 나거나 검은 화면이 된다.
//! - Slint 는 dirty-tracking 렌더러이므로 외부 텍스처 내용만 변해도 재렌더가 보장되지 않는다.
//!   여기서는 매 프레임 `Window::request_redraw()` 를 호출한다. (이 PoC 의 핵심 실무 함정)
//!
//! 실행: cargo run --release -p poc1-slint-wgpu-texture -- --mode shared

use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::wgpu_29::wgpu;
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

slint::include_modules!();

const TEX_W: u32 = 1280;
const TEX_H: u32 = 720;
const BYTES_PER_PIXEL: u32 = 4;

/// 전체 화면 삼각형 + 애니메이션 패턴. 합성 결과를 외부 텍스처에 직접 쓴다.
const SHADER: &str = r#"
struct Params {
    time: f32,
    width: f32,
    height: f32,
    _pad: f32,
};

@group(0) @binding(0) var<uniform> params: Params;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    var tri = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    return vec4<f32>(tri[vi], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    let uv = frag.xy / vec2<f32>(params.width, params.height);
    let t = params.time;

    let cx = uv.x - 0.5;
    let cy = uv.y - 0.5;
    let d = sqrt(cx * cx + cy * cy);

    let wave = 0.5 + 0.5 * sin(d * 26.0 - t * 3.0);
    let grid_x = smoothstep(0.97, 1.0, fract(uv.x * 32.0 + t));
    let grid_y = smoothstep(0.97, 1.0, fract(uv.y * 18.0));

    let r = clamp(wave * 0.55 + grid_x * 0.30, 0.0, 1.0);
    let g = clamp((1.0 - wave) * 0.45 + grid_y * 0.30, 0.0, 1.0);
    let b = clamp(0.30 + 0.45 * wave * (1.0 - d), 0.0, 1.0);

    return vec4<f32>(r, g, b, 1.0);
}
"#;

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

fn align_up(v: u32, a: u32) -> u32 {
    ((v + a - 1) / a) * a
}

struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl Gpu {
    fn info_line(&self) -> String {
        let i = self.adapter.get_info();
        format!(
            "adapter={} backend={:?} type={:?} driver={} {}",
            i.name, i.backend, i.device_type, i.driver, i.driver_info
        )
    }
}

fn power_pref() -> wgpu::PowerPreference {
    match arg_value("--power").as_deref() {
        Some("low") => wgpu::PowerPreference::LowPower,
        Some("none") => wgpu::PowerPreference::None,
        _ => wgpu::PowerPreference::HighPerformance,
    }
}

/// 우리가 직접 만든 디바이스. Slint 에 넘길 수도 있고(shared), 안 넘길 수도 있다(copy).
///
/// 주의: 여기서 `compatible_surface: None` 으로 어댑터를 고른다. 하이브리드 GPU 노트북에서는
/// 실제 창 표면과 호환되지 않는 어댑터가 선택될 수 있다. 그 경우 `--power low` 또는 `--adapter N`
/// 으로 바꿔서 시도한다(어댑터 목록은 실행 시 출력된다).
fn create_device(label: &str) -> Result<Gpu, Box<dyn std::error::Error>> {
    let instance = wgpu::Instance::default();

    let adapters = instance.enumerate_adapters(wgpu::Backends::all());
    println!("[poc1] 감지된 어댑터 {}개:", adapters.len());
    for (i, a) in adapters.iter().enumerate() {
        let info = a.get_info();
        println!(
            "   [{i}] {}  backend={:?} type={:?} driver={} {}",
            info.name, info.backend, info.device_type, info.driver, info.driver_info
        );
    }

    let adapter = if let Some(idx) = arg_value("--adapter").and_then(|v| v.parse::<usize>().ok()) {
        adapters
            .into_iter()
            .nth(idx)
            .ok_or_else(|| format!("--adapter {idx} 가 존재하지 않는다(위 목록 확인)"))?
    } else {
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: power_pref(),
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .map_err(|e| format!("어댑터를 찾지 못했다(GPU 드라이버 확인, --power low 도 시도): {e}"))?
    };

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some(label),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::downlevel_defaults(),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        trace: wgpu::Trace::Off,
        experimental_features: wgpu::ExperimentalFeatures::disabled(),
    }))
    .map_err(|e| format!("디바이스 생성 실패: {e}"))?;

    Ok(Gpu { instance, adapter, device, queue })
}

/// 외부 텍스처에 애니메이션 패턴을 그리는 최소 파이프라인.
struct Scene {
    device: wgpu::Device,
    queue: wgpu::Queue,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    pipeline: wgpu::RenderPipeline,
    params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl Scene {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue, usages: wgpu::TextureUsages) -> Self {
        // Slint 의 `TryFrom<wgpu::Texture> for Image` 가 요구하는 조건:
        //   format ∈ { Rgba8Unorm, Rgba8UnormSrgb } 그리고
        //   usage 에 TEXTURE_BINDING 과 RENDER_ATTACHMENT 가 둘 다 포함될 것.
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("poc1-external-texture"),
            size: wgpu::Extent3d { width: TEX_W, height: TEX_H, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: usages,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("poc1-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let params = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("poc1-params"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("poc1-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("poc1-bind-group"),
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: params.as_entire_binding(),
            }],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("poc1-pipeline-layout"),
            bind_group_layouts: &[&bgl],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("poc1-pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self { device: device.clone(), queue: queue.clone(), texture, view, pipeline, params, bind_group }
    }

    fn render(&self, time: f32) {
        let data: [f32; 4] = [time, TEX_W as f32, TEX_H as f32, 0.0];
        self.queue.write_buffer(&self.params, 0, bytemuck_cast(&data));

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("poc1-encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("poc1-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
    }

    /// copy 모드에서만 쓴다: 텍스처를 CPU 로 읽어온다(PCIe 왕복).
    fn read_back(&self, buffer: &wgpu::Buffer, padded_bytes_per_row: u32) -> Result<Vec<u8>, String> {
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("poc1-readback"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(TEX_H),
                },
            },
            wgpu::Extent3d { width: TEX_W, height: TEX_H, depth_or_array_layers: 1 },
        );
        self.queue.submit(Some(encoder.finish()));

        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |res| {
            let _ = tx.send(res);
        });
        self.device
            .poll(wgpu::PollType::Wait)
            .map_err(|e| format!("poll 실패: {e}"))?;
        rx.recv()
            .map_err(|e| format!("map_async 콜백을 받지 못했다: {e}"))?
            .map_err(|e| format!("버퍼 매핑 실패: {e}"))?;

        let data = slice.get_mapped_range().to_vec();
        buffer.unmap();
        Ok(data)
    }
}

fn bytemuck_cast(v: &[f32; 4]) -> &[u8] {
    // bytemuck 의존을 피하려고 직접 캐스팅한다(정렬은 f32 배열이므로 충분).
    unsafe { std::slice::from_raw_parts(v.as_ptr() as *const u8, std::mem::size_of_val(v)) }
}

fn write_report(name: &str, body: &str) {
    let path = format!("{name}-report.txt");
    if let Err(e) = std::fs::write(&path, body) {
        eprintln!("[poc1] 리포트 저장 실패({path}): {e}");
    } else {
        println!("[poc1] 리포트 저장: {path}");
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let mode = arg_value("--mode").unwrap_or_else(|| "shared".to_string());
    println!("[poc1] mode={mode}  texture={TEX_W}x{TEX_H} Rgba8UnormSrgb");

    match mode.as_str() {
        "shared" => run_shared(),
        "copy" => run_copy(),
        other => {
            eprintln!("[poc1] 알 수 없는 모드: {other} (shared | copy)");
            std::process::exit(2);
        }
    }
}

/// 디바이스/큐를 Slint 에 넘기고 텍스처를 제로카피로 표시한다.
fn run_shared() -> Result<(), Box<dyn std::error::Error>> {
    let gpu = create_device("poc1-shared-device")?;
    let info = gpu.info_line();
    println!("[poc1] (shared) 우리 디바이스: {info}");

    let scene = Rc::new(Scene::new(
        &gpu.device,
        &gpu.queue,
        wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
    ));

    // 우리 텍스처를 Slint 이미지로 가져온다(복사 없음, 텍스처 핸들 래핑).
    let image = Image::try_from(scene.texture.clone())
        .map_err(|e| format!("slint::Image::try_from 실패: {e:?}"))?;
    println!("[poc1] (shared) 외부 텍스처를 Slint UI 에 import 성공 (복사 0회)");

    // Slint 백엔드 선택: wgpu 29 + winit, 그리고 우리 instance/adapter/device/queue 를 Manual 로 주입.
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .require_wgpu_29(slint::wgpu_29::WGPUConfiguration::Manual {
            instance: gpu.instance.clone(),
            adapter: gpu.adapter.clone(),
            device: gpu.device.clone(),
            queue: gpu.queue.clone(),
        })
        .select()?;

    let ui = AppWindow::new()?;
    ui.set_preview(image.clone());
    ui.set_status_text(format!("mode=shared   {info}"));
    ui.set_metrics_text("렌더 루프 시작 대기…".into());
    ui.show()?;

    let ui_weak = ui.as_weak();
    let scene_for_timer = scene.clone();
    let image_for_timer = image.clone();
    let start = Instant::now();
    let mut frames: u64 = 0;
    let mut window = Instant::now();
    let mut worst_ms: f64 = 0.0;

    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(8), move || {
        let t0 = Instant::now();
        // 매 프레임 우리 텍스처를 갱신한다.
        scene_for_timer.render(start.elapsed().as_secs_f32());
        // Slint 는 dirty-tracking 이라 외부 텍스처 변경만으로는 재렌더되지 않는다 → 명시적 요청.
        if let Some(ui) = ui_weak.upgrade() {
            ui.window().request_redraw();
            ui.set_preview(image_for_timer.clone());
        }
        let dt = t0.elapsed().as_secs_f64() * 1000.0;
        if dt > worst_ms {
            worst_ms = dt;
        }
        frames += 1;

        if window.elapsed() >= Duration::from_secs(1) {
            let secs = window.elapsed().as_secs_f64();
            let fps = frames as f64 / secs;
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_metrics_text(format!(
                    "{fps:.1} fps   cpu-submit {:.2} ms/frame (worst {:.2})   텍스처 복사 0회 (zero-copy import)",
                    1000.0 / fps.max(0.001),
                    worst_ms
                ));
            }
            frames = 0;
            worst_ms = 0.0;
            window = Instant::now();
        }
    });

    let report = format!(
        "PoC1 shared-mode 보고\nmode=shared\n{info}\ntexture={TEX_W}x{TEX_H} Rgba8UnormSrgb\ncopy_count=0 (slint::Image::try_from)\n"
    );
    write_report("poc1-shared", &report);

    slint::run_event_loop()?;
    Ok(())
}

/// 디바이스를 공유하지 않는 경로의 비용을 측정한다.
fn run_copy() -> Result<(), Box<dyn std::error::Error>> {
    // Slint 는 기본 설정(자기 디바이스)으로 둔다.
    let ui = AppWindow::new()?;
    let gpu = create_device("poc1-separate-device")?;
    let info = gpu.info_line();
    println!("[poc1] (copy) 우리 디바이스(공유 안 함): {info}");

    let scene = Rc::new(Scene::new(
        &gpu.device,
        &gpu.queue,
        wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC,
    ));

    let padded = align_up(TEX_W * BYTES_PER_PIXEL, wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("poc1-readback-buffer"),
        size: (padded as u64) * (TEX_H as u64),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    ui.set_status_text(format!("mode=copy   {info}"));
    ui.set_metrics_text("측정 중…".into());
    ui.show()?;

    let ui_weak = ui.as_weak();
    let scene_for_timer = scene.clone();
    let gpu_device = gpu.device.clone();
    let readback_for_timer = readback;
    let start = Instant::now();
    let mut frames: u64 = 0;
    let mut window = Instant::now();
    let mut acc_ms: f64 = 0.0;
    let mut worst_ms: f64 = 0.0;

    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(8), move || {
        let t0 = Instant::now();
        scene_for_timer.render(start.elapsed().as_secs_f32());
        match scene_for_timer.read_back(&readback_for_timer, padded) {
            Ok(raw) => {
                let mut pb = SharedPixelBuffer::<Rgba8Pixel>::new(TEX_W, TEX_H);
                {
                    let dst = pb.make_mut_bytes();
                    let row_bytes = (TEX_W * BYTES_PER_PIXEL) as usize;
                    for y in 0..TEX_H as usize {
                        let s = y * padded as usize;
                        let d = y * row_bytes;
                        dst[d..d + row_bytes].copy_from_slice(&raw[s..s + row_bytes]);
                    }
                }
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_preview(Image::from_rgba8(pb));
                }
            }
            Err(e) => {
                eprintln!("[poc1] (copy) readback 실패: {e}");
            }
        }
        let dt = t0.elapsed().as_secs_f64() * 1000.0;
        acc_ms += dt;
        if dt > worst_ms {
            worst_ms = dt;
        }
        frames += 1;

        if window.elapsed() >= Duration::from_secs(1) {
            let secs = window.elapsed().as_secs_f64();
            let fps = frames as f64 / secs;
            let avg = acc_ms / frames as f64;
            let mb = (TEX_W as f64 * TEX_H as f64 * 4.0) / (1024.0 * 1024.0);
            if let Some(ui) = ui_weak.upgrade() {
                ui.set_metrics_text(format!(
                    "{fps:.1} fps   왕복 {avg:.2} ms/frame (worst {worst_ms:.2})   프레임당 {mb:.1} MiB 이동(읽기+업로드)"
                ));
            }
            frames = 0;
            acc_ms = 0.0;
            worst_ms = 0.0;
            window = Instant::now();
        }
    });

    let report = format!(
        "PoC1 copy-mode 보고\nmode=copy\n{info}\ntexture={TEX_W}x{TEX_H} Rgba8UnormSrgb\ncopy_count=1 (copy_texture_to_buffer + map_async + Image::from_rgba8)\nbytes_per_frame_mib={:.1}\n",
        (TEX_W as f64 * TEX_H as f64 * 4.0) / (1024.0 * 1024.0)
    );
    write_report("poc1-copy", &report);

    let _ = gpu_device; // 디바이스 수명 유지
    slint::run_event_loop()?;
    Ok(())
}
