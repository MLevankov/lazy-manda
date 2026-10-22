use crate::{api::{PluginApi, PluginMetadata}, jvm::{findClass, getJNIEnv, loadClassViaLoaders}, logger::log, trigger::Trigger};
use jni::{JNIEnv, objects::{JObject, JValue}};
use libloading::Library;
use serde::Deserialize;
use walkdir::WalkDir;
use std::{collections::HashMap, ffi::CString, fs, path::{Path, PathBuf}, sync::{Mutex, MutexGuard, OnceLock, atomic::{AtomicPtr, Ordering}}};

macro_rules! logf {
    ($api:expr, $err:expr, $($arg:tt)*) => {{
        let c = CString::new(format!($($arg)*)).unwrap_or_default();
        unsafe { log($api, $err, c.as_ptr()) };
    }};
}

static API: AtomicPtr<PluginApi> = AtomicPtr::new(std::ptr::null_mut());

#[derive(Debug, Clone, Deserialize, Default)]
struct Lazy {
    enabled: bool,
    #[serde(default)]
    priority: i32,
    #[serde(default)]
    triggers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Libs {
    unix: String,
    windows: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PluginConfig {
    enabled: bool,
    libs: Libs,
    #[serde(default)]
    lazy: Lazy,
}

pub fn parseConfig(configPath: &str) -> Result<PluginConfig, String> {
    let raw = fs::read_to_string(configPath).map_err(|err| format!("{}", err))?;
    let config = toml::from_str(&raw).map_err(|err| format!("{}", err));
    config
}


#[derive(Clone, Copy, PartialEq, Eq)]
enum State { Pending, Loading, Loaded, Failed }

struct Entry { libPath: PathBuf, priority: i32, state: State }

static REGISTRY: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
static TRIGGERS: OnceLock<Mutex<HashMap<Trigger, Vec<String>>>> = OnceLock::new();

fn registry() -> MutexGuard<'static, HashMap<String, Entry>> {
    REGISTRY
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn triggers() -> MutexGuard<'static, HashMap<Trigger, Vec<String>>> {
    TRIGGERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn pluginLoaderInstance<'l>(env: &mut JNIEnv<'l>) -> Result<JObject<'l>, String> {
    let cls: JObject = match findClass(env, "com/mandagram/plugin/loader/PluginLoader") {
        Some(c) => c.into(),
        None => {
            let _ = env.exception_clear();
            loadClassViaLoaders(env, "com.mandagram.plugin.loader.PluginLoader").map_err(|e| {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
                format!("class lookup failed: {e}")
            })?
        }
    };
    let cls = jni::objects::JClass::from(cls);

    match env
        .get_static_field(&cls, "INSTANCE", "Lcom/mandagram/plugin/loader/PluginLoader;")
        .and_then(|v| v.l())
    {
        Ok(o) if !o.is_null() => Ok(o),
        Ok(_) => Err("INSTANCE is null".into()),
        Err(e) => {
            let _ = env.exception_describe();
            let _ = env.exception_clear();
            Err(format!("INSTANCE field: {e}"))
        }
    }
}

pub fn registerPlugin(api: *const PluginApi, path: PathBuf) -> bool {
    let cfg = match parseConfig(&path.join("config.toml").to_string_lossy()) {
        Ok(c) => c,
        Err(e) => { logf!(api, true, "{e}"); return false }
    };
    if !cfg.enabled { logf!(api, false, "disabled: {path:?}"); return false; }

    let libPath = path.join(if cfg!(unix) { &cfg.libs.unix } else { &cfg.libs.windows });
    if !libPath.is_file() { logf!(api, true, "not a file: {libPath:?}"); return false; }
    let Some(name) = libPath.file_stem().map(|s| s.to_string_lossy().into_owned()) else { return false };

    if cfg.lazy.enabled {
        logf!(api, false, "deferred: {name} (priority {})", cfg.lazy.priority);
        for raw in &cfg.lazy.triggers {
            match Trigger::parse(raw) {
                Ok(t) => {
                    let mut tr = triggers();
                    let list = tr.entry(t).or_default();
                    if !list.contains(&name) { list.push(name.clone()); }
                }
                Err(e) => logf!(api, true, "bad trigger: {e}"),
            }
        }
        registry().insert(name, Entry { libPath, priority: cfg.lazy.priority, state: State::Pending });
        return true;
    }

    let ok = unsafe { loadPlugin(api, &libPath) };
    registry().insert(name, Entry {
        libPath, priority: cfg.lazy.priority,
        state: if ok { State::Loaded } else { State::Failed },
    });
    ok
}

