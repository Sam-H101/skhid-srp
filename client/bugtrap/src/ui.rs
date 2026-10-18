use crate::{crash_text, notice_text::language, text::Wide};
use core::{fmt::Write, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::*,
    Globalization::GetUserDefaultUILanguage,
    Graphics::Gdi::*,
    System::SystemServices::*,
    UI::{Controls::*, Shell::ShellExecuteW, WindowsAndMessaging::*},
};

// Original BugTrap icon and white message panel above a system-color footer.
// Open Discord opens SRP Discord in the browser; it never uploads the report.
// ICO payloads start two bytes past a DWORD boundary. Add two leading bytes
// so CreateIconFromResourceEx receives DWORD-aligned resource bits.
#[repr(C, align(4))]
struct IconFile {
    padding: [u8; 2],
    bytes: [u8; include_bytes!("../res/Bug.ico").len()],
}
static BUG_ICON: IconFile = IconFile {
    padding: [0; 2],
    bytes: *include_bytes!("../res/Bug.ico"),
};

unsafe fn bug_icon(small: bool) -> HICON {
    // The original ICO contains 16, 32 and 48 pixel images, in that order.
    let bytes = &BUG_ICON.bytes;
    let entry = 6 + if small { 0 } else { 16 };
    let dword = |offset| {
        u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ])
    };
    let size = dword(entry + 8);
    let offset = dword(entry + 12) as usize;
    let pixels = if small { SM_CXSMICON } else { SM_CXICON };
    let pixels = GetSystemMetrics(pixels);
    CreateIconFromResourceEx(
        bytes.as_ptr().add(offset),
        size,
        1,
        0x00030000,
        pixels,
        pixels,
        LR_DEFAULTCOLOR,
    )
}

#[repr(C, align(4))]
struct Template {
    data: [u16; 1024],
    n: usize,
}
struct DialogData {
    message: *const u16,
    font: HFONT,
    icon: HICON,
    small_icon: HICON,
    folder: *const u16,
    filename: *const u16,
    have_dump: bool,
    text: &'static crash_text::Text,
}

