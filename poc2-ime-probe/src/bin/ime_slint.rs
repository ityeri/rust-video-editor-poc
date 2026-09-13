//! PoC2-b — 실제 후보 툴킷(Slint)의 IME 실측.
//!
//! raw winit(PoC2-a)이 "플랫폼 레이어"를 본다면, 이쪽은 "제품에 쓸 툴킷이 그 레이어를 제대로
//! 소비하는가"를 본다. 100ms 주기로 각 TextInput 의 text 를 관찰해
//! **조합 중간값(preedit)이 text 프로퍼티에 들어오는지**를 판정한다.
//!
//! 산출물: `ime-slint.log`, `ime-slint-report.txt`

use std::cell::RefCell;
use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::rc::Rc;
use std::time::Duration;

use slint::ComponentHandle;

slint::include_modules!();

#[derive(Default)]
struct Snapshot {
    top: String,
    middle: String,
    bottom: String,
    scale: f32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let ui = ImeWindow::new()?;
    let scale = ui.window().scale_factor();
    let env_line = format!(
        "os={} scale_factor={:.3} WAYLAND_DISPLAY={:?} DISPLAY={:?} XMODIFIERS={:?} QT_IM_MODULE={:?} GTK_IM_MODULE={:?}",
        std::env::consts::OS,
        scale,
        std::env::var("WAYLAND_DISPLAY").ok(),
        std::env::var("DISPLAY").ok(),
        std::env::var("XMODIFIERS").ok(),
        std::env::var("QT_IM_MODULE").ok(),
        std::env::var("GTK_IM_MODULE").ok(),
    );
    ui.set_env_text(env_line.clone().into());
    println!("[ime-slint] {env_line}");

    let log = Rc::new(RefCell::new(
        File::create("ime-slint.log").ok().map(BufWriter::new),
    ));
    let log_for_cb = log.clone();
    ui.on_note(move |msg| {
        println!("[note] {msg}");
        if let Some(f) = log_for_cb.borrow_mut().as_mut() {
            let _ = writeln!(f, "[note] {msg}");
            let _ = f.flush();
        }
    });

    let ui_weak = ui.as_weak();
    let snap = Rc::new(RefCell::new(Snapshot { scale, ..Default::default() }));
    let snap_for_timer = snap.clone();
    let log_for_timer = log.clone();

    // 조합 중간값이 text 로 들어오는지 판정하기 위한 폴링.
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let Some(ui) = ui_weak.upgrade() else { return };
            let mut s = snap_for_timer.borrow_mut();
            let now = (
                ui.get_in_top().to_string(),
                ui.get_in_middle().to_string(),
                ui.get_in_bottom().to_string(),
            );
            let mut changes: Vec<String> = Vec::new();
            if now.0 != s.top {
                changes.push(format!("top: {:?} -> {:?}", s.top, now.0));
                s.top = now.0.clone();
            }
            if now.1 != s.middle {
                changes.push(format!("middle: {:?} -> {:?}", s.middle, now.1));
                s.middle = now.1.clone();
            }
            if now.2 != s.bottom {
                changes.push(format!("bottom: {:?} -> {:?}", s.bottom, now.2));
                s.bottom = now.2.clone();
            }
            let cur_scale = ui.window().scale_factor();
            if (cur_scale - s.scale).abs() > f32::EPSILON {
                changes.push(format!("scale_factor: {:.3} -> {:.3}", s.scale, cur_scale));
                s.scale = cur_scale;
            }
            for c in changes {
                println!("[text] {c}");
                ui.set_log_text(format!("{}\n{}", ui.get_log_text(), c).into());
                if let Some(f) = log_for_timer.borrow_mut().as_mut() {
                    let _ = writeln!(f, "[text] {c}");
                    let _ = f.flush();
                }
            }
        },
    );

    ui.show()?;
    println!("[ime-slint] 창 표시. 위/중간/아래 입력창에서 한글을 입력하고 조합 중간값이 로그에 찍히는지 확인하세요.");

    slint::run_event_loop()?;

    let report = format!(
        "PoC2-b (ime-slint) 요약\n{env_line}\ntop={:?}\nmiddle={:?}\nbottom={:?}\n",
        snap.borrow().top,
        snap.borrow().middle,
        snap.borrow().bottom
    );
    let _ = std::fs::write("ime-slint-report.txt", &report);
    println!("[ime-slint] 리포트 저장: ime-slint-report.txt");
    Ok(())
}
