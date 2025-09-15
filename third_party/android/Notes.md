To run the android build, use the below commands (first copy the libs (termux files) to the location)

```sh
export LILV_TERMUX_LIB=$(pwd)/thirdparty/android/termux/aarch64/sysroot/data/data/com.termux/files/usr/lib
```

### Add the lib target (if removed)

```toml
[lib]
name = "yadaw"
crate-type = ["cdylib"]
```

and then build it using 
`cargo apk build --target aarch64-linux-android --lib`

> Note: lv2 plugins that are compiled using the android ndk only work... Might allow for placing them in the plugins dir of the internal storage (using root or other methods)