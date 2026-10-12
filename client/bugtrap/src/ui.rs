// SPDX-License-Identifier: GPL-3.0-or-later
use crate::text::Wide;
use core::{fmt::Write, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::*, SystemServices::*},
    UI::WindowsAndMessaging::*,
};

// BugTrap-style header, static report text, separator and Close footer.
// No submit button, browser action, email integration, or networking controls.
#[repr(C, align(4))]
struct Template {
    data: [u16; 512],
    n: usize,
}
struct DialogData {
    message: *const u16,
    heading_font: HFONT,
    folder: *const u16,
    filename: *const u16,
    have_dump: bool,
    saved: bool,
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
            SetDlgItemTextW(window, 1031, data.message);
            if data.have_dump {
                SetDlgItemTextW(window, 1034, data.filename);
                SetDlgItemTextW(window, 1035, data.folder);
            } else {
                for id in [1034, 1035, 1037, 1038] {
                    ShowWindow(GetDlgItem(window, id), SW_HIDE);
                }
            }
            SetDlgItemTextW(
                window,
                1036,
                if data.saved {
                    windows_sys::w!(
                        "Attach this file when reporting the crash to SRP.\r\nNothing was uploaded."
                    )
                } else {
                    windows_sys::w!("Nothing was uploaded.")
                },
            );
            SendDlgItemMessageW(
                window,
                1032,
                STM_SETICON,
                LoadIconW(null_mut(), IDI_ERROR) as usize,
                0,
            );
            let mut font: LOGFONTW = core::mem::zeroed();
            let base = SendMessageW(window, WM_GETFONT, 0, 0) as HFONT;
            if GetObjectW(
                base,
                core::mem::size_of::<LOGFONTW>() as i32,
                (&mut font as *mut LOGFONTW).cast(),
            ) != 0
            {
                font.lfHeight = font.lfHeight * 13 / 11;
                font.lfWeight = 600;
                font.lfQuality = CLEARTYPE_QUALITY;
                data.heading_font = CreateFontIndirectW(&font);
                if !data.heading_font.is_null() {
                    SendDlgItemMessageW(window, 1030, WM_SETFONT, data.heading_font as usize, 1);
                }
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
    let mut message = Wide::<34000>::new();
    let split = path.buf[..path.len]
        .iter()
        .rposition(|&c| c == b'\\' as u16 || c == b'/' as u16)
        .map_or(0, |i| i + 1);
    let mut folder = Wide::<32768>::new();
    let _ = folder.wide(&path.buf[..split]);
    if saved {
        let _ = write!(message, "Crash dump saved:");
    } else if partial {
        let _ = write!(
            message,
            "A dump was saved, but could not be fully verified (Windows error {error}):\r\n"
        );
    } else {
        let _ = write!(
            message,
            "Could not save a crash dump beside the executable or in the temporary folder (Windows error {error})."
        );
    }
    let mut t = Template {
        data: [0; 512],
        n: 0,
    };
    t.dword(
        WS_POPUP
            | WS_CAPTION
            | WS_SYSMENU
            | DS_MODALFRAME as u32
            | DS_SETFONT as u32
            | DS_CENTER as u32,
    );
    t.dword(0);
    t.word(10);
    t.word(0);
    t.word(0);
    t.word(400);
    t.word(172);
    t.word(0);
    t.word(0);
    t.text("SRP crash report");
    t.word(11);
    // Segoe UI is usually absent in Wine; choose its available TrueType font
    // explicitly instead of letting an absent Windows font become a bitmap face.
    let wine = GetProcAddress(
        GetModuleHandleW(windows_sys::w!("ntdll.dll")),
        c"wine_get_version".as_ptr().cast(),
    )
    .is_some();
    t.text(if wine { "DejaVu Sans" } else { "Segoe UI" });
    t.control(
        WS_CHILD | WS_VISIBLE | SS_ICON,
        [14, 13, 20, 20],
        1032,
        0x82,
        "",
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_LEFT | SS_NOPREFIX,
        [43, 14, 343, 22],
        1030,
        0x82,
        "Survarium has stopped working",
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_LEFT | SS_NOPREFIX,
        [14, 41, 372, 24],
        1031,
        0x82,
        "",
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_LEFT,
        [14, 66, 40, 14],
        1037,
        0x82,
        "File:",
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_LEFTNOWORDWRAP | SS_NOPREFIX | SS_ENDELLIPSIS,
        [54, 66, 332, 14],
        1034,
        0x82,
        "",
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_LEFT,
        [14, 84, 40, 14],
        1038,
        0x82,
        "Folder:",
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_LEFTNOWORDWRAP | SS_NOPREFIX | SS_PATHELLIPSIS,
        [54, 84, 332, 14],
        1035,
        0x82,
        "",
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_LEFT | SS_NOPREFIX,
        [14, 108, 372, 26],
        1036,
        0x82,
        "",
    );
    t.control(
        WS_CHILD | WS_VISIBLE | SS_ETCHEDHORZ,
        [0, 139, 400, 1],
        1033,
        0x82,
        "",
    );
    t.control(
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
        [316, 148, 70, 18],
        IDCANCEL as u16,
        0x80,
        "Close",
    );
    let mut data = DialogData {
        message: message.ptr(),
        heading_font: null_mut(),
        folder: folder.ptr(),
        filename: path.buf.as_ptr().add(split),
        have_dump: saved || partial,
        saved,
    };
    let result = DialogBoxIndirectParamW(
        null_mut(),
        t.data.as_ptr().cast(),
        null_mut(),
        Some(dialog),
        (&mut data as *mut DialogData) as isize,
    );
    if !data.heading_font.is_null() {
        DeleteObject(data.heading_font);
    }
    if result == -1 {
        if saved || partial {
            let _ = write!(message, "\r\n");
            let _ = message.wide(&path.buf[..path.len]);
        }
        let _ = write!(message, "\r\n\r\nNothing was uploaded.");
        MessageBoxW(
            null_mut(),
            message.ptr(),
            windows_sys::w!("SRP crash report"),
            MB_OK | MB_ICONERROR | MB_TASKMODAL,
        );
    }
}
