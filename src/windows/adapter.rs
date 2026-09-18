//! Win32 input adapter.
//!
//! Keys are injected as set-1 *scan codes* (`KEYEVENTF_SCANCODE`), which is what
//! StarCraft 1 reacts to, and are independent of the active layout. The left
//! click is injected without `MOUSEEVENTF_MOVE`, so it lands on the *current*
//! cursor position — no coordinate is stored anywhere in this program.
//!
//! Every safety check runs before *each* event: the foreground window must
//! belong to the configured exe, and no Ctrl/Alt/Shift/Win may be held.

use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BitBlt, CAPTUREBLT, ClientToScreen, CreateCompatibleBitmap,
    CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HDC, HGDIOBJ,
    ReleaseDC, SRCCOPY, SelectObject,
};
use windows_sys::Win32::Storage::Xps::PrintWindow;
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP,
    KEYEVENTF_SCANCODE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT, SendInput,
    VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU,
    VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId, IsIconic,
    SetCursorPos,
};

use crate::frame::{Frame, Point, Rect};
use crate::input::{DesktopAdapter, InputAdapter, InputError, Modifier, unowned_modifiers};
use crate::macros::Key;

/// Modifiers that must not be held while injecting (checked individually so the
/// error message can name them).
const MODIFIER_KEYS: [(u16, &str); 11] = [
    (VK_LSHIFT, "left shift"),
    (VK_RSHIFT, "right shift"),
    (VK_SHIFT, "shift"),
    (VK_LCONTROL, "left ctrl"),
    (VK_RCONTROL, "right ctrl"),
    (VK_CONTROL, "ctrl"),
    (VK_LMENU, "left alt"),
    (VK_RMENU, "right alt"),
    (VK_MENU, "alt"),
    (VK_LWIN, "left win"),
    (VK_RWIN, "right win"),
];

/// Longest process image path we accept from `QueryFullProcessImageNameW`.
const MAX_PATH_CHARS: usize = 512;

/// Upper bound for one client capture (about 4× a 4K RGBA frame). Guards
/// against an absurd client size allocating without limit.
const MAX_CAPTURE_BYTES: usize = 3840 * 2160 * 4 * 2;

/// Injects input into the foreground window, if it belongs to `target_process`.
pub struct SendInputAdapter {
    target_process: String,
    held_keys: Vec<Key>,
    left_held: bool,
    /// Modifiers this adapter pressed itself for a chord, and therefore must
    /// not treat as "the user is holding a modifier".
    owned_modifiers: Vec<Modifier>,
    // Store the handle as an integer so this worker-owned adapter stays Send.
    // F6 binds one window for the whole run; F7 does not need a screen profile.
    row_window: Option<isize>,
}

impl SendInputAdapter {
    /// `target_process` is an exe basename such as `StarCraft.exe`.
    pub fn new(target_process: &str) -> Self {
        Self {
            target_process: target_process.trim().to_owned(),
            held_keys: Vec::with_capacity(Key::ALL.len()),
            left_held: false,
            owned_modifiers: Vec::with_capacity(Modifier::ALL.len()),
            row_window: None,
        }
    }

    fn bind_row_window(&mut self) -> Result<HWND, InputError> {
        self.safety_check()?;
        let needs_binding = self.row_window.is_none();
        let window = match self.row_window {
            Some(handle) => handle as HWND,
            // The safety gate just verified the foreground process. Bind that
            // exact window, not the first visible window found by enumeration
            // (which may belong to another instance of the game).
            None => unsafe { GetForegroundWindow() },
        };
        validate_row_window(window)?;
        self.row_window = Some(window as isize);
        if needs_binding {
            // Focus might have changed between checking the process name and
            // obtaining the handle. Recheck the bound identity before using it.
            self.safety_check()?;
        }
        Ok(window)
    }

    fn target_process(&self) -> &str {
        &self.target_process
    }

