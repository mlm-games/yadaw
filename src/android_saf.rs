#![cfg(target_os = "android")]
use anyhow::{Result, anyhow};
use jni::objects::{JByteArray, JClass, JObject, JString, JValue};
use jni::sys::jint;
use jni::{JNIEnv, JavaVM};
use std::io::{Read, Write};

pub fn with_env<F, R>(f: F) -> Result<R>
where
    F: for<'a> FnOnce(&JNIEnv<'a>, JObject<'a>) -> Result<R>,
{
    let ctx = ndk_context::android_context();
    let vm =
        unsafe { JavaVM::from_raw(ctx.vm().cast()) }.map_err(|_| anyhow!("VM not available"))?;
    let env_guard = vm.attach_current_thread()?;
    let context = unsafe { JObject::from_raw(ctx.context().cast()) };
    f(&env_guard, context)
}

pub fn copy_from_content_uri_to_internal(
    content_uri: &str,
    dest_rel_name: &str,
) -> Result<std::path::PathBuf> {
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

        let juri_class: JClass = env.find_class("android/net/Uri")?;
        let jstr: JString = env.new_string(content_uri)?;
        let jstr_obj = JObject::from(jstr);

        let juri = env
            .call_static_method(
                juri_class,
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&jstr_obj)],
            )?
            .l()?;

        let in_stream = env
            .call_method(
                &resolver,
                "openInputStream",
                "(Landroid/net/Uri;)Ljava/io/InputStream;",
                &[JValue::Object(&juri)],
            )?
            .l()?;

        if in_stream.is_null() {
            return Err(anyhow!("openInputStream returned null"));
        }

        let mut out = std::fs::File::create(&dest)?;

        // Reuse one Java byte[] and pull chunks out
        let jbuf: JByteArray = env.new_byte_array(64 * 1024)?;
        let jbuf_obj = JObject::from(jbuf.clone());

        loop {
            let read_bytes = env
                .call_method(&in_stream, "read", "([B)I", &[JValue::Object(&jbuf_obj)])?
                .i()?;

            if read_bytes == -1 {
                break; // EOF
            }
            if read_bytes == 0 {
                continue;
            }

            // Option A: fast path using convert_byte_array on a sliced copy:
            // Not available for slices, so do i8 -> u8 manually for the read size.
            let mut chunk_i8 = vec![0i8; read_bytes as usize];
            env.get_byte_array_region(JByteArray::from(jbuf_obj.clone()), 0, &mut chunk_i8)?;
            let chunk_u8: Vec<u8> = chunk_i8.into_iter().map(|b| b as u8).collect();
            out.write_all(&chunk_u8)?;
        }

        let _ = env.call_method(&in_stream, "close", "()V", &[])?;
        Ok(())
    })?;

    Ok(dest)
}

pub fn copy_to_content_uri_from_internal(
    src_path: &std::path::Path,
    content_uri: &str,
) -> Result<()> {
    with_env(|env, context| {
        let resolver = env
            .call_method(
                &context,
                "getContentResolver",
                "()Landroid/content/ContentResolver;",
                &[],
            )?
            .l()?;

        let juri_class: JClass = env.find_class("android/net/Uri")?;
        let jstr: JString = env.new_string(content_uri)?;
        let jstr_obj = JObject::from(jstr);

        let juri = env
            .call_static_method(
                juri_class,
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&jstr_obj)],
            )?
            .l()?;

        let out_stream = env
            .call_method(
                &resolver,
                "openOutputStream",
                "(Landroid/net/Uri;)Ljava/io/OutputStream;",
                &[JValue::Object(&juri)],
            )?
            .l()?;

        if out_stream.is_null() {
            return Err(anyhow!("openOutputStream returned null"));
        }

        let mut file = std::fs::File::open(src_path)?;
        let mut buf = [0u8; 64 * 1024];

        // Reuse a single JByteArray and use write(byte[], off, len)
        let jarr: JByteArray = env.new_byte_array(buf.len() as jint)?;
        let jarr_obj = JObject::from(jarr.clone());

        loop {
            let n = Read::read(&mut file, &mut buf)?;
            if n == 0 {
                break;
            }

            // Convert to i8 and write only the n prefix
            let tmp_i8: &[i8] = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const i8, n) };
            env.set_byte_array_region(jarr, 0, tmp_i8)?;

            let _ = env.call_method(
                &out_stream,
                "write",
                "([BII)V",
                &[
                    JValue::Object(&jarr_obj),
                    JValue::Int(0),
                    JValue::Int(n as jint),
                ],
            )?;
        }

        let _ = env.call_method(&out_stream, "flush", "()V", &[])?;
        let _ = env.call_method(&out_stream, "close", "()V", &[])?;
        Ok(())
    })
}
