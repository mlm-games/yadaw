To run the android build, use the below commands (first copy the libs (termux files) to the location)

```sh
export LILV_TERMUX_LIB=/home/user/clone_src.../yadaw/thirdparty/android/termux/aarch64/sysroot/data/data/com.termux/files/usr/lib
```

### Add the lib target (if removed)

```toml
[lib]
name = "yadaw"
crate-type = ["cdylib"]
```

and then build it using 
`cargo apk build --target aarch64-linux-android --lib`