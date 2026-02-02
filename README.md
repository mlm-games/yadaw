## Yadaw 

A basic daw for sound effects (works on Android too, but not as functional). Is also pretty lightweight (<20mb)

Current intention is to not fill the app with patches for outdated/non-standardised parts, and to keep it minimal, might implement a plugin system like blender if needed (for an example, midi controller lanes feature could be implemented as a plugin, etc..)

<img src="others/assets/demo.gif" align="center">

### Build from source
#### Desktop
- Just run using `cargo run` for windows, mac or linux, and install any missing libs (mostly pkgconfig, lv2 headers & livi components, windows doesn't have a release build, and needs to be compiled manually for now, maybe donation only in future? Need some sort of revenue ig).
#### Android
- Follow the instructions in the third_party folder's md.
- Plugins should be compiled beforehand for android (tested vitsel-clap and works well on android)
- Hint: Saving and loading works by scrolling fully to the bottom and clicking on the last 4th (or 5th) entry (is a hack for now, will implement proper perms later; currently only works for certain internal folders)


#### Missing common features (would not be done in the near future)

- No VST support (LV2 and CLAP only).
- No plugin GUIs (parameter based / DAW generated UIs only). (Is a egui/winit limitation, and the workarounds are questionable in terms of working, and the amount of spaghetti that'll be needed for them)

## Quick start

Refer the [quick start](quick_start.md) readme 

(or)

### Get-started via a video



https://github.com/user-attachments/assets/4843c131-f972-49ce-92e3-96b08c483fff


The video uses [VSCO2's](https://versilian-studios.com/vsco-community/) instruments via [samplo](https://github.com/mlm-games/samplo-clap), and plays the midi file from [here](https://www.ninsheetmusic.org/download/mid/5282) ([ninsheetmusic](https://www.ninsheetmusic.org/))

## Version Trackers

| Platform    | Version |
|-------------|---------|
| AUR         | [![AUR Version](https://img.shields.io/aur/version/yadaw-bin)](https://aur.archlinux.org/packages/yadaw-bin) |
| Flathub    | [![Flathub Version](https://img.shields.io/flathub/v/io.github.mlm_games.yadaw)](https://flathub.org/apps/io.github.mlm_games.yadaw) |

### License 

[AGPL-3.0](https://www.gnu.org/licenses/agpl-3.0.en.html)
