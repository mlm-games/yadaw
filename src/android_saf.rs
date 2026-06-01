#![cfg(target_os = "android")]
use anyhow::{Result, anyhow};
use jni::objects::JObject;
use jni::{JNIEnv, JavaVM};
use std::path::PathBuf;

pub fn with_env<F, R>(f: F) -> Result<R>
where
    F: for<'a> FnOnce(&mut JNIEnv<'a>, JObject<'a>) -> Result<R>,
{
    let ctx = ndk_context::android_context();
    let vm =
        unsafe { JavaVM::from_raw(ctx.vm().cast()) }.map_err(|_| anyhow!("VM not available"))?;
    let mut env = vm.attach_current_thread()?;
    let raw_context = unsafe { JObject::from_raw(ctx.context().cast()) };
    let context = env
        .new_local_ref(&raw_context)
        .map_err(|e| anyhow!("Failed to create local ref: {e}"))?;
    std::mem::forget(raw_context);
    f(&mut env, context)
}

pub fn files_dir_path() -> Result<PathBuf> {
    with_env(|env, context| {
        let file_obj = env
            .call_method(&context, "getFilesDir", "()Ljava/io/File;", &[])?
            .l()?;
        let jpath = env
            .call_method(&file_obj, "getAbsolutePath", "()Ljava/lang/String;", &[])?
            .l()?;
        let s: String = env.get_string(&jni::objects::JString::from(jpath))?.into();
        Ok(PathBuf::from(s))
    })
}
