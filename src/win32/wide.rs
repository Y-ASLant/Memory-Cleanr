//! Null-terminated UTF-16 string helpers for Win32 APIs.

pub(crate) fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}
