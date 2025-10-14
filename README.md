## Yadaw 

A basic daw for sound effects (works on Android too)

### Build from source
#### Desktop
- Just run using `cargo run` for desktops.
#### Android
- Follow the instructions in the third_party folder's md.
- Plugins should be compiled beforehand for android (plan to add a default robust one later)
- Hint: Saving and loading works on Android (is a hack for now, will implement proper perms later; currently only works for certain internal folders)


#### Known Issues

- "No VST support (LV2 and CLAP only)."
- "No MP3/FLAC export (WAV only)."
- No MIDI hardware support (will eventually add, mostly when midir supports android)
