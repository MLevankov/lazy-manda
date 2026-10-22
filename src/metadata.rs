use crate::api::PluginMetadata;

pub static METADATA: PluginMetadata = PluginMetadata {
    name: b"lazy.manda\0".as_ptr().cast(),
    author: b"@nyanplugins\0".as_ptr().cast(),
    version: b"1.0.0\0".as_ptr().cast(),
    description: b"Lazy plugins loader\0".as_ptr().cast(),
};