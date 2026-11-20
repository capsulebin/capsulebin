# Brand assets

| File | Content |
| --- | --- |
| `capsulebin-logo.svg` | The symbol (hexagon with lock), vector source |
| `capsulebin-logo.png` | The symbol, 512 × 512 |
| `capsulebin-wordmark.png` | The wordmark "CapsuleBin", 1119 × 264 |
| `capsulebin-logo-wordmark.png` | Symbol and wordmark side by side, 1431 × 304 |

All PNGs have a transparent background and are meant for light backgrounds.

- Symbol: accent colour `#0f766e` (`--cb-accent`), the same paths as the logo in the web client.
- Wordmark: `#0d1b1a` (`--cb-text`), Noto Sans at weight 650, letter spacing −0.01 em. The web
  client renders the wordmark in the system font; these files are the fixed reference.

The web client's built-in favicon, `src/server/web/public/favicon.svg`, is the symbol in white on a
rounded `#0f766e` tile, with fixed colours.

The name "CapsuleBin" and the logo identify the project; the AGPL covers the code, not the right to
use the name or logo for other products or services.

An operator can show an instance under their own name, logo and favicon (`[branding]`, see
[branding.md](../branding.md)). The footer link "Powered by CapsuleBin" stays on every instance and
keeps the project's name: it names the software the instance runs and points to its source code.
