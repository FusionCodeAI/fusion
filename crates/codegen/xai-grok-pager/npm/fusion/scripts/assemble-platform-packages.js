#!/usr/bin/env node
// Assemble the six per-platform npm packages prior to `npm publish`.
const fs = require('fs');
const path = require('path');
const { promisify } = require('util');
const zlib = require('zlib');

const brotliCompress = promisify(zlib.brotliCompress);

const fusionRoot = process.env.FUSION_ROOT || path.resolve(__dirname, '..', '..', '..', '..', '..', '..');
const npmRoot = path.resolve(__dirname, '..', '..');

const NOTICES_SOURCE = path.resolve(fusionRoot, 'LICENSE');
const NOTICES_NAME = 'THIRD_PARTY_NOTICES.md';

const META_PKG_JSON = path.resolve(__dirname, '..', 'package.json');
const meta = JSON.parse(fs.readFileSync(META_PKG_JSON, 'utf8'));
const VERSION = meta.version;

function ensureDir(p) { fs.mkdirSync(path.dirname(p), { recursive: true }); }

async function packPlatform({ platform, arch, envVar, defaultSource, binName }) {
    const pkgDir = path.join(npmRoot, `fusion-${platform}-${arch}`);
    const pkgJsonPath = path.join(pkgDir, 'package.json');

    if (!fs.existsSync(pkgJsonPath)) {
        console.error(`[assemble] Missing per-platform package at ${pkgDir}`);
        return false;
    }

    const source = process.env[envVar] || defaultSource;
    if (!fs.existsSync(source)) {
        console.warn(`[assemble] Skipping ${platform}-${arch}: missing binary at ${source}`);
        return true;
    }

    // Stamp the sub-package's version to match the meta package.
    const subPkg = JSON.parse(fs.readFileSync(pkgJsonPath, 'utf8'));
    subPkg.version = VERSION;
    fs.writeFileSync(pkgJsonPath, JSON.stringify(subPkg, null, 4) + '\n');

    if (fs.existsSync(NOTICES_SOURCE)) {
        fs.copyFileSync(NOTICES_SOURCE, path.join(pkgDir, NOTICES_NAME));
    }
    const README_SOURCE = path.resolve(fusionRoot, 'README.md');
    if (fs.existsSync(README_SOURCE)) {
        fs.copyFileSync(README_SOURCE, path.join(pkgDir, 'README.md'));
    }

    // Brotli-compress into the sub-package's bin/.
    const outBr = path.join(pkgDir, 'bin', `${binName}.br`);
    ensureDir(outBr);
    const raw = fs.readFileSync(source);
    const compressed = await brotliCompress(raw, {
        params: { [zlib.constants.BROTLI_PARAM_QUALITY]: zlib.constants.BROTLI_MAX_QUALITY },
    });
    fs.writeFileSync(outBr, compressed);
    console.log(
        `[assemble] fusion-${platform}-${arch}@${VERSION}: ` +
        `${(raw.length / 1048576).toFixed(1)} MB -> ${(compressed.length / 1048576).toFixed(1)} MB ` +
        `(${path.relative(npmRoot, outBr)})`
    );
    return true;
}

async function main() {
    const targets = [
        {
            platform: 'darwin', arch: 'arm64', binName: 'fusion',
            envVar: 'FUSION_DARWIN_ARM64',
            defaultSource: fs.existsSync(path.join(fusionRoot, 'target', 'release', 'fusion'))
                ? path.join(fusionRoot, 'target', 'release', 'fusion')
                : path.join(fusionRoot, 'target', 'aarch64-apple-darwin', 'release', 'fusion'),
        },
        {
            platform: 'darwin', arch: 'x64', binName: 'fusion',
            envVar: 'FUSION_DARWIN_X64',
            defaultSource: path.join(fusionRoot, 'target', 'x86_64-apple-darwin', 'release', 'fusion'),
        },
        {
            platform: 'linux', arch: 'x64', binName: 'fusion',
            envVar: 'FUSION_LINUX_X64',
            defaultSource: path.join(fusionRoot, 'target', 'x86_64-unknown-linux-gnu', 'release', 'fusion'),
        },
        {
            platform: 'linux', arch: 'arm64', binName: 'fusion',
            envVar: 'FUSION_LINUX_ARM64',
            defaultSource: path.join(fusionRoot, 'target', 'aarch64-linux-android', 'release', 'fusion'),
        },
        {
            platform: 'win32', arch: 'x64', binName: 'fusion.exe',
            envVar: 'FUSION_WIN32_X64',
            defaultSource: path.join(fusionRoot, 'target', 'x86_64-pc-windows-msvc', 'release', 'fusion.exe'),
        },
        {
            platform: 'win32', arch: 'arm64', binName: 'fusion.exe',
            envVar: 'FUSION_WIN32_ARM64',
            defaultSource: path.join(fusionRoot, 'target', 'aarch64-pc-windows-msvc', 'release', 'fusion.exe'),
        },
    ];

    const results = await Promise.all(targets.map(packPlatform));
    const failed = results.filter(r => !r).length;
    if (failed > 0) {
        console.error(`[assemble] ${failed} target(s) failed.`);
        process.exit(1);
    }

    console.log(`[assemble] All 6 per-platform packages assembled at version ${VERSION}.`);
}

main().catch((err) => { console.error(err); process.exit(1); });
