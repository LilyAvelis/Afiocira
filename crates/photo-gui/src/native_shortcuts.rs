//! Observe paste presses before egui-winit consumes non-text clipboard shortcuts.
use std::{
    marker::PhantomData,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

type Handle = *mut std::ffi::c_void;

#[repr(C)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
struct Message {
    hwnd: Handle,
    message: u32,
    w_param: usize,
    l_param: isize,
    time: u32,
    point: Point,
    private: u32,
}

type HookProc = unsafe extern "system" fn(i32, usize, isize) -> isize;

#[link(name = "user32")]
unsafe extern "system" {
    fn SetWindowsHookExW(
        kind: i32,
        callback: Option<HookProc>,
        module: Handle,
        thread: u32,
    ) -> Handle;
    fn UnhookWindowsHookEx(hook: Handle) -> i32;
    fn CallNextHookEx(hook: Handle, code: i32, w_param: usize, l_param: isize) -> isize;
    fn GetKeyState(key: i32) -> i16;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentThreadId() -> u32;
}

thread_local! {
    static PASTE: AtomicBool = const { AtomicBool::new(false) };
    static INSTALLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub(super) struct Hook {
    handle: Handle,
    // Installation, draining, and destruction must remain on the GUI thread.
    _thread_bound: PhantomData<Rc<()>>,
}

impl Hook {
    pub(super) fn install() -> Result<Self, String> {
        if INSTALLED.with(|installed| installed.get()) {
            return Err("Paste shortcut hook is already installed on this thread.".to_owned());
        }
        // WH_GETMESSAGE=3; a nonzero thread id scopes the hook to this GUI thread.
        // A same-process thread hook needs no DLL module and does not affect other apps.
        let handle = unsafe {
            SetWindowsHookExW(
                3,
                Some(on_message),
                std::ptr::null_mut(),
                GetCurrentThreadId(),
            )
        };
        if handle.is_null() {
            return Err(format!(
                "Could not install paste shortcut hook: {}",
                std::io::Error::last_os_error()
            ));
        }
        PASTE.with(|pending| pending.store(false, Ordering::Relaxed));
        INSTALLED.with(|installed| installed.set(true));
        Ok(Self {
            handle,
            _thread_bound: PhantomData,
        })
    }

    pub(super) fn take_paste(&self) -> bool {
        PASTE.with(|pending| pending.swap(false, Ordering::Relaxed))
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        // This owns the live hook and cannot move off its installing thread.
        unsafe {
            UnhookWindowsHookEx(self.handle);
        }
        INSTALLED.with(|installed| installed.set(false));
        PASTE.with(|pending| pending.store(false, Ordering::Relaxed));
    }
}

unsafe extern "system" fn on_message(code: i32, w_param: usize, l_param: isize) -> isize {
    // For HC_ACTION and PM_REMOVE, Windows supplies a valid MSG for this call.
    // Observing only removed messages prevents duplicate notifications from peeks.
    if code == 0 && w_param == 1 && l_param != 0 {
        let message = unsafe { &*(l_param as *const Message) };
        if message.message == 0x0100 // WM_KEYDOWN
            && message.w_param == 0x56 // VK_V
            && (message.l_param as usize & (1 << 30)) == 0
        // first press, not repeat
        {
            // GetKeyState follows the calling thread's message queue, keeping modifier
            // state aligned with this key message even when dispatch is delayed.
            let (ctrl, alt, shift) = unsafe {
                (
                    GetKeyState(0x11) < 0,
                    GetKeyState(0x12) < 0,
                    GetKeyState(0x10) < 0,
                )
            };
            if ctrl && !alt && !shift {
                // try_with avoids unwinding through the native callback during teardown.
                let _ = PASTE.try_with(|pending| pending.store(true, Ordering::Relaxed));
            }
        }
    }
    // Never consume, modify, or inject a message; always forward all hook codes.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param) }
}