    fn send(&self, input: INPUT) -> Result<(), InputError> {
        // SAFETY: `input` is fully initialised and `cbSize` matches the struct
        // size the API expects for this ABI.
        let sent = unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) };
        if sent == 1 {
            Ok(())
        } else {
            Err(InputError::Injection(describe_send_input_failure(unsafe {
                GetLastError()
            })))
        }
    }

    fn note_modifier(&mut self, key: Key, pressed: bool) {
        let modifier = match key {
            Key::Control => Modifier::Control,
            Key::Shift => Modifier::Shift,
            _ => return,
        };
        if pressed {
            if !self.owned_modifiers.contains(&modifier) {
                self.owned_modifiers.push(modifier);
            }
        } else {
            self.owned_modifiers.retain(|owned| *owned != modifier);
        }
    }

    fn send_key(&self, key: Key, key_up: bool) -> Result<(), InputError> {
        let flags = KEYEVENTF_SCANCODE | if key_up { KEYEVENTF_KEYUP } else { 0 };
        let mut input = INPUT {
            r#type: INPUT_KEYBOARD,
            ..Default::default()
        };
        // Writing a union field is safe; the payload has no destructor.
        input.Anonymous.ki = KEYBDINPUT {
            // Scan code only: `wVk` is ignored when KEYEVENTF_SCANCODE is set.
            wVk: 0,
            wScan: key.scan_code(),
            dwFlags: flags,
            time: 0,
            dwExtraInfo: 0,
        };
        self.send(input)
    }

    fn send_mouse_left(&self, button_up: bool) -> Result<(), InputError> {
        let mut input = INPUT {
            r#type: INPUT_MOUSE,
            ..Default::default()
        };
        // Writing a union field is safe; the payload has no destructor.
        input.Anonymous.mi = MOUSEINPUT {
            // dx/dy stay 0 and MOUSEEVENTF_MOVE is not set: the click is
            // delivered at the current cursor position.
            dx: 0,
            dy: 0,
            mouseData: 0,
            dwFlags: if button_up {
                MOUSEEVENTF_LEFTUP
            } else {
                MOUSEEVENTF_LEFTDOWN
            },
            time: 0,
            dwExtraInfo: 0,
        };
        self.send(input)
    }
}

impl InputAdapter for SendInputAdapter {
    fn key_down(&mut self, key: Key) -> Result<(), InputError> {
        self.send_key(key, false)?;
        if !self.held_keys.contains(&key) {
            self.held_keys.push(key);
        }
        self.note_modifier(key, true);
        Ok(())
    }

    fn key_up(&mut self, key: Key) -> Result<(), InputError> {
        self.send_key(key, true)?;
        self.held_keys.retain(|held| *held != key);
        self.note_modifier(key, false);
        Ok(())
    }

    fn mouse_left_down(&mut self) -> Result<(), InputError> {
        self.send_mouse_left(false)?;
        self.left_held = true;
        Ok(())
    }

    fn mouse_left_up(&mut self) -> Result<(), InputError> {
        self.send_mouse_left(true)?;
        self.left_held = false;
        Ok(())
    }

    /// Deliberately bypasses [`Self::safety_check`]: a release must always be
    /// possible, even after the user switched away from the game. Owned
    /// modifiers are forgotten here too, so a later run starts clean.
    fn release_all(&mut self) -> Result<(), InputError> {
        let mut first_error = None;
        for key in Key::ALL {
            if self.held_keys.contains(&key)
                && let Err(error) = self.key_up(key)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        if self.left_held
            && let Err(error) = self.mouse_left_up()
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        self.owned_modifiers.clear();
        first_error.map_or(Ok(()), Err)
    }

    fn safety_check(&mut self) -> Result<(), InputError> {
        // The user's own modifiers still block; the Ctrl/Shift this adapter
        // pressed for a chord are excluded.
        let held = unowned_modifiers(&held_modifiers(), &self.owned_modifiers);
        if !held.is_empty() {
            return Err(InputError::Unsafe(format!(
                "modifier keys are held ({}); refusing to inject so the game does not \
                 receive a modified hotkey",
                held.join(", ")
            )));
        }

        match foreground_process_name() {
            Ok(name) if name.eq_ignore_ascii_case(self.target_process()) => {
                if let Some(window) = self.row_window {
                    validate_row_window(window as HWND)?;
                }
                Ok(())
            }
            Ok(name) => Err(InputError::Unsafe(format!(
                "foreground window belongs to '{name}', expected '{}'",
                self.target_process()
            ))),
            Err(detail) => Err(InputError::Unsafe(detail)),
        }
    }
}

impl DesktopAdapter for SendInputAdapter {
    fn cursor_position(&mut self) -> Result<Point, InputError> {
        self.bind_row_window()?;
        let mut point = POINT { x: 0, y: 0 };
        // SAFETY: GetCursorPos writes into a POINT we own.
        let ok = unsafe { GetCursorPos(&mut point) };
        if ok == 0 {
            return Err(InputError::Injection(format!(
                "GetCursorPos failed (GetLastError={})",
                unsafe { GetLastError() }
            )));
        }
        Ok(Point::new(point.x, point.y))
    }

