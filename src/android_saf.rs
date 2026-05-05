#![cfg(target_os = "android")]
use anyhow::{anyhow, Result};
use jni::errors::Error as JniError;
use jni::objects::{JByteArray, JClass, JObject, JString, JValue};
use jni::sys::jint;
use jni::{JNIEnv, JavaVM};
use std::io::Write;
use std::path::PathBuf;

const FLAG_GRANT_READ_URI_PERMISSION: jint = 1;
const FLAG_GRANT_WRITE_URI_PERMISSION: jint = 2;

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

fn java_exception_detail(env: &mut JNIEnv<'_>) -> String {
    let throwable = match env.exception_occurred() {
        Ok(value) => value,
        Err(err) => {
            return format!("Java exception (failed to fetch throwable: {err})");
        }
    };

    if let Err(err) = env.exception_clear() {
        return format!("Java exception (failed to clear pending exception: {err})");
    }

    if throwable.is_null() {
        return "Java exception (no throwable object)".to_string();
    }

    let throwable_obj: JObject<'_> = throwable.into();
    let rendered = match env.call_method(&throwable_obj, "toString", "()Ljava/lang/String;", &[]) {
        Ok(v) => v,
        Err(err) => {
            return format!("Java exception (failed to render throwable: {err})");
        }
    };

    let rendered_obj = match rendered.l() {
        Ok(obj) => obj,
        Err(err) => {
            return format!("Java exception (throwable toString returned invalid value: {err})");
        }
    };

    let rendered_jstr = JString::from(rendered_obj);
    match env.get_string(&rendered_jstr) {
        Ok(value) => value.to_string_lossy().into_owned(),
        Err(err) => format!("Java exception (failed to decode throwable string: {err})"),
    }
}

fn map_jni_error(env: &mut JNIEnv<'_>, stage: &str, error: JniError) -> anyhow::Error {
    match error {
        JniError::JavaException => anyhow!("{stage}: {}", java_exception_detail(env)),
        other => anyhow!("{stage}: {other}"),
    }
}

fn best_effort_take_persistable_uri_permission(
    env: &mut JNIEnv<'_>,
    resolver: &JObject<'_>,
    uri: &JObject<'_>,
    flags: jint,
    content_uri: &str,
) {
    let grant_result = env.call_method(
        resolver,
        "takePersistableUriPermission",
        "(Landroid/net/Uri;I)V",
        &[JValue::Object(uri), JValue::Int(flags)],
    );

    match grant_result {
        Ok(_) => {
            log::info!(
                "yadaw: took persistable URI permission for {} with flags=0x{:x}",
                content_uri,
                flags
            );
        }
        Err(JniError::JavaException) => {
            let detail = java_exception_detail(env);
            log::warn!(
                "yadaw: persistable URI permission unavailable for {}: {}",
                content_uri,
                detail
            );
        }
        Err(other) => {
            log::warn!(
                "yadaw: failed to request persistable URI permission for {}: {}",
                content_uri,
                other
            );
        }
    }
}

fn best_effort_grant_self_uri_permission(
    env: &mut JNIEnv<'_>,
    context: &JObject<'_>,
    uri: &JObject<'_>,
    flags: jint,
    content_uri: &str,
) {
    let mut rw_flags = flags & (FLAG_GRANT_READ_URI_PERMISSION | FLAG_GRANT_WRITE_URI_PERMISSION);
    if rw_flags == 0 {
        rw_flags = FLAG_GRANT_READ_URI_PERMISSION;
    }

    let package_name_obj =
        match env.call_method(context, "getPackageName", "()Ljava/lang/String;", &[]) {
            Ok(v) => match v.l() {
                Ok(obj) if !obj.is_null() => obj,
                _ => return,
            },
            _ => return,
        };

    let _ = env.call_method(
        context,
        "grantUriPermission",
        "(Ljava/lang/String;Landroid/net/Uri;I)V",
        &[
            JValue::Object(&package_name_obj),
            JValue::Object(uri),
            JValue::Int(rw_flags),
        ],
    );

    log::info!(
        "yadaw: attempted grantUriPermission for {} flags=0x{:x}",
        content_uri,
        rw_flags
    );
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
        .map_err(|e| anyhow!("Failed to create local ref for Android context: {e}"))?;
    std::mem::forget(raw_context);
    f(&mut env, context)
}

