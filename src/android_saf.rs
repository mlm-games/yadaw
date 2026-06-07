#![cfg(target_os = "android")]

use jni::{
    objects::JObject,
    signature::RuntimeMethodSignature,
    strings::JNIString,
    Env, JavaVM,
};
use std::path::PathBuf;

pub fn with_env<F, R>(f: F) -> Result<R, jni::errors::Error>
where
    F: FnOnce(&mut Env, &JObject) -> Result<R, jni::errors::Error>,
{
    let ctx = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) };
    let raw_context = ctx.context().cast::<jni::sys::_jobject>();

    vm.attach_current_thread(|env| {
        let context = unsafe { JObject::from_raw(env, raw_context) };
        f(env, &context)
    })
}

pub fn files_dir_path() -> Result<PathBuf, jni::errors::Error> {
    with_env(|env, context| {
        let file_obj = env
            .call_method(
                context,
                JNIString::from("getFilesDir"),
                RuntimeMethodSignature::from_str("()Ljava/io/File;")?.method_signature(),
                &[],
            )?
            .l()?;
        let jpath = env
            .call_method(
                &file_obj,
                JNIString::from("getAbsolutePath"),
                RuntimeMethodSignature::from_str("()Ljava/lang/String;")?.method_signature(),
                &[],
            )?
            .l()?;
        let jstr = env.cast_local::<jni::objects::JString>(jpath)?;
        let s = jstr.try_to_string(&*env)?;
        Ok(PathBuf::from(s))
    })
}
