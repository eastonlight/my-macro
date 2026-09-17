//! Entry point.
//!
//! The GUI and every Win32 call are Windows-only. On any other host the program
//! explains the situation and exits with a non-zero code instead of panicking —
//! and it never injects input.

#[cfg(windows)]
fn main() {
    if let Err(error) = oh_my_macro::windows::app::run() {
        eprintln!("oh-my-macro: could not start the user interface: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("oh-my-macro는 Windows 전용입니다. (oh-my-macro requires Windows.)");
    eprintln!();
    eprintln!("입력 주입(SendInput)과 전역 단축키(RegisterHotKey)는 Windows API이므로");
    eprintln!("이 호스트용으로는 GUI와 매크로 실행을 사용할 수 없습니다.");
    eprintln!("This build target does not provide the Windows input adapter or the GUI.");
    eprintln!();
    eprintln!("Windows에서 실행하려면 (build and run on Windows):");
    eprintln!("    cargo run --release");
    eprintln!("다른 호스트에서 교차 확인만 하려면 (cross-check only):");
    eprintln!("    cargo check --target x86_64-pc-windows-gnu");
    eprintln!();
    eprintln!("순수 로직 테스트는 이 호스트에서도 실행할 수 있습니다 (pure tests still work):");
    eprintln!("    cargo test");
    std::process::exit(1);
}
