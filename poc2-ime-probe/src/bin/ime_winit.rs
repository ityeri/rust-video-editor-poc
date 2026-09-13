//! PoC2-a — raw winit 으로 IME 이벤트를 그대로 덤프한다.
//!
//! 목적: iced/egui/slint 등 winit 기반 툴킷은 모두 이 레이어를 공유한다. 여기서 나오는 값이
//! "플랫폼에서 IME 가 어떤 순서로 도착하는가"의 원시 데이터다.
//!
//! 관찰 항목
//! - `Ime::Enabled` / `Preedit(text, cursor)` / `Commit(text)` / `Disabled` 시퀀스
//! - 조합 중 키 이벤트가 앱으로 새는지(`text` 가 None 인지, KeyEvent 가 오는지)
//! - 캐럿을 옮겼을 때 후보창이 따라오는지: ←→↑↓ 로 가상 캐럿 이동 → `set_ime_cursor_area` 갱신
//! - 조합 중 포커스 상실(다른 창 클릭) 시 preedit 가 어떻게 되는지
//! - HiDPI: `scale_factor` / `ScaleFactorChanged`
//!
//! 조작
//! - 방향키: 가상 캐럿 이동(후보창 추적 테스트)
//! - 숫자키 1~6: 테스트 케이스 구분선 기록
//! - Ctrl+Q: 종료 (종료 시 요약 리포트 파일 생성)
//!
//! 산출물: `ime-winit.log`(전체 이벤트), `ime-winit-report.txt`(환경+요약)

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write as _};

use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, Ime, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{ImePurpose, Window, WindowId};

const CASES: [&str; 6] = [
    "CASE1: 순수 한글 입력 (예: 안녕하세요)",
    "CASE2: 한자 변환 (Windows 한자키 / macOS Option+Return)",
    "CASE3: 조합 중 포커스 상실 → 복귀",
    "CASE4: 캐럿 이동 중 후보창 위치 추적",
    "CASE5: 영문/숫자/기호 혼합 및 스페이스 확정",
    "CASE6: 자모 단독/복합(ㄱ, ㄲ, ㅘ, ㅢ) 및 Backspace 수정",
];

fn env_snapshot() -> Vec<(String, String)> {
    let keys = [
        "WAYLAND_DISPLAY", "DISPLAY", "XDG_SESSION_TYPE", "XMODIFIERS", "QT_IM_MODULE",
        "GTK_IM_MODULE", "SDL_IM_MODULE", "GLFW_IM_MODULE", "LANG", "LC_ALL", "LC_CTYPE",
        "XKB_DEFAULT_LAYOUT", "SLINT_BACKEND", "IM_MODULE", "INPUT_METHOD",
    ];
    keys.iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
        .collect()
}

struct Probe {
    window: Option<Window>,
    log: Option<BufWriter<File>>,
    caret: LogicalPosition<f64>,
    preedit: String,
    preedit_count: u32,
    commit_count: u32,
    enabled_count: u32,
    disabled_count: u32,
    key_count: u32,
    key_with_text: u32,
    modifiers: ModifiersState,
    scale: f64,
    focused: bool,
    committed: String,
    sections: Vec<String>,
}

impl Probe {
    fn new() -> Self {
        let log = File::create("ime-winit.log").ok().map(BufWriter::new);
        Self {
            window: None,
            log,
            caret: LogicalPosition::new(80.0, 60.0),
            preedit: String::new(),
            preedit_count: 0,
            commit_count: 0,
            enabled_count: 0,
            disabled_count: 0,
            key_count: 0,
            key_with_text: 0,
            modifiers: ModifiersState::empty(),
            scale: 1.0,
            focused: false,
            committed: String::new(),
            sections: Vec::new(),
        }
    }

    fn line(&mut self, s: &str) {
        println!("{s}");
        if let Some(log) = self.log.as_mut() {
            let _ = writeln!(log, "{s}");
            let _ = log.flush();
        }
    }

    fn caret_area(&self) {
        if let Some(w) = &self.window {
            w.set_ime_cursor_area(
                LogicalPosition::new(self.caret.x, self.caret.y),
                LogicalSize::new(2.0, 24.0),
            );
        }
    }

    fn refresh_title(&self) {
        if let Some(w) = &self.window {
            let esc = |s: &str| s.replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t");
            let title = format!(
                "IME probe | enabled={} preedit=\"{}\" ({}) commits={} keys={}+text{} | caret=({:.0},{:.0}) scale={:.2} | Ctrl+Q 종료",
                self.enabled_count > 0,
                esc(&self.preedit),
                self.preedit_count,
                self.commit_count,
                self.key_count,
                self.key_with_text,
                self.caret.x,
                self.caret.y,
                self.scale,
            );
            w.set_title(&title);
        }
    }

    fn report(&mut self) {
        let mut out = String::new();
        let _ = writeln!(out, "PoC2-a (ime-winit) 요약");
        let _ = writeln!(out, "os={} arch={}", std::env::consts::OS, std::env::consts::ARCH);
        for (k, v) in env_snapshot() {
            let _ = writeln!(out, "env {k}={v}");
        }
        let _ = writeln!(out, "scale_factor={:.3}", self.scale);
        let _ = writeln!(
            out,
            "counts: enabled={} preedit={} commit={} disabled={} key_events={} key_events_with_text={}",
            self.enabled_count, self.preedit_count, self.commit_count, self.disabled_count,
            self.key_count, self.key_with_text
        );
        let _ = writeln!(out, "committed_text={}", self.committed);
        if !self.sections.is_empty() {
            let _ = writeln!(out, "-- 테스트 케이스 기록 --");
            for s in &self.sections {
                let _ = writeln!(out, "{s}");
            }
        }
        let _ = std::fs::write("ime-winit-report.txt", &out);
        if let Some(log) = self.log.as_mut() {
            let _ = writeln!(log, "{out}");
            let _ = log.flush();
        }
        println!("[ime-winit] 리포트 저장: ime-winit-report.txt");
    }
}