impl Template {
    fn word(&mut self, v: u16) {
        self.data[self.n] = v;
        self.n += 1;
    }
    fn dword(&mut self, v: u32) {
        self.word(v as u16);
        self.word((v >> 16) as u16);
    }
    fn text(&mut self, s: &str) {
        for c in s.encode_utf16() {
            self.word(c);
        }
        self.word(0);
    }
    fn align(&mut self) {
        if self.n % 2 != 0 {
            self.word(0);
        }
    }
    fn control(&mut self, style: u32, bounds: [u16; 4], id: u16, class: u16, text: &str) {
        self.align();
        self.dword(style);
        self.dword(0);
        for v in bounds {
            self.word(v);
        }
        self.word(id);
        self.word(0xffff);
        self.word(class);
        self.text(text);
        self.word(0);
    }
}
unsafe extern "system" fn dialog(window: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    match msg {
        WM_INITDIALOG => {
            let data = &mut *(lp as *mut DialogData);
            // This DLL is Win32-only: the dialog's user slot is a LONG.
            SetWindowLongW(window, DWL_USER as i32, lp as i32);
            // Use the same compact TrueType face for every control. Explicit
            // smoothing avoids Wine's jagged small-font rendering.
            let base = SendMessageW(window, WM_GETFONT, 0, 0) as HFONT;
            let mut font: LOGFONTW = core::mem::zeroed();
            if GetObjectW(
                base,
                core::mem::size_of::<LOGFONTW>() as i32,
                (&mut font as *mut LOGFONTW).cast(),
            ) != 0
            {
                font.lfQuality = ANTIALIASED_QUALITY;
                data.font = CreateFontIndirectW(&font);
                if !data.font.is_null() {
                    for id in [1030, 1031, 1034, 1035, 1036, 1037, 1038, 1013, IDCANCEL] {
                        SendDlgItemMessageW(window, id, WM_SETFONT, data.font as usize, 1);
                    }
                }
            }
            SetDlgItemTextW(window, 1031, data.message);
            if data.have_dump {
                SetDlgItemTextW(window, 1034, data.filename);
                SetDlgItemTextW(window, 1035, data.folder);
            } else {
                for id in [1034, 1035, 1037, 1038] {
                    ShowWindow(GetDlgItem(window, id), SW_HIDE);
                }
            }
            data.icon = bug_icon(false);
            data.small_icon = bug_icon(true);
            if !data.icon.is_null() {
                SendDlgItemMessageW(window, 1032, STM_SETICON, data.icon as usize, 0);
                SendMessageW(window, WM_SETICON, ICON_BIG as usize, data.icon as isize);
            }
            if !data.small_icon.is_null() {
                SendMessageW(
                    window,
                    WM_SETICON,
                    ICON_SMALL as usize,
                    data.small_icon as isize,
                );
            }
            1
        }
        WM_CTLCOLORSTATIC if matches!(GetDlgCtrlID(lp as HWND), 1030..=1032 | 1034..=1039) => {
            SetTextColor(wp as HDC, GetSysColor(COLOR_WINDOWTEXT));
            SetBkColor(wp as HDC, GetSysColor(COLOR_WINDOW));
            GetSysColorBrush(COLOR_WINDOW) as isize
        }
        WM_DRAWITEM if wp == 1013 => {
            let item = &*(lp as *const DRAWITEMSTRUCT);
            let saved = SaveDC(item.hDC);
            let mut rect = item.rcItem;
            let pressed = item.itemState & ODS_SELECTED != 0;
            let disabled = item.itemState & ODS_DISABLED != 0;
            FillRect(item.hDC, &rect, GetSysColorBrush(COLOR_BTNFACE));
            DrawFrameControl(
                item.hDC,
                &mut rect,
                DFC_BUTTON,
                DFCS_BUTTONPUSH
                    | if pressed { DFCS_PUSHED } else { 0 }
                    | if disabled { DFCS_INACTIVE } else { 0 },
            );
            // Draw directly in the button's full client rectangle, using the
            // same font for measurement and painting. Wine's native caption
            // path can offset this label even with BS_CENTER set.
            SelectObject(
                item.hDC,
                SendMessageW(item.hwndItem, WM_GETFONT, 0, 0) as HFONT,
            );
            SetBkMode(item.hDC, TRANSPARENT as i32);
            SetTextColor(
                item.hDC,
                GetSysColor(if disabled {
                    COLOR_GRAYTEXT
                } else {
                    COLOR_BTNTEXT
                }),
            );
            rect = item.rcItem;
            if pressed {
                OffsetRect(&mut rect, 1, 1);
            }
            let mut size: SIZE = core::mem::zeroed();
            let mut label = Wide::<64>::new();
            label.len = GetWindowTextW(item.hwndItem, label.buf.as_mut_ptr(), 64) as usize;
            let mut advances = [0; 64];
            GetTextExtentExPointW(
                item.hDC,
                label.ptr(),
                label.len as i32,
                0,
                null_mut(),
                advances.as_mut_ptr(),
                &mut size,
            );
            for i in (1..label.len).rev() {
                advances[i] -= advances[i - 1];
            }
            SetTextAlign(item.hDC, TA_LEFT | TA_TOP | TA_NOUPDATECP);
            ExtTextOutW(
                item.hDC,
                rect.left + (rect.right - rect.left - size.cx) / 2,
                rect.top + (rect.bottom - rect.top - size.cy) / 2,
                ETO_CLIPPED,
                &rect,
                label.ptr(),
                label.len as u32,
                advances.as_ptr(),
            );
            if item.itemState & ODS_FOCUS != 0 && item.itemState & ODS_NOFOCUSRECT == 0 {
                rect = item.rcItem;
                InflateRect(&mut rect, -3, -3);
                DrawFocusRect(item.hDC, &rect);
            }
            RestoreDC(item.hDC, saved);
            1
        }
        WM_COMMAND if wp as u16 == 1013 => {
            let result = ShellExecuteW(
                window,
                windows_sys::w!("open"),
                windows_sys::w!("https://discord.gg/Pxa4prXfwu"),
                core::ptr::null(),
                core::ptr::null(),
                SW_SHOWNORMAL,
            );
            if result as isize <= 32 {
                let data = &*(GetWindowLongW(window, DWL_USER as i32) as *const DialogData);
                let mut message = Wide::<512>::new();
                let mut title = Wide::<128>::new();
                let _ = write!(
                    message,
                    "{}\r\n\r\n{}",
                    data.text.browser_failed, data.text.manual
                );
                let _ = write!(title, "{}", data.text.title);
                MessageBoxW(
                    window,
                    message.ptr(),
                    title.ptr(),
                    MB_OK | MB_ICONINFORMATION,
                );
            }
            1
        }
        WM_COMMAND if wp as u16 == IDCANCEL as u16 || wp as u16 == IDOK as u16 => {
            EndDialog(window, 0);
            1
        }
        WM_CLOSE => {
            EndDialog(window, 0);
            1
        }
        _ => 0,
    }
}
pub unsafe fn show(path: &Wide<32768>, saved: bool, partial: bool, error: u32) {
    let text = crash_text::text(language(GetUserDefaultUILanguage()));
    let mut title = Wide::<128>::new();
    let _ = write!(title, "{}", text.title);
    let mut message = Wide::<34000>::new();
    let split = path.buf[..path.len]
        .iter()
        .rposition(|&c| c == b'\\' as u16 || c == b'/' as u16)
        .map_or(0, |i| i + 1);
    let mut folder = Wide::<32768>::new();
    let _ = folder.wide(&path.buf[..split]);
    let _ = write!(
        message,
        "{}",
        if saved {
            text.saved
        } else if partial {
            text.partial
        } else {
            text.failed
        }
    );
    if !saved {
        let _ = write!(message, "\r\n({} {error})", text.error);
    }
    let instructions = if saved || partial {
        text.instructions
    } else {
        text.no_file
    };
    let mut t = Template {
        data: [0; 1024],
        n: 0,
    };
    t.dword(
        WS_POPUP
            | WS_CAPTION
            | WS_SYSMENU
            | DS_SETFOREGROUND as u32
            | DS_FIXEDSYS as u32
            | DS_SETFONT as u32
            | DS_CENTER as u32,
    );
    t.dword(0);
    t.word(12);
    t.word(0);
    t.word(0);
    t.word(380);
    t.word(166);
    t.word(0);
    t.word(0);
    t.text(text.title);
    // Match IDD_SIMPLE_DLG in the original BugTrap.rc. The white panel is
    // taller to fit our persistent dump location and local-reporting text.
    t.word(8);
    t.text("Tahoma");
    t.control(
        WS_CHILD | WS_VISIBLE | SS_LEFT,
        [0, 0, 380, 132],
        1039,
        0x82,
        "",
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_ICON,
        [7, 7, 20, 20],
        1032,
        0x82,
        "",
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_LEFT | SS_NOPREFIX,
        [35, 7, 338, 18],
        1030,
        0x82,
        text.intro,
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_LEFT | SS_NOPREFIX,
        [35, 27, 338, 26],
        1031,
        0x82,
        "",
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_LEFT,
        [35, 58, 38, 8],
        1037,
        0x82,
        text.file,
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_LEFTNOWORDWRAP | SS_NOPREFIX | SS_ENDELLIPSIS,
        [75, 58, 298, 8],
        1034,
        0x82,
        "",
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_LEFT,
        [35, 70, 38, 8],
        1038,
        0x82,
        text.folder,
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_LEFTNOWORDWRAP | SS_NOPREFIX | SS_PATHELLIPSIS,
        [75, 70, 298, 8],
        1035,
        0x82,
        "",
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_LEFT | SS_NOPREFIX,
        [35, 86, 338, 38],
        1036,
        0x82,
        instructions,
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_ETCHEDHORZ,
        [0, 131, 380, 1],
        1033,
        0x82,
        "",
    );
    t.control(
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | (BS_DEFPUSHBUTTON | BS_CENTER | BS_VCENTER) as u32,
        [303, 143, 70, 16],
        IDCANCEL as u16,
        0x80,
        text.close,
    );
    t.control(
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
        [7, 143, 86, 16],
        1013,
        0x80,
        text.discord,
    );
    let mut data = DialogData {
        message: message.ptr(),
        font: null_mut(),
        icon: null_mut(),
        small_icon: null_mut(),
        folder: folder.ptr(),
        filename: path.buf.as_ptr().add(split),
        have_dump: saved || partial,
        text,
    };
    let result = DialogBoxIndirectParamW(
        null_mut(),
        t.data.as_ptr().cast(),
        null_mut(),
        Some(dialog),
        (&mut data as *mut DialogData) as isize,
    );
    if !data.font.is_null() {
        DeleteObject(data.font);
    }
    for icon in [data.icon, data.small_icon] {
        if !icon.is_null() {
            DestroyIcon(icon);
        }
    }
    if result == -1 {
        if saved || partial {
            let _ = write!(message, "\r\n");
            let _ = message.wide(&path.buf[..path.len]);
        }
        let _ = write!(message, "\r\n\r\n{}", text.manual);
        MessageBoxW(
            null_mut(),
            message.ptr(),
            title.ptr(),
            MB_OK | MB_ICONERROR | MB_TASKMODAL,
        );
    }
}
