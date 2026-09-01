# DOOM Overlay Demo

Play DOOM as a micro overlay. The example exercises real-time terminal rendering at 35 FPS.

## Usage

```bash
micro --extension ./examples/extensions/doom-overlay
```

Then run:
```
/doom-overlay
```

Pass a local WAD path to `/doom-overlay`, or place `doom1.wad` in the current directory, your home directory, or `~/.doom/`. The extension host has no network or write access, so it does not download the WAD.

The extension also needs `doom/build/doom.js` and `doom/build/doom.wasm`. If those generated files are absent, install Emscripten and build them from the repository root:

```bash
./examples/extensions/doom-overlay/doom/build.sh
```

## Controls

| Action | Keys |
|--------|------|
| Move | WASD or Arrow Keys |
| Run | Shift + WASD |
| Fire | F or Ctrl |
| Use/Open | Space |
| Weapons | 1-7 |
| Map | Tab |
| Menu | Escape |
| Pause/Quit | Q |

## How It Works

DOOM runs as WebAssembly compiled from [doomgeneric](https://github.com/ozkl/doomgeneric). Each frame is rendered using half-block characters (▀) with 24-bit color, where the top pixel is the foreground color and the bottom pixel is the background color.

The overlay uses:

- `width: "75%"` - 75% of terminal width
- `maxHeight: "95%"` - Maximum 95% of terminal height
- `anchor: "center"` - Centered in terminal

Height is calculated from width to maintain DOOM's 3.2:1 aspect ratio (accounting for half-block rendering).

## Credits

- [id Software](https://github.com/id-Software/DOOM) for the original DOOM
- [doomgeneric](https://github.com/ozkl/doomgeneric) for the portable DOOM implementation
- [pi-doom](https://github.com/badlogic/pi-doom) for the original pi integration
