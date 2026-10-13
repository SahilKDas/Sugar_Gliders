#[cfg(windows)]
mod native {
    use windows_sys::Win32::System::{DataExchange::*, Memory::*};
    const UNICODE: u32 = 13;
    struct Clipboard;
    impl Clipboard {
        fn open() -> Option<Self> {
            // SAFETY: a null owner is supported for reading; copying uses the app HWND.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } == 0 {
                None
            } else {
                Some(Self)
            }
        }
    }
    impl Drop for Clipboard {
        fn drop(&mut self) {
            unsafe {
                CloseClipboard();
            }
        }
    }
    pub fn read() -> Option<zeroize::Zeroizing<String>> {
        let _clipboard = Clipboard::open()?;
        // SAFETY: clipboard ownership is held until the guard is dropped. The global
        // buffer is locked, bounded by GlobalSize, and read only before unlocking.
        unsafe {
            let handle = GetClipboardData(UNICODE);
            if handle.is_null() {
                return None;
            }
            let length = GlobalSize(handle) / 2;
            if length == 0 || length > 1_048_576 {
                return None;
            }
            let pointer = GlobalLock(handle) as *const u16;
            if pointer.is_null() {
                return None;
            }
            let data = std::slice::from_raw_parts(pointer, length);
            let end = data.iter().position(|c| *c == 0).unwrap_or(length);
            let value = String::from_utf16_lossy(&data[..end]);
            GlobalUnlock(handle);
            Some(zeroize::Zeroizing::new(value))
        }
    }
    pub fn write(text: &str, owner: windows_sys::Win32::Foundation::HWND) -> bool {
        if owner.is_null() {
            return false;
        }
        let data: Vec<u16> = text.encode_utf16().chain([0]).collect();
        // SAFETY: a valid native window owns the clipboard. Allocate movable memory;
        // successful SetClipboardData transfers ownership to Windows, otherwise free it.
        unsafe {
            if OpenClipboard(owner) == 0 {
                return false;
            }
            let _clipboard = Clipboard;
            let handle = GlobalAlloc(GMEM_MOVEABLE, data.len() * 2);
            if handle.is_null() {
                return false;
            }
            let pointer = GlobalLock(handle) as *mut u16;
            if pointer.is_null() {
                windows_sys::Win32::Foundation::GlobalFree(handle);
                return false;
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), pointer, data.len());
            GlobalUnlock(handle);
            if EmptyClipboard() == 0 || SetClipboardData(UNICODE, handle).is_null() {
                windows_sys::Win32::Foundation::GlobalFree(handle);
                return false;
            }
            true
        }
    }
}
#[cfg(windows)]
pub use native::{read, write};
#[cfg(not(windows))]
pub fn read() -> Option<zeroize::Zeroizing<String>> {
    None
}
