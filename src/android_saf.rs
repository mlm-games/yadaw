#![cfg(target_os = "android")]

use jni::{Env, JavaVM, objects::JObject, signature::RuntimeMethodSignature, strings::JNIString};
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

pub static EGUI_CTX: std::sync::OnceLock<egui::Context> = std::sync::OnceLock::new();

pub fn take_pending_shared_files() -> Vec<PathBuf> {
    let Ok(dir) = files_dir_path() else {
        return Vec::new();
    };
    let incoming = dir.join("incoming");
    let mut staged = Vec::new();
    for intent in rlobkit_app_events::intents::drain_intents_from(&dir) {
        for file in &intent.files {
            let Ok(bytes) = file.read_bytes() else {
                log::warn!("cannot read shared file {}", file.name());
                continue;
            };
            let name = std::path::Path::new(file.name())
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| String::from("shared"));
            if std::fs::create_dir_all(&incoming).is_err() {
                continue;
            }
            let mut path = incoming.join(&name);
            let mut n = 1;
            while path.exists() {
                path = incoming.join(format!("{n}_{name}"));
                n += 1;
            }
            if std::fs::write(&path, &bytes).is_ok() {
                log::info!("staged shared file {}", path.display());
                staged.push(path);
            }
        }
    }
    staged
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
