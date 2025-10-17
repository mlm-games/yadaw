## Yadaw 

A basic daw for sound effects (works on Android too)

Current intention is to not add too much code for outdated parts, and to keep it minimal; helps refactoring later on, might implement a plugin system like blender if needed (for example, midi controller lanes feature could be implemented as a plugin, etc..)

### Build from source
#### Desktop
- Just run using `cargo run` for desktops.
#### Android
- Follow the instructions in the third_party folder's md.
- Plugins should be compiled beforehand for android (plan to add a default robust one later)
- Hint: Saving and loading works on Android (is a hack for now, will implement proper perms later; currently only works for certain internal folders)


#### Known Issues (would not be done in the near future)

- No VST support (LV2 and CLAP only).
- No MP3/FLAC export (WAV only).
