use std::{ptr, sync::Mutex};
use jni::{JNIEnv, JavaVM, errors::{JniError, Result as JResult}, objects::{GlobalRef, JClass, JString, JObject, JValue}, sys::{JNI_EDETACHED, JavaVM as SysJavaVM, jint, jsize, jstring}};
use once_cell::sync::OnceCell;

pub static G_CLASS_LOADER: Mutex<Option<GlobalRef>> = Mutex::new(None);

type GetCreatedJavaVMsFn = unsafe extern "system" fn(vmBuffer: *mut *mut SysJavaVM, bufferLength: jsize, vmsCount: *mut jsize) -> jint;

static GLOBAL_JVM: OnceCell<JavaVM> = OnceCell::new();

#[cfg(unix)]
pub unsafe fn getJVMPtr() -> *mut SysJavaVM {
    use libc::{dlsym, dlopen, RTLD_LAZY, RTLD_NOLOAD};

    let mut handle = dlopen(b"libjvm.so\0".as_ptr().cast(), RTLD_NOLOAD);
    if handle.is_null() { handle = dlopen(b"libjvm.so\0".as_ptr().cast(), RTLD_LAZY); }
    if handle.is_null() { return ptr::null_mut(); }

    let procAddr = dlsym(handle, b"JNI_GetCreatedJavaVMs\0".as_ptr().cast());
    if procAddr.is_null() { return ptr::null_mut(); }

    let getVMs: GetCreatedJavaVMsFn = std::mem::transmute(procAddr);
    let mut VMs: [*mut SysJavaVM; 2] = [ptr::null_mut(); 2];
    let mut count: jsize = 0;

    if getVMs(VMs.as_mut_ptr(), 2, &mut count) == 0 && count > 0 { return VMs[0]; }
    ptr::null_mut()
}

#[cfg(windows)]
pub unsafe fn getJVMPtr() -> *mut SysJavaVM {
    use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress, LoadLibraryA};

    let mut handle = GetModuleHandleA(b"jvm.dll\0".as_ptr());
    if handle == 0 { handle = LoadLibraryA(b"jvm.dll\0".as_ptr()); }
    if handle == 0 { return ptr::null_mut(); }

    let Some(procAddr) = GetProcAddress(handle, b"JNI_GetCreatedJavaVMs\0".as_ptr()) else {
        return ptr::null_mut();
    };

    let getVMs: GetCreatedJavaVMsFn = std::mem::transmute(procAddr);
    let mut VMs: [*mut SysJavaVM; 2] = [ptr::null_mut(); 2]; // было *mut JavaVM
    let mut count: jsize = 0;

    if getVMs(VMs.as_mut_ptr(), 2, &mut count) == 0 && count > 0 { return VMs[0]; }
    ptr::null_mut()
}

pub unsafe fn getJNIEnv<'a>() -> Option<JNIEnv<'a>> {
    let jvm = GLOBAL_JVM.get_or_init(|| unsafe {
        let jvmPtr = getJVMPtr();
        if jvmPtr.is_null() { panic!("unable to get a jvm ptr"); }
        JavaVM::from_raw(jvmPtr.cast()).unwrap()
    });

    match jvm.get_env() {
        Ok(env) => Some(env),
        Err(jni::errors::Error::JniCall(JniError::ThreadDetached)) => {
            let envRef = jvm.attach_current_thread_as_daemon().ok()?;
            Some(std::mem::transmute::<JNIEnv<'_>, JNIEnv<'a>>(envRef))
        },
        _ => None,
    }
}

pub fn findClass<'local>(env: &mut JNIEnv<'local>, name: &str) -> Option<JClass<'local>> {
    if let Ok(class) = env.find_class(name) {
        return Some(class);
    }

    let _ = env.exception_clear();
    let guard = G_CLASS_LOADER.lock().ok()?;
    let loader = guard.as_ref()?;

    let jname: JString = env.new_string(name.replace('/', ".")).ok()?;

    let class: JObject = env
        .call_method(
            loader,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
            &[JValue::Object(&jname)],
        )
        .ok()?
        .l()
        .ok()?;

    if class.is_null() || env.exception_check().unwrap_or(true) {
        let _ = env.exception_clear();
        return None;
    }

    Some(class.into())
}

pub fn loadClassViaLoaders<'l>(env: &mut JNIEnv<'l>, dotted: &str) -> JResult<JObject<'l>> {
    let jName = env.new_string(dotted)?;
    let thread = env
        .call_static_method("java/lang/Thread", "currentThread", "()Ljava/lang/Thread;", &[])?
        .l()?;
    let mut loader = env
        .call_method(&thread, "getContextClassLoader", "()Ljava/lang/ClassLoader;", &[])?
        .l()?;
    if loader.is_null() {
        loader = env
            .call_static_method("java/lang/ClassLoader", "getSystemClassLoader", "()Ljava/lang/ClassLoader;", &[])?
            .l()?;
    }
    env.call_static_method(
        "java/lang/Class",
        "forName",
        "(Ljava/lang/String;ZLjava/lang/ClassLoader;)Ljava/lang/Class;",
        &[JValue::Object(&jName), JValue::Bool(1), JValue::Object(&loader)],
    )?
    .l()
}