pub unsafe fn loadPlugin(api: *const PluginApi, path: &Path) -> bool {
    let pathString = &path.to_string_lossy();
    let loadingMsg = CString::new(format!("loading plugin by path: {pathString}")).unwrap();
    log(api, false, loadingMsg.as_ptr());
    let Some(mut jniEnv) = getJNIEnv() else { return false };
    let loader = match pluginLoaderInstance(&mut jniEnv) {
        Ok(ok) => ok,
        Err(err) => {
            logf!(api, true, "{}", err);
            return false;
        },
    };
    let Ok(jPath) = jniEnv.new_string(path.to_string_lossy().as_ref()) else { return false };
    let file = match jniEnv.new_object("java/io/File", "(Ljava/lang/String;)V", &[JValue::Object(&jPath)]) {
        Ok(f) => f,
        Err(_) => {
            _ = jniEnv.exception_clear();
            return false;
        },
    };
    match jniEnv.call_method(&loader, "loadPlugin", "(Ljava/io/File;)Z", &[JValue::Object(&file)]).and_then(|v| v.z()) {
        Ok(ok) => {
            let text = CString::new(ok.to_string()).unwrap();
            log(api, !ok, text.as_ptr());
        },
        Err(err) => {
            let text = CString::new(err.to_string()).unwrap();
            log(api, true, text.as_ptr());
            _ = jniEnv.exception_describe();
            _ = jniEnv.exception_clear();
        },
    }
    true
}

pub fn ensureLoaded(api: *const PluginApi, name: &str) -> bool {
    let libPath = {
        let mut reg = registry();
        let Some(e) = reg.get_mut(name) else { return false };
        match e.state {
            State::Loaded => return true,
            State::Loading | State::Failed => return false,
            State::Pending => { e.state = State::Loading; e.libPath.clone() }
        }
    };

    let ok = unsafe { loadPlugin(api, &libPath) };
    if let Some(e) = registry().get_mut(name) {
        e.state = if ok { State::Loaded } else { State::Failed };
    }
    ok
}

pub fn loadDeferred(api: *const PluginApi) {
    let mut pending: Vec<(i32, String)> = registry()
        .iter()
        .filter(|(_, e)| e.state == State::Pending)
        .map(|(n, e)| (e.priority, n.clone()))
        .collect();
    pending.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, name) in pending {
        ensureLoaded(api, &name);
    }
}

pub fn onTrigger(api: *const PluginApi, trigger: &Trigger) {
    let names = {
        let t = triggers();
        match t.get(trigger) {
            Some(n) => n.clone(),
            None => return,
        }
    };
    for name in names { ensureLoaded(api, &name); }
    triggers().remove(trigger);
}

pub unsafe fn getPluginsDir() -> Result<PathBuf, String> {
    let Some(mut jniEnv) = getJNIEnv() else { return Err("unable to get jni env".into()) };
    let loader = pluginLoaderInstance(&mut jniEnv)?;

    let fail = |env: &mut JNIEnv, what: &str, e: jni::errors::Error| {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
        format!("{what}: {e}")
    };

    let file = match jniEnv
        .call_method(&loader, "getPluginsDir", "()Ljava/io/File;", &[])
        .and_then(|v| v.l())
    {
        Ok(f) if !f.is_null() => f,
        Ok(_) => return Err("getPluginsDir() returned null".into()),
        Err(e) => return Err(fail(&mut jniEnv, "getPluginsDir", e)),
    };

    let jstr = match jniEnv
        .call_method(&file, "getAbsolutePath", "()Ljava/lang/String;", &[])
        .and_then(|v| v.l())
    {
        Ok(s) if !s.is_null() => jni::objects::JString::from(s),
        Ok(_) => return Err("getAbsolutePath() returned null".into()),
        Err(e) => return Err(fail(&mut jniEnv, "getAbsolutePath", e)),
    };

    let s: String = jniEnv
        .get_string(&jstr)
        .map_err(|e| format!("get_string: {e}"))?
        .into();
    Ok(PathBuf::from(s))
}

pub unsafe fn getLazyPlugins() -> Result<Vec<PathBuf>, String> {
    let pluginsDir = getPluginsDir()?;
    let pluginDirs: Vec<PathBuf> = WalkDir::new(pluginsDir.join("lazy"))
        .min_depth(1)
        .max_depth(8)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_dir())
        .map(|e| e.into_path())
        .collect();
    Ok(pluginDirs)
}