impl ApplicationHandler for Probe {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let attrs = Window::default_attributes()
            .with_title("IME probe | Ctrl+Q 종료")
            .with_inner_size(LogicalSize::new(1024.0, 240.0));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("[ime-winit] 창 생성 실패: {e}");
                event_loop.exit();
                return;
            }
        };

        self.scale = window.scale_factor();
        window.set_ime_allowed(true);
        window.set_ime_purpose(ImePurpose::Normal);
        self.caret_area();
        self.refresh_title();

        self.line("================ ime-winit 시작 ================");
        self.line(&format!("os={} arch={}", std::env::consts::OS, std::env::consts::ARCH));
        for (k, v) in env_snapshot() {
            self.line(&format!("env {k}={v}"));
        }
        self.line(&format!("scale_factor={:.3}", self.scale));
        self.line("set_ime_allowed(true) / set_ime_purpose(Normal) 완료");
        self.line("조작: 방향키=가상 캐럿 이동, 1~6=테스트 케이스 기록, Ctrl+Q=종료");
        self.line("※ 한글 입력기를 켠 상태에서 창을 클릭해 포커스를 준 뒤 숫자키로 케이스를 표시하고 입력하세요.");

        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.report();
                event_loop.exit();
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                self.line(&format!("[FOCUS] focused={focused} (조합 중이면 여기 preedit 처리 확인)"));
                self.refresh_title();
            }
            WindowEvent::Ime(ime) => {
                match ime {
                    Ime::Enabled => {
                        self.enabled_count += 1;
                        self.line("[IME] Enabled");
                    }
                    Ime::Preedit(text, cursor) => {
                        self.preedit_count += 1;
                        self.preedit = text.clone();
                        let esc = text.replace('\n', "\\n");
                        self.line(&format!("[IME] Preedit len={} cursor={:?} text=\"{esc}\"", text.len(), cursor));
                    }
                    Ime::Commit(text) => {
                        self.commit_count += 1;
                        self.preedit.clear();
                        let esc = text.replace('\n', "\\n");
                        self.line(&format!("[IME] Commit len={} text=\"{esc}\"", text.len()));
                        self.committed.push_str(&text);
                    }
                    Ime::Disabled => {
                        self.disabled_count += 1;
                        self.preedit.clear();
                        self.line("[IME] Disabled");
                    }
                    other => {
                        self.line(&format!("[IME] 기타: {other:?}"));
                    }
                }
                self.refresh_title();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.key_count += 1;
                let has_text = event.text.is_some();
                if has_text {
                    self.key_with_text += 1;
                }
                if event.state == ElementState::Pressed {
                    // 종료 단축키
                    if self.modifiers.control_key()
                        && matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("q"))
                    {
                        self.report();
                        event_loop.exit();
                        return;
                    }
                    // 테스트 케이스 구분선
                    if let Key::Character(c) = &event.logical_key {
                        if let Some(d) = c.chars().next().and_then(|ch| ch.to_digit(10)) {
                            if (1..=6).contains(&d) {
                                let s = format!("== {} ==", CASES[(d - 1) as usize]);
                                self.sections.push(s.clone());
                                self.line(&format!("[CASE] {s}"));
                            }
                        }
                    }
                    // 가상 캐럿 이동 → 후보창 추적 테스트
                    let step = 20.0;
                    let mut moved = true;
                    match &event.logical_key {
                        Key::Named(NamedKey::ArrowLeft) => self.caret.x -= step,
                        Key::Named(NamedKey::ArrowRight) => self.caret.x += step,
                        Key::Named(NamedKey::ArrowUp) => self.caret.y -= step,
                        Key::Named(NamedKey::ArrowDown) => self.caret.y += step,
                        _ => moved = false,
                    }
                    if moved {
                        self.caret_area();
                        self.line(&format!(
                            "[CARET] 이동 → set_ime_cursor_area({:.0},{:.0})  ※후보창이 따라왔는지 눈으로 확인",
                            self.caret.x, self.caret.y
                        ));
                        self.refresh_title();
                    }
                }
                self.line(&format!(
                    "[KEY] state={:?} logical={:?} text={:?} repeat={}",
                    event.state, event.logical_key, event.text, event.repeat
                ));
            }
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
                self.line(&format!("[MOD] {:?}", self.modifiers));
            }
            WindowEvent::CursorMoved { position, .. } => {
                // 마우스 위치를 캐럿 후보 위치로 삼아 후보창 추적을 추가로 시험할 수 있다.
                if self.modifiers.shift_key() {
                    self.caret = LogicalPosition::new(position.x, position.y);
                    self.caret_area();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.scale = scale_factor;
                self.line(&format!("[DPI] scale_factor={scale_factor:.3} (HiDPI 변경 관찰)"));
                self.refresh_title();
            }
            WindowEvent::Resized(size) => {
                self.line(&format!("[RESIZE] {size:?}"));
            }
            _ => {}
        }
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.line("================ ime-winit 종료 ================");
        self.report();
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut probe = Probe::new();
    event_loop.run_app(&mut probe)?;
    Ok(())
}
