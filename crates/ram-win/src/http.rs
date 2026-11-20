//! Synchronous HTTPS client on WinHTTP.

use std::ffi::c_void;

use ram_core::platform::{HttpApi, HttpResponse, NetError};
use windows::core::{Error, PCWSTR};
use windows::Win32::Networking::WinHttp::{
    WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl, WinHttpOpen,
    WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts, ERROR_WINHTTP_CANNOT_CONNECT,
    ERROR_WINHTTP_CONNECTION_ERROR, ERROR_WINHTTP_NAME_NOT_RESOLVED, ERROR_WINHTTP_TIMEOUT,
    URL_COMPONENTS, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD,
    WINHTTP_ADDREQ_FLAG_REPLACE, WINHTTP_DISABLE_COOKIES, WINHTTP_DISABLE_REDIRECTS,
    WINHTTP_FLAG_SECURE, WINHTTP_INTERNET_SCHEME_HTTPS, WINHTTP_OPTION_DISABLE_FEATURE,
    WINHTTP_OPTION_REDIRECT_POLICY, WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP,
    WINHTTP_QUERY_CUSTOM, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

use crate::util::{code_of, from_wide, wide};

/// Largest response body accepted.
pub const MAX_BODY: usize = 1024 * 1024;

const USER_AGENT: &str = "Waypoint";

/// Owns a WinHTTP handle.
struct Internet(*mut c_void);

impl Drop for Internet {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// [`HttpApi`] over WinHTTP.
///
/// - `https://` only; TLS and certificate validation are the system defaults.
/// - Uses the system proxy configuration.
/// - WinHTTP's cookie jar is disabled: nothing is stored or replayed between
///   requests.
/// - Redirects are never followed to plain HTTP, and not followed at all for
///   requests that carry an `Authorization` header (the 3xx is returned), so
///   a token is only ever sent to the host the caller named.
/// - `timeout_ms` applies to each phase (resolve, connect, send, receive)
///   separately, so a request can take longer than that in total.
/// - A response body larger than [`MAX_BODY`] is an error.
pub struct WinHttp {
    session: Internet,
}

// WinHTTP session handles may be used from any thread, concurrently.
unsafe impl Send for WinHttp {}
unsafe impl Sync for WinHttp {}

impl Default for WinHttp {
    fn default() -> Self {
        Self::new()
    }
}

fn net_error(e: &Error, what: &str) -> NetError {
    match code_of(e) {
        ERROR_WINHTTP_TIMEOUT => NetError::Timeout,
        ERROR_WINHTTP_NAME_NOT_RESOLVED
        | ERROR_WINHTTP_CANNOT_CONNECT
        | ERROR_WINHTTP_CONNECTION_ERROR => NetError::Offline,
        code => {
            let text = e.message();
            if text.is_empty() {
                NetError::Other(format!("{what} failed (error {code})"))
            } else {
                NetError::Other(format!("{what} failed: {} (error {code})", text.trim_end()))
            }
        }
    }
}

fn last_net_error(what: &str) -> NetError {
    net_error(&Error::from_thread(), what)
}

fn is_token(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

struct Target {
    host: Vec<u16>,
    port: u16,
    path: Vec<u16>,
}

fn crack(url: &str) -> Result<Target, NetError> {
    let https = url
        .get(..8)
        .is_some_and(|s| s.eq_ignore_ascii_case("https://"));
    if !https || url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(NetError::Other("only https:// URLs are supported".into()));
    }
    let url16: Vec<u16> = url.encode_utf16().collect();
    // Requesting lengths without buffers makes WinHTTP return pointers into
    // `url16` for each component.
    let mut parts = URL_COMPONENTS {
        dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
        dwHostNameLength: u32::MAX,
        dwUrlPathLength: u32::MAX,
        dwExtraInfoLength: u32::MAX,
        ..Default::default()
    };
    unsafe { WinHttpCrackUrl(&url16, 0, &mut parts) }.map_err(|e| net_error(&e, "parsing the URL"))?;
    if parts.nScheme != WINHTTP_INTERNET_SCHEME_HTTPS || parts.lpszHostName.is_null() {
        return Err(NetError::Other("only https:// URLs are supported".into()));
    }
    let slice = |ptr: windows::core::PWSTR, len: u32| -> Vec<u16> {
        if ptr.is_null() || len == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(ptr.0, len as usize) }.to_vec()
        }
    };
    let mut host = slice(parts.lpszHostName, parts.dwHostNameLength);
    if host.is_empty() {
        return Err(NetError::Other("URL has no host".into()));
    }
    host.push(0);
    let mut path = slice(parts.lpszUrlPath, parts.dwUrlPathLength);
    // The fragment is never sent to a server.
    let extra = slice(parts.lpszExtraInfo, parts.dwExtraInfoLength);
    let query_len = extra
        .iter()
        .position(|&c| c == b'#' as u16)
        .unwrap_or(extra.len());
    path.extend_from_slice(&extra[..query_len]);
    if path.is_empty() {
        path.push(b'/' as u16);
    }
    path.push(0);
    Ok(Target {
        host,
        port: parts.nPort,
        path,
    })
}

