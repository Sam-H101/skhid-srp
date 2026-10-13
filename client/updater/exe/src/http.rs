//! One HTTP GET into memory, through Windows' own WinHTTP: its TLS, its
//! certificate store, the system's proxy settings.

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::ptr::{null, null_mut};

use updater_logic::url::Url;
use windows_sys::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
    WINHTTP_DISABLE_COOKIES, WINHTTP_FLAG_BYPASS_PROXY_CACHE, WINHTTP_FLAG_SECURE,
    WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2, WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3,
    WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_OPTION_REDIRECT_POLICY,
    WINHTTP_OPTION_REDIRECT_POLICY_NEVER, WINHTTP_OPTION_SECURE_PROTOCOLS,
    WINHTTP_QUERY_CONTENT_LENGTH, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption,
    WinHttpSetTimeouts,
};

use crate::win::{Code, last_error, wide};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// A step Windows could not do: no network, no such host, no answer, a
    /// certificate it does not trust. The step's name and the error code.
    Network(&'static str, Code),
    /// The server answered, with another status than 200.
    Status(u32),
    /// The answer is longer than the caller allows.
    TooLarge,
    /// The connection ended before the answer did.
    Cut { got: u64, expected: u64 },
    /// The caller's progress function said stop.
    Stopped,
}

struct Internet(*mut c_void);

impl Drop for Internet {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}

fn handle(raw: *mut c_void, step: &'static str) -> Result<Internet, HttpError> {
    if raw.is_null() {
        Err(HttpError::Network(step, last_error()))
    } else {
        Ok(Internet(raw))
    }
}

fn set_option(handle: &Internet, option: u32, value: u32) -> bool {
    unsafe { WinHttpSetOption(handle.0, option, (&raw const value).cast::<c_void>(), 4) != 0 }
}

fn number_header(request: &Internet, which: u32) -> Option<u32> {
    let mut value = 0_u32;
    let mut size = 4_u32;
    let ok = unsafe {
        WinHttpQueryHeaders(
            request.0,
            which | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&raw mut value).cast::<c_void>(),
            &mut size,
            null_mut(),
        )
    };
    (ok != 0).then_some(value)
}

/// GET `path` from the server: the body, if the status is 200 and the body
/// at most `limit` bytes. `progress(done, total)` is called as the bytes
/// come (the total when the server names it); `false` from it stops the
/// download.
pub fn get(
    url: &Url,
    path: &str,
    limit: usize,
    progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<Vec<u8>, HttpError> {
    let mut body = Vec::new();
    get_stream(
        url,
        path,
        limit as u64,
        &mut |chunk| {
            body.extend_from_slice(chunk);
            Ok(())
        },
        progress,
    )?;
    Ok(body)
}

/// A bounded download to a sink; even multi-gigabyte files use one 64 KiB buffer.
/// Redirects and cookies remain disabled, including for the game mirror.
pub fn get_stream(
    url: &Url,
    path: &str,
    limit: u64,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), HttpError>,
    progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<(), HttpError> {
    let agent = wide("SkhidUpdater/1");
    let mut session = unsafe {
        WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        )
    };
    if session.is_null() {
        // Windows before 8.1 does not know the automatic setting.
        session = unsafe {
            WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                null(),
                null(),
                0,
            )
        };
    }
    let session = handle(session, "start")?;
    unsafe {
        // Name lookup, connect, send, receive: no step waits longer.
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 30_000, 30_000);
    }
    // TLS 1.2 and newer; a Windows that does not know 1.3 takes 1.2 alone,
    // and one that refuses both keeps its own default.
    if !set_option(
        &session,
        WINHTTP_OPTION_SECURE_PROTOCOLS,
        WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3,
    ) {
        set_option(
            &session,
            WINHTTP_OPTION_SECURE_PROTOCOLS,
            WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2,
        );
    }
    let connection = handle(
        unsafe { WinHttpConnect(session.0, wide(&url.host).as_ptr(), url.port, 0) },
        "connect",
    )?;
    let mut full_path = String::from(&*url.prefix);
    full_path.push_str(path);
    let flags = WINHTTP_FLAG_BYPASS_PROXY_CACHE | if url.secure { WINHTTP_FLAG_SECURE } else { 0 };
    let request = handle(
        unsafe {
            WinHttpOpenRequest(
                connection.0,
                wide("GET").as_ptr(),
                wide(&full_path).as_ptr(),
                null(),
                null(),
                null(),
                flags,
            )
        },
        "request",
    )?;
    // The answer is this server's own or nothing: no redirect is followed,
    // and no cookie is kept or sent.
    if !set_option(
        &request,
        WINHTTP_OPTION_REDIRECT_POLICY,
        WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
    ) {
        return Err(HttpError::Network("disable redirects", last_error()));
    }
    if !set_option(
        &request,
        WINHTTP_OPTION_DISABLE_FEATURE,
        WINHTTP_DISABLE_COOKIES,
    ) {
        return Err(HttpError::Network("disable cookies", last_error()));
    }
    if unsafe { WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0) } == 0 {
        return Err(HttpError::Network("send", last_error()));
    }
    if unsafe { WinHttpReceiveResponse(request.0, null_mut()) } == 0 {
        return Err(HttpError::Network("receive", last_error()));
    }
    let status = number_header(&request, WINHTTP_QUERY_STATUS_CODE)
        .ok_or_else(|| HttpError::Network("status", last_error()))?;
    if status != 200 {
        return Err(HttpError::Status(status));
    }
    let expected = number_header(&request, WINHTTP_QUERY_CONTENT_LENGTH).map(u64::from);
    if expected.is_some_and(|expected| expected > limit) {
        return Err(HttpError::TooLarge);
    }
    let mut received = 0_u64;
    let mut buffer = alloc::vec![0_u8; 64 * 1024];
    loop {
        if !progress(received, expected) {
            return Err(HttpError::Stopped);
        }
        let mut read = 0_u32;
        let ok = unsafe {
            WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast::<c_void>(),
                buffer.len() as u32,
                &mut read,
            )
        };
        if ok == 0 {
            return Err(HttpError::Network("read", last_error()));
        }
        if read == 0 {
            break;
        }
        if received + u64::from(read) > limit {
            return Err(HttpError::TooLarge);
        }
        sink(&buffer[..read as usize])?;
        received += u64::from(read);
    }
    if let Some(expected) = expected
        && received != expected
    {
        return Err(HttpError::Cut {
            got: received,
            expected,
        });
    }
    if !progress(received, expected) {
        return Err(HttpError::Stopped);
    }
    Ok(())
}
