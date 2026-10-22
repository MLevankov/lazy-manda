use std::{ffi::{CStr, CString, c_char}, path::PathBuf, sync::atomic::{AtomicPtr, Ordering}};

use jni::{JNIEnv, objects::{JObject, JValue}};

use crate::{api::{HookParam, PluginApi}, jvm::{findClass, getJNIEnv, loadClassViaLoaders}, loader::{getLazyPlugins, loadPlugin, onTrigger, registerPlugin}, logger::log, trigger::Trigger};

static API: AtomicPtr<PluginApi> = AtomicPtr::new(std::ptr::null_mut());

pub unsafe extern "C" fn onSendMessageHook(_: i64, text: *const c_char, _: usize) -> *const c_char {
    let bytes = CStr::from_ptr(text).to_bytes();
    let messageText = String::from_utf8_lossy(bytes);
    let command = messageText.split_whitespace().next().unwrap();
    onTrigger(API.load(Ordering::Acquire), &Trigger::Command(command.to_string()));
    text
}

#[unsafe(no_mangle)]
pub unsafe fn pluginInit(apiPtr: *const PluginApi) -> bool {
    let Some(api) = apiPtr.as_ref() else { return false };
    API.store(apiPtr as *mut PluginApi, Ordering::Release);
    let Some(register) = api.register_message_send_hook else { return false };
    log(api, false, b"lazy.manda is loaded\0".as_ptr().cast());
    let lazyPlugins = match getLazyPlugins() {
        Ok(ok) => ok,
        Err(err) => {
            let text = CString::new(err).unwrap();
            log(api, true, text.as_ptr().cast());
            return false;
        },
    };
    for plugin in lazyPlugins { registerPlugin(api, plugin); }
    register(Some(onSendMessageHook), None, 0) == 0
}