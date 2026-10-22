use crate::api::PluginApi;
use std::ffi::c_char;

pub unsafe fn log(api: *const PluginApi, isError: bool, text: *const c_char) -> bool {
    let Some(api) = api.as_ref() else { return false };
    if isError {
        let Some(logFn) = api.log_error else { return false };
        logFn(b"lazy.manda\0".as_ptr().cast(), text);
    }
    else {
        let Some(logFn) = api.log_info else { return false };
        logFn(b"lazy.manda\0".as_ptr().cast(), text);
    }
    true
}