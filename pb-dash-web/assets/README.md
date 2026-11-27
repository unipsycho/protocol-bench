# Dashboard fonts

egui's built-in fonts are switched off (`default-features = false`); the app embeds these instead
(`src/theme.rs`, `install_fonts`).

| File | What | Licence |
| --- | --- | --- |
| `NotoSans-Medium.ttf` | body text | SIL OFL 1.1 (`LICENSE-noto-fonts.txt`, from the Arch noto-fonts package) |
| `NotoSans-Bold.ttf` | values, headings | same |
| `NotoEmoji-Regular.ttf` | emoji / symbol fallback (copied from egui's `epaint_default_fonts`) | SIL OFL 1.1 (`LICENSE-NotoEmoji-OFL.txt`) |
| `emoji-icon-font.ttf` | more symbols (copied from egui's `epaint_default_fonts`) | MIT (`LICENSE-emoji-icon-font-MIT.txt`) |

The two Noto Sans files are trimmed (~620 KB → ~89 KB each) to Latin letters plus common technical
symbols; hinting and kerning are kept. Characters outside the set fall back to the emoji fonts.
To redo it from the full fonts:

```bash
uv run --isolated --with fonttools pyftsubset NotoSans-Medium-full.ttf \
  --unicodes="U+0020-007E,U+00A0-017F,U+0192,U+02C6-02DD,U+0394,U+03A3,U+03A9,U+03B1-03C9,U+2010-2027,U+2030,U+2032-2033,U+2039-203A,U+2044,U+2070-209F,U+20AC,U+2103,U+2109,U+2116,U+2122,U+2126,U+2190-2195,U+21C4,U+2202,U+2206,U+220F,U+2211-2212,U+2215,U+2219-221A,U+221E,U+2229,U+222B,U+2248,U+2260,U+2264-2265,U+25CF,U+FEFF,U+FFFD" \
  --layout-features='*' --name-IDs='*' --output-file=NotoSans-Medium.ttf
```

(Same for `NotoSans-Bold`.)
