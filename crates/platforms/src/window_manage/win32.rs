use raw_window_handle::RawWindowHandle;
use windows::Win32::Foundation::*;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::*;

const SUBCLASS_ID: usize = 4242;

struct MaxSizeData {
    max_width: f32,
    max_height: f32,
}

unsafe extern "system" fn subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    ref_data: usize,
) -> LRESULT {
    // Handle when the window is being queried for its min/max size.
    if msg == WM_GETMINMAXINFO && ref_data != 0 {
        unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
        let data = unsafe { &*(ref_data as *const MaxSizeData) };
        let minmax = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
        let dpi = unsafe { GetDpiForWindow(hwnd) } as f32;
        let scale = dpi / 96.0;
        minmax.ptMaxTrackSize.x = (data.max_width * scale) as i32;
        minmax.ptMaxTrackSize.y = (data.max_height * scale) as i32;
        return LRESULT(0);
    }

    if msg == WM_NCDESTROY {
        if ref_data != 0 {
            unsafe { drop(Box::from_raw(ref_data as *mut MaxSizeData)) };
        }
        let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID);
        }
        return result;
    }

    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// Configures a Win32 window: disables maximize and caps the maximum
/// trackable size (so the user cannot drag-resize beyond `max_size`).
/// Both `max_width` and `max_height` are in logical pixels (96 DPI base).
pub fn configure_max_window_size(
    window: RawWindowHandle,
    max_width: f32,
    max_height: f32,
) -> Result<(), String> {
    let hwnd = match window {
        RawWindowHandle::Win32(w) => HWND(w.hwnd.get() as *mut core::ffi::c_void),
        _ => return Err("not a Win32 window".into()),
    };

    unsafe {
        // 1. Remove WS_MAXIMIZEBOX — disables the maximize button
        //    and blocks double-click-to-maximize on the title bar.
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(hwnd, GWL_STYLE, style & !(WS_MAXIMIZEBOX.0 as isize));

        // Force a frame recalculation so the title bar updates immediately.
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED;
        SetWindowPos(hwnd, None, 0, 0, 0, 0, flags).ok();

        // 2. Install subclass to clamp the max track size via WM_GETMINMAXINFO,
        //    so the window cannot be drag-resized beyond (max_width, max_height).
        let data = Box::new(MaxSizeData {
            max_width,
            max_height,
        });
        SetWindowSubclass(
            hwnd,
            Some(subclass_proc),
            SUBCLASS_ID,
            Box::into_raw(data) as usize,
        )
        .as_bool()
        .then_some(())
        .ok_or_else(|| "SetWindowSubclass returned false".to_string())?;
    }

    Ok(())
}
