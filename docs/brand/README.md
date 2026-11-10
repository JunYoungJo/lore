# Lore mark

`lore.svg` is the editable source for the selected Lore icon concept. The white spine and bottom cover form an **L**, with a bookmarked page beside it. Its blue palette follows the current application's accent colors.

`256x256.png` is used by the repository README files. Regenerate it from the repository root:

```sh
pnpm tauri icon docs/brand/lore.svg --output docs/brand --png 256
```

These documentation assets do not replace the current runtime icons in `src-tauri/icons/` or `src/ui/BrandMark.tsx`.