    fn move_cursor(&mut self, point: Point) -> Result<(), InputError> {
        // Moving the cursor is an action in the game, so the same gate that
        // guards injection guards it too.
        self.bind_row_window()?;
        // SAFETY: SetCursorPos takes plain screen coordinates.
        let ok = unsafe { SetCursorPos(point.x, point.y) };
        if ok == 0 {
            return Err(InputError::Injection(format!(
                "SetCursorPos failed (GetLastError={})",
                unsafe { GetLastError() }
            )));
        }
        Ok(())
    }

    fn capture_client(&mut self) -> Result<Frame, InputError> {
        let window = self.bind_row_window()?;
        // The verified foreground client is already composed. Prefer a direct
        // readback instead of asking the game to synchronously render again.
        // Retain PrintWindow for systems whose desktop capture is unavailable
        // or black; callers still reject an unusable fallback frame.
        let frame = match capture_screen_region(Rect::new(0, 0, 1920, 1080)) {
            Ok(frame) if !frame.is_blank() => frame,
            _ => capture_window_client(window)?,
        };
        // Capture can take time: do not accept a frame after a focus/geometry change.
        self.safety_check()?;
        Ok(frame)
    }

    fn capture_region(&mut self, rect: Rect) -> Result<Frame, InputError> {
        if rect.w <= 0
            || rect.h <= 0
            || rect.x < 0
            || rect.y < 0
            || i64::from(rect.x) + i64::from(rect.w) > 1920
            || i64::from(rect.y) + i64::from(rect.h) > 1080
        {
            return Err(InputError::Injection(
                "capture_region needs a positive rectangle inside the calibrated client".to_owned(),
            ));
        }
        let window = self.bind_row_window()?;
        // Fast path: the gate above proved the game is the foreground window, so
        // the composed desktop already holds the client's pixels. Reading back
        // just this small rectangle costs a fraction of a full 1920x1080
        // `PrintWindow` render — this is the per-target verification read, so it
        // dominates the action's speed.
        //
        // A *dark* rectangle is not a failed capture: StarCraft draws black
        // unexplored terrain, space and shadows, and a legitimate preview probe
        // often lands there. Only a minimized window (or a refused `BitBlt`) is
        // treated as "the composed desktop cannot be trusted", because falling
        // back on darkness alone would render the whole client for every dark
        // probe and stall a whole-screen search.
        let minimized = unsafe { IsIconic(window) } != 0;
        if !minimized && let Ok(frame) = capture_screen_region(rect) {
            self.safety_check()?;
            return Ok(frame);
        }
        // Minimized or a refused BitBlt: render the whole client and crop it.
        // Slower, but it is the path that works when the composed desktop is
        // genuinely unavailable.
        let frame = capture_window_client(window)?;
        self.safety_check()?;
        crop_frame(&frame, rect)
    }
}

/// The calibrated detector uses primary-screen coordinates. Refuse windowed,
/// moved, resized or different windows rather than silently clicking stale points.
fn validate_row_window(window: HWND) -> Result<(), InputError> {
    let mut rect = RECT::default();
    let mut origin = POINT::default();
    // SAFETY: query-only calls, with output structures owned by this function.
    let valid = unsafe {
        GetForegroundWindow() == window
            && IsIconic(window) == 0
            && GetClientRect(window, &mut rect) != 0
            && ClientToScreen(window, &mut origin) != 0
    };
    if !valid
        || rect.right - rect.left != 1920
        || rect.bottom - rect.top != 1080
        || origin.x != 0
        || origin.y != 0
    {
        return Err(InputError::Unsafe(
            "F6 needs the same foreground 1920x1080 game client at screen (0,0); \
             window movement, resizing, secondary monitors and focus changes stop the run"
                .to_owned(),
        ));
    }
    Ok(())
}

/// Exe basename of the current foreground window's process.
///
/// Used by the safety gate and shown in the GUI so a refusing macro can be
/// explained to the user.
pub fn foreground_process_name() -> Result<String, String> {
    // SAFETY: GetForegroundWindow is a plain query.
    let window = unsafe { GetForegroundWindow() };
    if window.is_null() {
        return Err("no foreground window".to_owned());
    }
    process_name_of_window(window)
}

/// Exe basename of the process owning `window`.
fn process_name_of_window(window: HWND) -> Result<String, String> {
    // SAFETY: all calls below are plain queries; the process handle is closed
    // on every path.
    unsafe {
        let mut process_id = 0u32;
        GetWindowThreadProcessId(window, &mut process_id);
        if process_id == 0 {
            return Err("window has no process id".to_owned());
        }

        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id);
        if process.is_null() {
            return Err(format!(
                "cannot query process {process_id} (GetLastError={})",
                GetLastError()
            ));
        }

        let mut buffer = [0u16; MAX_PATH_CHARS];
        let mut length = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut length);
        CloseHandle(process);
        if ok == 0 {
            return Err(format!(
                "cannot read the image name of process {process_id} (GetLastError={})",
                GetLastError()
            ));
        }

