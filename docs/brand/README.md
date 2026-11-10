# Lore mark

`lore.svg` is the editable color mark. Its white spine and bottom cover form an **L**, with a bookmarked page beside it. The app header renders the same geometry in `src/ui/BrandMark.tsx`.

`256x256.png` is used by the README files. Regenerate it from the repository root:

```sh
pnpm tauri icon docs/brand/lore.svg --output docs/brand --png 256
```

All tracked platform icons in `src-tauri/icons/` are generated from this mark, including the Windows ICO and macOS ICNS used by the app and installers. `src-tauri/icons/source.svg` is a copy of the color source.

The macOS menu bar uses a separate monochrome L/book symbol, `src-tauri/icons/tray.svg`, rendered to `tray.png` at 44 × 44 pixels and marked as a template image. Windows and Linux trays use the color app icon.
