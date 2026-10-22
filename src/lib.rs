pub mod metadata;
pub mod api;
pub mod plugin;
pub mod logger;
pub mod loader;
pub mod jvm;
pub mod trigger;

use metadata::METADATA;
use api::{PluginApi, PluginMetadata};

use crate::plugin::pluginInit;

#[unsafe(no_mangle)]
pub extern "C" fn mandagram_plugin_metadata() -> *const PluginMetadata { &METADATA }

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mandagram_plugin_init(api: *const PluginApi) -> bool {
    pluginInit(api)
}