        let path = String::from_utf16_lossy(&buffer[..length as usize]);
        match path.rsplit(['\\', '/']).next() {
            Some(name) if !name.is_empty() => Ok(name.to_owned()),
            _ => Err(format!("unexpected process image path '{path}'")),
        }
    }
}

/// Makes this process per-monitor DPI aware so cursor coordinates and the
/// `PrintWindow` capture share the same physical pixel space. Failure is not
/// fatal: it only means the manifest already decided.
pub fn make_process_dpi_aware() {
    // SAFETY: a plain process-wide setting, no pointers involved.
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

/// Captures the client area of `window` with `PrintWindow` into a [`Frame`].
///
/// Buffers are bounded by the client size and every GDI handle is released on
/// every path, including the early-error paths.
fn capture_window_client(window: HWND) -> Result<Frame, InputError> {
    let mut rect = RECT::default();
    // SAFETY: GetClientRect writes into a RECT we own.
    if unsafe { GetClientRect(window, &mut rect) } == 0 {
        return Err(InputError::Injection(format!(
            "GetClientRect failed (GetLastError={})",
            unsafe { GetLastError() }
        )));
    }
    let width = rect.right - rect.left;
    let height = rect.bottom - rect.top;
    if width <= 0 || height <= 0 {
        return Err(InputError::Injection(
            "the game client has no size (is the window minimized?)".to_owned(),
        ));
    }
    let Ok(area) = usize::try_from(width).map(|w| w.checked_mul(height as usize)) else {
        return Err(InputError::Injection("client area too large".to_owned()));
    };
    let Some(twice) = area.and_then(|a| a.checked_mul(4)) else {
        return Err(InputError::Injection("client area too large".to_owned()));
    };
    if twice > MAX_CAPTURE_BYTES {
        return Err(InputError::Injection(format!(
            "client area {width}x{height} is too large to capture"
        )));
    }

    // Screen position of the client's top-left corner: coordinates are physical
    // and never assumed to be the desktop origin.
    let mut origin = POINT { x: 0, y: 0 };
    // SAFETY: ClientToScreen maps the point in place.
    if unsafe { ClientToScreen(window, &mut origin) } == 0 {
        return Err(InputError::Injection(format!(
            "ClientToScreen failed (GetLastError={})",
            unsafe { GetLastError() }
        )));
    }

    // SAFETY: every GDI handle created below is released before returning.
    unsafe {
        let screen_dc: HDC = GetDC(std::ptr::null_mut());
        if screen_dc.is_null() {
            return Err(InputError::Injection("GetDC failed".to_owned()));
        }
        let memory_dc = CreateCompatibleDC(screen_dc);
        let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
        let previous = if memory_dc.is_null() || bitmap.is_null() {
            std::ptr::null_mut()
        } else {
            SelectObject(memory_dc, bitmap as HGDIOBJ)
        };

        let result = if memory_dc.is_null() || bitmap.is_null() || previous.is_null() {
            Err(InputError::Injection(
                "could not create a capture surface".to_owned(),
            ))
        } else {
            // Render while selected, then deselect: GetDIBits requires its
            // source bitmap NOT to be selected into a device context.
            let printed = PrintWindow(window, memory_dc, 3);
            let print_error = GetLastError();
            SelectObject(memory_dc, previous);
            if printed == 0 {
                Err(InputError::Injection(format!(
                    "PrintWindow failed (GetLastError={print_error})"
                )))
            } else {
                capture_into_buffer(memory_dc, bitmap, width, height, origin)
            }
        };

        if !previous.is_null() {
            SelectObject(memory_dc, previous);
        }
        if !bitmap.is_null() {
            DeleteObject(bitmap as HGDIOBJ);
        }
        if !memory_dc.is_null() {
            DeleteDC(memory_dc);
        }
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        result
    }
}

/// Second half of [`capture_window_client`], with the GDI handles already
/// created. Copies BGRA (BI_RGB 32bpp) into RGBA.
///
/// # Safety
/// `bitmap` must contain the rendered client and be deselected from every DC.
unsafe fn capture_into_buffer(
    memory_dc: HDC,
    bitmap: windows_sys::Win32::Graphics::Gdi::HBITMAP,
    width: i32,
    height: i32,
    origin: POINT,
) -> Result<Frame, InputError> {
    let mut info = BITMAPINFO::default();
    info.bmiHeader.biSize = size_of::<windows_sys::Win32::Graphics::Gdi::BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = width;
    // Negative height requests a top-down bitmap, matching the Frame layout.
    info.bmiHeader.biHeight = -height;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;

    let mut buffer = vec![0u8; width as usize * height as usize * 4];
    // SAFETY: buffer is exactly width*height*4 bytes and info describes it.
    let lines = unsafe {
        GetDIBits(
            memory_dc,
            bitmap,
            0,
            height as u32,
            buffer.as_mut_ptr().cast(),
            &mut info,
            DIB_RGB_COLORS,
        )
    };
    if lines != height {
        return Err(InputError::Injection(format!(
            "GetDIBits returned an incomplete capture (GetLastError={})",
            unsafe { GetLastError() }
        )));
    }

    // BI_RGB 32bpp is BGRA; the Frame stores RGBA.
    for pixel in buffer.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    Frame::new(
        width as u32,
        height as u32,
        Point::new(origin.x, origin.y),
        buffer,
    )
    .ok_or_else(|| InputError::Injection("capture buffer size mismatch".to_owned()))
}

/// Modifier keys that are physically down right now.
fn held_modifiers() -> Vec<&'static str> {
    let mut held = Vec::new();
    for (virtual_key, name) in MODIFIER_KEYS {
        // SAFETY: GetAsyncKeyState accepts any virtual key code.
        let state = unsafe { GetAsyncKeyState(i32::from(virtual_key)) };
        if (state as u16) & 0x8000 != 0 {
            held.push(name);
        }
    }
    held
}

