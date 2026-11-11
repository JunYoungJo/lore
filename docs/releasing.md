# Desktop releases

Lore uses GitHub Releases for installers and the Tauri updater manifest. The source repository and release assets are public. Update signatures are separate from operating-system code signing: current builds have no Apple Developer ID/notarization or Windows signing certificate.

macOS builds use ad-hoc signing (`bundle.macOS.signingIdentity: "-"` in `tauri.macos.conf.json`). This seals the app bundle and CLI without an Apple-issued identity. It does not bypass Gatekeeper or replace notarization; downloaded apps may still require approval in Privacy & Security.

## One-time signing setup

- The public key is committed under `plugins.updater.pubkey` in `src-tauri/tauri.conf.json`.
- The matching private key and its password are Actions secrets named `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
- Keep an independent backup of the private key and password. GitHub Secrets cannot be read back as a backup. Never commit either value or include it in a release artifact, log, issue, or conversation.

No signing secret is required to run tests or build without updater artifacts. For a local development bundle without updater artifacts, override only `bundle.createUpdaterArtifacts`:

```sh
pnpm tauri build --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

## Prepare a version

1. Set the same version in `Cargo.toml` (`workspace.package.version`), `package.json`, and `src-tauri/tauri.conf.json`. Refresh `Cargo.lock` if the workspace version changes.
2. Write `releases/vX.Y.Z.md`. Record concrete changes, requirements, installation notes, and material limits.
3. Run the normal tests and `node --test scripts/release.test.mjs`. Validate the version with `node scripts/release.mjs validate vX.Y.Z`.
4. Commit the changes to `main`, then create and push the matching `vX.Y.Z` tag. Existing published version tags and releases must not be overwritten.

## Build and review

The **Release Lore** workflow creates a draft, then builds and tests macOS Apple Silicon, macOS Intel, Windows x64, and Linux x64. Each platform bundles its matching `lore-cli` sidecar. Windows publishes NSIS only; Linux publishes DEB and AppImage. The GitHub updater uses the AppImage, not the DEB package.

Every updater artifact has a `.sig` file. After all build jobs succeed, one manifest job checks the four platform artifacts and signatures, then uploads `latest.json`. Matrix jobs do not each modify that file. A missing or ambiguous platform fails the job; the release remains a draft.

The macOS build jobs also run `scripts/verify-macos-bundles.sh` against the app, the extracted updater archive, and the mounted DMG. Both the app and its CLI must pass strict code-signature verification. This checks bundle integrity, not Developer ID trust or notarization.

Check the draft assets and native startup before publishing. The release must have two DMGs, a Windows `setup.exe`, a DEB, an AppImage, both macOS app updater archives, their signatures, and `latest.json`.

Publish a checked draft through GitHub Releases or:

```sh
gh release edit vX.Y.Z --repo JunYoungJo/lore --draft=false --latest
```

Until publication, unauthenticated clients cannot download the draft or its manifest. A first installation of the same version reports that it is current; testing a real upgrade requires an older installed version and a newer published release. Do not label that as verified after only testing the current-version check.

For a failed draft build, rerun failed workflow jobs. **Run workflow** can also rebuild an existing version tag. Both paths refuse to modify a published release. If source changes are needed after a tag was created, use a new version/tag rather than moving the existing tag.

## In-app behavior

Release builds check for updates once after startup. A found version appears in the tray popover; Settings → App updates and the tray's **Check for updates…** menu provide manual checks. A failed check stays in the update settings and does not interrupt recording.

**Install & restart** is explicit. The backend takes the shared operation gate and setup exclusion flag, refuses active recording/sync/review/setup-terminal work, downloads and verifies the update, and only then installs it. Windows hands off to NSIS; macOS and AppImage restart after successful installation. Queue, settings, and memory are not reset. Automatic Windows upgrades retain the NSIS update-mode guard that skips integration cleanup.

A standalone development binary is not an installation target. On Linux, DEB users should install a newer package; in-app installation requires running the AppImage. After moving from a development build to an installed app, use **Manage Lore** to repair connections to the installed sidecar, and review Codex hook trust if requested.
