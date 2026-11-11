import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { requireDraft, selectArtifacts, validateTag } from './release.mjs';

const names = ['Lore_0.1.0_aarch64.app.tar.gz', 'Lore_0.1.0_x64.app.tar.gz', 'Lore_0.1.0_x64-setup.exe', 'Lore_0.1.0_amd64.AppImage'];
const assets = names.flatMap((name, id) => [
  { id, name, size: 100, browser_download_url: `https://github.com/JunYoungJo/lore/releases/download/v0.1.0/${name}` },
  { id: id + 10, name: `${name}.sig`, size: 100 },
]);

test('selects every platform and its signature, ignoring unrelated assets', () => {
  assert.equal(Object.keys(selectArtifacts([...assets, { name: 'latest.json', size: 100 }])).length, 4);
});
test('rejects missing builds, missing signatures, ambiguous files, and foreign URLs', () => {
  assert.throws(() => selectArtifacts(assets.slice(2)), /Expected one/);
  assert.throws(() => selectArtifacts(assets.filter(a => a.name !== `${names[0]}.sig`)), /Missing signature/);
  assert.throws(() => selectArtifacts([...assets, { ...assets[0] }]), /Expected one/);
  assert.throws(() => selectArtifacts(assets.map(a => a === assets[0] ? { ...a, browser_download_url: 'https://example.com/installer' } : a)), /Unexpected artifact URL/);
});
test('requires release tag, app, frontend and Rust versions to agree', () => {
  const version = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8')).version;
  assert.equal(validateTag(`v${version}`), version);
  assert.throws(() => validateTag('not-a-version-tag'), /Release tag/);
});

test('finds unpublished drafts in the authenticated list and refuses published releases', () => {
  const releases = [{ tag_name: 'v0.1.0', draft: false }, { tag_name: 'v0.1.1', draft: true, id: 12 }];
  assert.equal(requireDraft(releases, 'v0.1.1').id, 12);
  assert.throws(() => requireDraft(releases, 'v0.1.0'), /published release/);
  assert.throws(() => requireDraft(releases, 'v0.2.0'), /No release found/);
});