/// Captures one screen rectangle straight from the desktop DC.
///
/// Used for the small per-target verification reads: the caller's gate has
/// already established that the game owns the foreground, so the composed
/// desktop contains the same pixels as a full window render while only the
/// requested rectangle is transferred back. `CAPTUREBLT` is included so
/// layered windows are composed as well.
fn capture_screen_region(rect: Rect) -> Result<Frame, InputError> {
    // SAFETY: every GDI handle created here is released before returning.
    unsafe {
        let screen_dc: HDC = GetDC(std::ptr::null_mut());
        if screen_dc.is_null() {
            return Err(InputError::Injection("GetDC failed".to_owned()));
        }
        let memory_dc = CreateCompatibleDC(screen_dc);
        let bitmap = CreateCompatibleBitmap(screen_dc, rect.w, rect.h);
        let previous = if memory_dc.is_null() || bitmap.is_null() {
            std::ptr::null_mut()
        } else {
            SelectObject(memory_dc, bitmap as HGDIOBJ)
        };

        let result = if memory_dc.is_null() || bitmap.is_null() || previous.is_null() {
            Err(InputError::Injection(
                "could not create a capture surface".to_owned(),
            ))
        } else {
            let copied = BitBlt(
                memory_dc,
                0,
                0,
                rect.w,
                rect.h,
                screen_dc,
                rect.x,
                rect.y,
                SRCCOPY | CAPTUREBLT,
            );
            let copy_error = GetLastError();
            SelectObject(memory_dc, previous);
            if copied == 0 {
                Err(InputError::Injection(format!(
                    "BitBlt failed (GetLastError={copy_error})"
                )))
            } else {
                capture_into_buffer(
                    memory_dc,
                    bitmap,
                    rect.w,
                    rect.h,
                    POINT {
                        x: rect.x,
                        y: rect.y,
                    },
                )
            }
        };

        if !previous.is_null() {
            SelectObject(memory_dc, previous);
        }
        if !bitmap.is_null() {
            DeleteObject(bitmap as HGDIOBJ);
        }
        if !memory_dc.is_null() {
            DeleteDC(memory_dc);
        }
        ReleaseDC(std::ptr::null_mut(), screen_dc);
        result
    }
}

/// Crops `rect` (screen pixels) out of a full client `frame`, refusing a
/// rectangle that is not fully inside it.
fn crop_frame(frame: &Frame, rect: Rect) -> Result<Frame, InputError> {
    frame.crop(rect).ok_or_else(|| {
        InputError::Injection(format!(
            "capture region {rect:?} is outside the {}x{} client at ({}, {})",
            frame.width(),
            frame.height(),
            frame.origin().x,
            frame.origin().y
        ))
    })
}

fn describe_send_input_failure(code: u32) -> String {
    // ERROR_ACCESS_DENIED: UIPI blocked the event, typically because the game
    // runs elevated (or a security tool blocks synthetic input).
    if code == 5 {
        return "SendInput was blocked (GetLastError=5, access denied). Run the game without \
                administrator rights, or start this tool with the same integrity level."
            .to_owned();
    }
    format!("SendInput inserted 0 events (GetLastError={code})")
}
