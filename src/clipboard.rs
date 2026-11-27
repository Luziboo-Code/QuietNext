//! The system pasteboard beyond what GPUI reads.

/// Devtools stand-in for the pasteboard: scripted tests
/// must not touch the shared system clipboard.
#[derive(Clone, Default)]
pub struct FakeClipboard {
    pub html: Option<String>,
    pub text: String,
    pub images: Vec<gpui::Image>,
}

pub static FAKE_CLIPBOARD: std::sync::Mutex<Option<FakeClipboard>> = std::sync::Mutex::new(None);

/// The `public.html` flavor of the general pasteboard (gpui reads only
/// plain text and images).
#[cfg(target_os = "macos")]
pub fn clipboard_html() -> Option<String> {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    const TYPE: &str = "public.html";
    unsafe {
        let pb: *mut Object = msg_send![class!(NSPasteboard), generalPasteboard];
        let ty: *mut Object = msg_send![class!(NSString), alloc];
        let ty: *mut Object = msg_send![ty,
            initWithBytes: TYPE.as_ptr()
            length: TYPE.len()
            encoding: 4usize]; // NSUTF8StringEncoding
        let s: *mut Object = msg_send![pb, stringForType: ty];
        let _: () = msg_send![ty, release];
        if s.is_null() {
            return None;
        }
        let p: *const std::os::raw::c_char = msg_send![s, UTF8String];
        (!p.is_null()).then(|| std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned())
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn clipboard_html() -> Option<String> {
    None
}

/// The Windows `HTML Format` pasteboard flavor (CF_HTML). GPUI reads only
/// plain text and images, so a browser copy loses every `<img>` without this.
#[cfg(target_os = "windows")]
pub fn clipboard_html() -> Option<String> {
    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW};
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    unsafe {
        let format = RegisterClipboardFormatW(windows::core::w!("HTML Format"));
        if format == 0 { return None }
        if OpenClipboard(None).is_err() { return None }
        let html = (|| {
            let handle = GetClipboardData(format).ok()?;
            let global = HGLOBAL(handle.0);
            let size = GlobalSize(global);
            if size == 0 { return None }
            let ptr = GlobalLock(global) as *const u8;
            if ptr.is_null() { return None }
            let bytes = std::slice::from_raw_parts(ptr, size).to_vec();
            let _ = GlobalUnlock(global);
            let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
            parse_cf_html(&bytes[..end])
        })();
        let _ = CloseClipboard();
        html
    }
}

/// The fragment of a CF_HTML buffer, using its byte offsets. Falls back to the
/// fragment comments, then to the first `<` after the ASCII header.
#[cfg(target_os = "windows")]
fn parse_cf_html(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let header = |key: &str| text.lines().find_map(|line| line.strip_prefix(key).and_then(|v| v.trim().parse::<usize>().ok()));
    if let (Some(start), Some(end)) = (header("StartFragment:"), header("EndFragment:"))
        && start < end && end <= bytes.len()
    {
        return String::from_utf8(bytes[start..end].to_vec()).ok();
    }
    let fragments = |open: &str, close: &str| {
        let a = text.find(open)? + open.len();
        let b = text[a..].find(close)? + a;
        String::from_utf8(bytes[a..b].to_vec()).ok()
    };
    if let Some(fragment) = fragments("<!--StartFragment-->", "<!--EndFragment-->") { return Some(fragment) }
    let first = text.find('<')?;
    String::from_utf8(bytes[first..].to_vec()).ok()
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::parse_cf_html;

    #[test]
    fn cf_html_uses_the_fragment() {
        let raw = b"Version:0.9\r\n<html><!--StartFragment--><p>x</p><!--EndFragment--></html>\0";
        assert_eq!(parse_cf_html(&raw[..raw.len() - 1]).as_deref(), Some("<p>x</p>"));
    }
}

