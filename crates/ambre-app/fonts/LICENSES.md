# Bundled font licenses

Fonts shipped under this directory are served from `/fonts/` and copied to
`dist/fonts/` by Trunk. The Noto faces are under the **SIL Open Font License,
Version 1.1** (OFL) -- full text in [`OFL.txt`](./OFL.txt); the DejaVu faces
are under the permissive, OFL-compatible Bitstream Vera / DejaVu license.

| File | Family | Source | License |
| :--- | :--- | :--- | :--- |
| `NotoSans-Regular.ttf` | Noto Sans | Google Noto Fonts | OFL 1.1 |
| `NotoSans-Bold.ttf` | Noto Sans | Google Noto Fonts | OFL 1.1 |
| `NotoSans-Italic.ttf` | Noto Sans | Google Noto Fonts | OFL 1.1 |
| `NotoSans-BoldItalic.ttf` | Noto Sans | Google Noto Fonts | OFL 1.1 |
| `NotoSansSC-Regular.otf` | Noto Sans SC | Google Noto Fonts | OFL 1.1 |
| `NotoSansArabic-Regular.ttf` | Noto Sans Arabic | Google Noto Fonts | OFL 1.1 |
| `NotoSansDevanagari-Regular.ttf` | Noto Sans Devanagari | Google Noto Fonts | OFL 1.1 |
| `NotoColorEmoji.ttf` | Noto Color Emoji (CBDT bitmap build) | Google Noto Emoji | OFL 1.1 |
| `DejaVuSerif.ttf` | DejaVu Serif | DejaVu Fonts | Bitstream Vera / DejaVu (OFL-compatible, permissive) |
| `DejaVuSansMono.ttf` | DejaVu Sans Mono | DejaVu Fonts | Bitstream Vera / DejaVu (OFL-compatible, permissive) |

The OFL permits embedding, redistribution, and self-hosting (which Ambre relies
on: fonts are served same-origin, never from a CDN) with no runtime-attribution
burden in the UI. `NotoColorEmoji.ttf` is the CBDT embedded-bitmap build:
swash 0.2.7 rasterizes its color glyphs (`Content::Color`, RGBA), whereas the
COLRv1 vector build is not yet supported by swash.
