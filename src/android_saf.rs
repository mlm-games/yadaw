#![cfg(target_os = "android")]
use anyhow::{Result, anyhow};
use jni::objects::{JByteArray, JClass, JObject, JString, JValue};
use jni::sys::jint;
use jni::{JNIEnv, JavaVM};
use std::path::PathBuf;

fn extension_from_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "audio/wav" | "audio/x-wav" | "audio/wave" | "audio/vnd.wave" => Some("wav"),
        "audio/mpeg" | "audio/mp3" => Some("mp3"),
        "audio/flac" | "audio/x-flac" => Some("flac"),
        "audio/ogg" => Some("ogg"),
        "audio/mp4" | "audio/x-m4a" => Some("m4a"),
        "audio/aac" | "audio/aacp" => Some("aac"),
        "audio/midi" | "audio/x-midi" | "audio/sp-midi" => Some("mid"),
        _ => None,
    }
}

fn map_jni_error(env: &mut JNIEnv<'_>, stage: &str, error: jni::errors::Error) -> anyhow::Error {
    match error {
        jni::errors::Error::JavaException => {
            let throwable = match env.exception_occurred() {
                Ok(value) => value,
                Err(err) => return anyhow!("{stage}: failed to fetch throwable: {err}"),
            };
            if let Err(err) = env.exception_clear() {
                return anyhow!("{stage}: failed to clear pending exception: {err}");
            }
            if throwable.is_null() {
                return anyhow!("{stage}: no throwable object");
            }
            let throwable_obj: JObject<'_> = throwable.into();
            match env.call_method(&throwable_obj, "toString", "()Ljava/lang/String;", &[]) {
                Ok(v) => match v.l() {
                    Ok(obj) if !obj.is_null() => {
                        let rendered_jstr = JString::from(obj);
                        match env.get_string(&rendered_jstr) {
                            Ok(value) => anyhow!("{stage}: {}", value.to_string_lossy()),
                            Err(err) => {
                                anyhow!("{stage}: failed to decode throwable string: {err}")
                            }
                        }
                    }
                    Ok(_) => anyhow!("{stage}: toString returned null"),
                    Err(err) => anyhow!("{stage}: toString returned invalid value: {err}"),
                },
                Err(err) => anyhow!("{stage}: failed to render throwable: {err}"),
            }
        }
        other => anyhow!("{stage}: {other}"),
    }
}

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

pub fn guess_extension_for_content_uri(content_uri: &str) -> Option<String> {
    with_env(|env, context| {
        let resolver = env
            .call_method(
                &context,
                "getContentResolver",
                "()Landroid/content/ContentResolver;",
                &[],
            )?
            .l()?;

        let juri_class: JClass = env
            .find_class("android/net/Uri")
            .map_err(|e| map_jni_error(env, "find_class(Uri)", e))?;
        let jstr: JString = env.new_string(content_uri)?;
        let juri = env
            .call_static_method(
                juri_class,
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&jstr.into())],
            )
            .map_err(|e| map_jni_error(env, "Uri.parse", e))?
            .l()
            .map_err(|e| anyhow!("Uri.parse returned invalid: {e}"))?;

        let mime_obj = env
            .call_method(
                &resolver,
                "getType",
                "(Landroid/net/Uri;)Ljava/lang/String;",
                &[JValue::Object(&juri)],
            )
            .map_err(|e| map_jni_error(env, "ContentResolver.getType", e))?
            .l()
            .map_err(|e| anyhow!("getType returned invalid: {e}"))?;

        if mime_obj.is_null() {
            return Ok(None);
        }

        let mime = env
            .get_string(&JString::from(mime_obj))
            .map_err(|e| anyhow!("Failed to decode MIME: {e}"))?
            .to_string_lossy()
            .to_lowercase();
        Ok(extension_from_mime(&mime).map(str::to_owned))
    })
    .ok()
    .flatten()
}
