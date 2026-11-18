# Bundled fonts

The UI ships its own monospace font so it looks the same on every machine and
never fetches fonts over the network.

| File | Font | Version | Licence |
|---|---|---|---|
| `SourceCodePro-VF-Upright.woff2` | Source Code Pro, variable upright (weights 200–900) | 2.042R-u / 1.026R-vf | SIL OFL 1.1, see `OFL-SourceCodePro.md` |

Source: <https://github.com/adobe-fonts/source-code-pro/releases> (`SourceCodeVF-Upright.ttf.woff2`
from the WOFF2 archive, unmodified apart from the file name).

Chosen because it covers the glyphs the terminal needs: ₹, ▲/▼ and arrows. JetBrains
Mono was considered but has no ₹ glyph.
