# YADAW Quick Start

## Recording Audio

1. Click "Add Audio Track"
2. Click the red circle to arm the track
3. Press Space or click Record
4. Press Space again to stop

Your recording appears as a clip on the timeline.

## Creating MIDI

1. Click "Add MIDI Track"
2. Click "+ Plugins" and load a CLAP/LV2 instrument
3. Double-click on the timeline to create a clip
4. Double-click the clip to open piano roll
5. Double-click in piano roll to add notes

## Adding Effects

1. Click "+ Plugins" on any track
2. Select from the plugin browser
3. Adjust parameters

Plugins process top-to-bottom in the chain. Drag to reorder.

## Editing

- **Move clips**: Click and drag
- **Resize**: Drag the right edge
- **Duplicate**: Ctrl+D (duplicates to next beat)
- **Split**: Position playhead, press S
- **Delete**: Select and press Delete

## Automation

1. Expand a track to show automation lanes
2. Click "+" to add a parameter
3. Draw points on the automation curve
4. Drag points to adjust values

## Exporting

1. File → Export (or Ctrl+E)
2. Choose range (loop region or full project)
3. Select output location
4. Click Export

Exports to WAV only.

## Keyboard Shortcuts (defaults, changable in-app)

| Key | Action |
|-----|--------|
| Space | Play/Stop |
| Shift+Space | Record |
| Ctrl+Z | Undo |
| Ctrl+Shift+Z | Redo |
| Ctrl+D | Duplicate clip to next beat |
| S | Split at playhead |
| M | Toggle Mixer |
| Ctrl+E | Export |
| Ctrl+S | Save |

## Android Notes

- Use internal storage folders for saving (access is limited)
- Touch gestures work for zoom/pan
- Connect MIDI controllers via USB for better control
- Some features are limited compared to desktop

## Troubleshooting

**No audio output?**
- Check audio device settings
- Make sure track isn't muted
- Check master output level

**Plugins not showing?**
- Verify they're CLAP or LV2 format (not VST)
- Check plugin paths in settings
- Restart YADAW after installing new plugins

**MIDI not working?**
- Check MIDI input device is selected
- Make sure MIDI track is armed
- Verify instrument plugin is loaded

## Tips

- Color-code tracks by right-clicking (helps organize)
- Use the mixer (M key) for level balancing
- Set loop markers to work on specific sections
- Save frequently with Ctrl+S

For more help, check the video tutorial below or [open an issue](https://github.com/mlm-games/yadaw/issues).


### Get-started via a video

https://github.com/user-attachments/assets/4843c131-f972-49ce-92e3-96b08c483fff