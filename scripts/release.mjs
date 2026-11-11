import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { appendFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const gh = (...args) => execFileSync('gh', args, { encoding: 'utf8', cwd: root }).trim();
const read = path => readFileSync(join(root, path), 'utf8');

export function validateTag(tag) {
  const version = JSON.parse(read('src-tauri/tauri.conf.json')).version;
  assert.equal(tag, `v${version}`, 'Release tag must match tauri.conf.json');
  assert.equal(JSON.parse(read('package.json')).version, version, 'package.json version differs');
  assert.equal(read('Cargo.toml').match(/\[workspace.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m)?.[1], version, 'Cargo workspace version differs');
  assert(read(`releases/${tag}.md`).trim(), 'Release notes are empty');
  return version;
}

const targets = [
  ['darwin-aarch64', /(?:aarch64|arm64).*\.app\.tar\.gz$/i],
  ['darwin-x86_64', /(?:x64|x86_64|amd64).*\.app\.tar\.gz$/i],
  ['windows-x86_64', /(?:x64|x86_64).*setup\.exe$/i],
  ['linux-x86_64', /(?:amd64|x86_64|x64).*\.AppImage$/i],
];

export function selectArtifacts(assets) {
  return Object.fromEntries(targets.map(([target, pattern]) => {
    const matches = assets.filter(asset => asset.name.startsWith('Lore_') && pattern.test(asset.name));
    assert.equal(matches.length, 1, `Expected one updater artifact for ${target}; found ${matches.length}`);
    const artifact = matches[0];
    assert(artifact.size > 0, `Empty updater artifact: ${artifact.name}`);
    const signature = assets.find(asset => asset.name === `${artifact.name}.sig`);
    assert(signature?.size > 0, `Missing signature for ${artifact.name}`);
    assert(artifact.browser_download_url.startsWith('https://github.com/JunYoungJo/lore/releases/download/'), 'Unexpected artifact URL');
    return [target, { artifact, signature }];
  }));
}

export function platformEntry(tag, artifact, signature) {
  // Draft asset URLs can contain a temporary "untagged-..." path that stops
  // working after publication. The updater must use the final version tag.
  assert(!/[\\/]/.test(artifact.name), 'Invalid release asset name');
  return { url: `https://github.com/JunYoungJo/lore/releases/download/${encodeURIComponent(tag)}/${encodeURIComponent(artifact.name)}`, signature };
}

export function requireDraft(releases, tag) {
  const release = releases.find(release => release.tag_name === tag);
  assert(release, `No release found for ${tag}`);
  assert(release.draft, 'Refusing to modify a published release');
  return release;
}

export function loadDraft(tag) {
  // GitHub's release-by-tag endpoint can return 404 for an unpublished draft.
  // The authenticated releases list includes drafts owned by this repository.
  return requireDraft(JSON.parse(gh('api', 'repos/JunYoungJo/lore/releases?per_page=100')), tag);
}

export function existingOrCreatedDraft(releases, tag, create) {
  const existing = releases.find(release => release.tag_name === tag);
  return requireDraft([existing ?? create()], tag);
}

function createDraft(tag) {
  gh('api', `repos/JunYoungJo/lore/git/ref/tags/${tag}`);
  const temporary = mkdtempSync(join(tmpdir(), 'lore-draft-'));
  try {
    const input = join(temporary, 'release.json');
    writeFileSync(input, JSON.stringify({ tag_name: tag, name: `Lore ${tag}`, body: read(`releases/${tag}.md`).trim(), draft: true, prerelease: false }));
    // Use the POST response directly: a freshly created draft may not appear
    // in GitHub's cached releases list immediately.
    return JSON.parse(gh('api', '--method', 'POST', 'repos/JunYoungJo/lore/releases', '--input', input));
  } finally { rmSync(temporary, { recursive: true, force: true }); }
}

export function prepare(tag) {
  validateTag(tag);
  const releases = JSON.parse(gh('api', 'repos/JunYoungJo/lore/releases?per_page=100'));
  const release = existingOrCreatedDraft(releases, tag, () => createDraft(tag));
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `tag=${tag}\nid=${release.id}\n`);
  console.log(`Draft release ready: ${release.html_url}`);
}

export function manifest(tag) {
  const version = validateTag(tag), release = loadDraft(tag);
  const artifacts = selectArtifacts(release.assets);
  const temporary = mkdtempSync(join(tmpdir(), 'lore-release-'));
  try {
    const platforms = {};
    for (const [target, { artifact, signature }] of Object.entries(artifacts)) {
      gh('release', 'download', tag, '--pattern', signature.name, '--dir', temporary);
      const signed = readFileSync(join(temporary, signature.name), 'utf8').trim();
      assert(/^[-A-Za-z0-9+/=\r\n]+$/.test(signed), `Invalid signature encoding for ${target}`);
      platforms[target] = platformEntry(tag, artifact, signed);
    }
    const output = join(temporary, 'latest.json');
    writeFileSync(output, JSON.stringify({ version, notes: read(`releases/${tag}.md`).trim(), pub_date: release.created_at, platforms }, null, 2) + '\n');
    gh('release', 'upload', tag, output, '--clobber');
    console.log(`Uploaded latest.json with all ${Object.keys(platforms).length} signed platforms. Release remains a draft.`);
  } finally { rmSync(temporary, { recursive: true, force: true }); }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [, , command, tag] = process.argv;
  assert(['prepare', 'manifest', 'validate'].includes(command), 'Usage: node scripts/release.mjs prepare|manifest|validate vX.Y.Z');
  if (command === 'validate') console.log(`Release version: ${validateTag(tag)}`);
  else if (command === 'prepare') prepare(tag);
  else manifest(tag);
}