fn parse_retry_after(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

impl WinHttp {
    pub fn new() -> Self {
        let agent = wide(USER_AGENT);
        let session = unsafe {
            WinHttpOpen(
                PCWSTR(agent.as_ptr()),
                WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
                PCWSTR::null(),
                PCWSTR::null(),
                0,
            )
        };
        Self {
            session: Internet(session),
        }
    }
}

impl HttpApi for WinHttp {
    fn request(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
        timeout_ms: u32,
    ) -> Result<HttpResponse, NetError> {
        if self.session.0.is_null() {
            return Err(NetError::Other("WinHTTP is not available".into()));
        }
        if !is_token(method) {
            return Err(NetError::Other("invalid HTTP method".into()));
        }
        let target = crack(url)?;
        let body = body.unwrap_or(&[]);
        if body.len() > u32::MAX as usize {
            return Err(NetError::Other("request body is too large".into()));
        }

        let connection = Internet(unsafe {
            WinHttpConnect(self.session.0, PCWSTR(target.host.as_ptr()), target.port, 0)
        });
        if connection.0.is_null() {
            return Err(last_net_error("connecting"));
        }
        let verb = wide(&method.to_ascii_uppercase());
        let request = Internet(unsafe {
            WinHttpOpenRequest(
                connection.0,
                PCWSTR(verb.as_ptr()),
                PCWSTR(target.path.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                std::ptr::null(),
                WINHTTP_FLAG_SECURE,
            )
        });
        if request.0.is_null() {
            return Err(last_net_error("creating the request"));
        }

        let timeout = timeout_ms.clamp(1, i32::MAX as u32) as i32;
        unsafe { WinHttpSetTimeouts(request.0, timeout, timeout, timeout, timeout) }
            .map_err(|e| net_error(&e, "setting timeouts"))?;

        let mut disabled = WINHTTP_DISABLE_COOKIES;
        if headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        {
            disabled |= WINHTTP_DISABLE_REDIRECTS;
        }
        let set_option = |option: u32, value: u32| {
            unsafe { WinHttpSetOption(Some(request.0), option, Some(&value.to_ne_bytes())) }
                .map_err(|e| net_error(&e, "configuring the request"))
        };
        set_option(WINHTTP_OPTION_DISABLE_FEATURE, disabled)?;
        set_option(
            WINHTTP_OPTION_REDIRECT_POLICY,
            WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP,
        )?;

        for (name, value) in headers {
            if !is_token(name) || value.chars().any(|c| c == '\r' || c == '\n' || c == '\0') {
                return Err(NetError::Other(format!("invalid header {name:?}")));
            }
            let line: Vec<u16> = format!("{name}: {value}\r\n").encode_utf16().collect();
            unsafe {
                WinHttpAddRequestHeaders(
                    request.0,
                    &line,
                    WINHTTP_ADDREQ_FLAG_ADD | WINHTTP_ADDREQ_FLAG_REPLACE,
                )
            }
            .map_err(|e| net_error(&e, "adding a header"))?;
        }

        let optional = (!body.is_empty()).then_some(body.as_ptr() as *const c_void);
        unsafe {
            WinHttpSendRequest(request.0, None, optional, body.len() as u32, body.len() as u32, 0)
        }
        .map_err(|e| net_error(&e, "sending the request"))?;
        unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) }
            .map_err(|e| net_error(&e, "receiving the response"))?;

        let mut status = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        unsafe {
            WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                PCWSTR::null(),
                Some(&mut status as *mut u32 as *mut c_void),
                &mut size,
                std::ptr::null_mut(),
            )
        }
        .map_err(|e| net_error(&e, "reading the status"))?;

        let retry_after = {
            let name = wide("Retry-After");
            let mut buf = [0u16; 64];
            let mut size = (buf.len() * 2) as u32;
            unsafe {
                WinHttpQueryHeaders(
                    request.0,
                    WINHTTP_QUERY_CUSTOM,
                    PCWSTR(name.as_ptr()),
                    Some(buf.as_mut_ptr() as *mut c_void),
                    &mut size,
                    std::ptr::null_mut(),
                )
            }
            .ok()
            .and_then(|()| parse_retry_after(&from_wide(&buf)))
        };

        let mut response = Vec::new();
        let mut chunk = vec![0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            unsafe {
                WinHttpReadData(
                    request.0,
                    chunk.as_mut_ptr() as *mut c_void,
                    chunk.len() as u32,
                    &mut read,
                )
            }
            .map_err(|e| net_error(&e, "reading the response"))?;
            if read == 0 {
                break;
            }
            if response.len() + read as usize > MAX_BODY {
                return Err(NetError::Other("response is larger than 1 MiB".into()));
            }
            response.extend_from_slice(&chunk[..read as usize]);
        }

        Ok(HttpResponse {
            status: status as u16,
            retry_after,
            body: response,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(units: &[u16]) -> String {
        from_wide(units)
    }

    #[test]
    fn cracks_https_urls() {
        let t = crack("https://apis.roblox.com/oauth/v1/token?x=1&y=2#frag").unwrap();
        assert_eq!(text(&t.host), "apis.roblox.com");
        assert_eq!(t.port, 443);
        assert_eq!(text(&t.path), "/oauth/v1/token?x=1&y=2");

        let t = crack("https://example.com:8443").unwrap();
        assert_eq!(text(&t.host), "example.com");
        assert_eq!(t.port, 8443);
        assert_eq!(text(&t.path), "/");
    }

    #[test]
    fn rejects_everything_but_https() {
        let http = WinHttp::new();
        for url in ["http://example.com/", "ftp://example.com/", "example.com", "", "https://"] {
            assert!(
                matches!(http.request("GET", url, &[], None, 1000), Err(NetError::Other(_))),
                "{url:?}"
            );
        }
    }

    #[test]
    fn rejects_header_and_method_injection() {
        let http = WinHttp::new();
        let url = "https://example.invalid/";
        assert!(matches!(
            http.request("GET", url, &[("X-A", "b\r\nX-Evil: 1")], None, 1000),
            Err(NetError::Other(_))
        ));
        assert!(matches!(
            http.request("GET", url, &[("X A", "b")], None, 1000),
            Err(NetError::Other(_))
        ));
        assert!(matches!(
            http.request("GET /x HTTP/1.1\r\n", url, &[], None, 1000),
            Err(NetError::Other(_))
        ));
    }

    #[test]
    fn retry_after_accepts_integer_seconds_only() {
        assert_eq!(parse_retry_after("120"), Some(120));
        assert_eq!(parse_retry_after(" 5 "), Some(5));
        assert_eq!(parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT"), None);
        assert_eq!(parse_retry_after("-1"), None);
        assert_eq!(parse_retry_after(""), None);
    }
}
