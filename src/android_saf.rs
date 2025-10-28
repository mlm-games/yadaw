#![cfg(target_os = "android")]
use anyhow::{Result, anyhow};
use jni::JNIEnv;
use jni::objects::{JObject, JString, JValue};
use jni::sys::jint;

fn with_env<F, R>(f: F) -> Result<R>
where
    F: FnOnce(JNIEnv, JObject) -> Result<R>,
{
    let ctx = ndk_context::android_context();
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }
        .map_err(|_| anyhow!("VM not available"))?;
    let env = vm.attach_current_thread()?;
    let context = unsafe { JObject::from_raw(ctx.context().cast()) };
    f(env, context)
}

pub fn copy_from_content_uri_to_internal(
    content_uri: &str,
    dest_rel_name: &str,
) -> Result<std::path::PathBuf> {
    let dest = crate::paths::projects_dir().join(dest_rel_name);
    with_env(|mut env, context| {
        let resolver = env
            .call_method(
                &context,
                "getContentResolver",
                "()Landroid/content/ContentResolver;",
                &[],
            )?
            .l()?;
        let juri_class = env.find_class("android/net/Uri")?;
        let juri = env
            .call_static_method(
                juri_class,
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(JObject::from(env.new_string(content_uri)?))],
            )?
            .l()?;

        let in_stream = env
            .call_method(
                resolver,
                "openInputStream",
                "(Landroid/net/Uri;)Ljava/io/InputStream;",
                &[JValue::Object(juri)],
            )?
            .l()?;

        if in_stream.is_null() {
            return Err(anyhow!("openInputStream returned null"));
        }

        // Read InputStream -> Vec<u8> (64KB chunks)
        let buf_class = env.find_class("java/lang/Class")?; // only to keep env happy
        let input_stream = in_stream;
        let read_method_sig = "([B)I";
        let byte_array = env.new_byte_array(64 * 1024)?;
        let mut out = std::fs::File::create(&dest)?;

        loop {
            let read_bytes = env
                .call_method(
                    input_stream,
                    "read",
                    read_method_sig,
                    &[JValue::Object(JObject::from(byte_array))],
                )?
                .i()? as i32;

            if read_bytes <= 0 {
                break;
            }

            let mut buf = vec![0u8; read_bytes as usize];
            env.get_byte_array_region(byte_array, 0, &mut buf)?;
            use std::io::Write;
            out.write_all(&buf)?;
        }
        Ok(())
    })?;
    Ok(dest)
}

pub fn copy_to_content_uri_from_internal(
    src_path: &std::path::Path,
    content_uri: &str,
) -> Result<()> {
    with_env(|mut env, context| {
        let resolver = env
            .call_method(
                &context,
                "getContentResolver",
                "()Landroid/content/ContentResolver;",
                &[],
            )?
            .l()?;
        let juri_class = env.find_class("android/net/Uri")?;
        let juri = env
            .call_static_method(
                juri_class,
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(JObject::from(env.new_string(content_uri)?))],
            )?
            .l()?;

        let out_stream = env
            .call_method(
                resolver,
                "openOutputStream",
                "(Landroid/net/Uri;)Ljava/io/OutputStream;",
                &[JValue::Object(juri)],
            )?
            .l()?;

        if out_stream.is_null() {
            return Err(anyhow!("openOutputStream returned null"));
        }

        let mut file = std::fs::File::open(src_path)?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = std::io::Read::read(&mut file, &mut buf)?;
            if n == 0 {
                break;
            }
            let byte_array = env.new_byte_array(n as jint)?;
            env.set_byte_array_region(byte_array, 0, &buf[..n])?;
            let _ = env.call_method(
                out_stream,
                "write",
                "([B)V",
                &[JValue::Object(JObject::from(byte_array))],
            )?;
        }
        let _ = env.call_method(out_stream, "flush", "()V", &[])?;
        let _ = env.call_method(out_stream, "close", "()V", &[])?;
        Ok(())
    })
}