pub fn guess_extension_for_content_uri(content_uri: &str) -> Option<String> {
    let guessed = with_env(|env, context| {
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
            .map_err(|e| map_jni_error(env, "find_class(android/net/Uri)", e))?;
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
            .map_err(|e| anyhow!("Uri.parse returned invalid object: {e}"))?;

        let mime_obj = env
            .call_method(
                &resolver,
                "getType",
                "(Landroid/net/Uri;)Ljava/lang/String;",
                &[JValue::Object(&juri)],
            )
            .map_err(|e| map_jni_error(env, "ContentResolver.getType", e))?
            .l()
            .map_err(|e| anyhow!("ContentResolver.getType returned invalid object: {e}"))?;

        if mime_obj.is_null() {
            return Ok(None);
        }

        let mime = env
            .get_string(&JString::from(mime_obj))
            .map_err(|e| anyhow!("Failed to decode MIME type string: {e}"))?
            .to_string_lossy()
            .to_lowercase();

        Ok(extension_from_mime(&mime).map(str::to_owned))
    });

    match guessed {
        Ok(value) => value,
        Err(err) => {
            log::warn!(
                "yadaw: failed to guess extension for URI {}: {}",
                content_uri,
                err
            );
            None
        }
    }
}

pub fn copy_from_content_uri_to_internal(
    content_uri: &str,
    dest_rel_name: &str,
) -> Result<PathBuf> {
    log::info!(
        "yadaw: copy_from_content_uri_to_internal uri={} dest_rel_name={}",
        content_uri,
        dest_rel_name
    );
    let dest = crate::paths::projects_dir().join(dest_rel_name);

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
            .map_err(|e| map_jni_error(env, "find_class(android/net/Uri)", e))?;
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
            .map_err(|e| anyhow!("Uri.parse returned invalid object: {e}"))?;

        best_effort_grant_self_uri_permission(
            env,
            &context,
            &juri,
            FLAG_GRANT_READ_URI_PERMISSION,
            content_uri,
        );

        best_effort_take_persistable_uri_permission(
            env,
            &resolver,
            &juri,
            FLAG_GRANT_READ_URI_PERMISSION,
            content_uri,
        );

        let in_stream = env
            .call_method(
                &resolver,
                "openInputStream",
                "(Landroid/net/Uri;)Ljava/io/InputStream;",
                &[JValue::Object(&juri)],
            )
            .map_err(|e| {
                map_jni_error(
                    env,
                    &format!("ContentResolver.openInputStream('{content_uri}')"),
                    e,
                )
            })?
            .l()
            .map_err(|e| anyhow!("openInputStream returned invalid object: {e}"))?;

        if in_stream.is_null() {
            return Err(anyhow!(
                "openInputStream for '{}' returned null. Check URI and permissions.",
                content_uri
            ));
        }

        let mut out = std::fs::File::create(&dest)?;
        let jbuf: JByteArray = env.new_byte_array(64 * 1024)?;

        loop {
            let read_bytes = env
                .call_method(&in_stream, "read", "([B)I", &[JValue::Object(&jbuf)])
                .map_err(|e| map_jni_error(env, "InputStream.read", e))?
                .i()
                .map_err(|e| anyhow!("InputStream.read returned invalid value: {e}"))?;

            if read_bytes == -1 {
                break;
            }
            if read_bytes == 0 {
                continue;
            }

            let mut chunk_i8 = vec![0i8; read_bytes as usize];
            env.get_byte_array_region(&jbuf, 0, &mut chunk_i8)
                .map_err(|e| map_jni_error(env, "get_byte_array_region", e))?;

            let chunk_u8: &[u8] = unsafe {
                std::slice::from_raw_parts(chunk_i8.as_ptr() as *const u8, read_bytes as usize)
            };
            out.write_all(chunk_u8)?;
        }

        let _ = env
            .call_method(&in_stream, "close", "()V", &[])
            .map_err(|e| map_jni_error(env, "InputStream.close", e))?;
        log::info!(
            "yadaw: copy_from_content_uri_to_internal complete uri={} dest={}",
            content_uri,
            dest.display()
        );
        Ok(())
    })?;

    Ok(dest)
}
