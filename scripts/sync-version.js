/**
 * 版本号同步脚本
 * 从 package.json 读取版本号，同步到 tauri.conf.json 和 Cargo.toml
 * 用法: node scripts/sync-version.js
 */
import { readFileSync, writeFileSync } from 'fs';
import { resolve, dirname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const root = resolve(__dirname, '..');

// 读取 package.json 版本号（唯一来源）
const pkg = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf-8'));
const version = pkg.version;
console.log(`📦 同步版本号: v${version}`);

// 同步 tauri.conf.json
const tauriConfPath = resolve(root, 'src-tauri/tauri.conf.json');
const tauriConf = JSON.parse(readFileSync(tauriConfPath, 'utf-8'));
if (tauriConf.version !== version) {
  tauriConf.version = version;
  writeFileSync(tauriConfPath, JSON.stringify(tauriConf, null, 2) + '\n', 'utf-8');
  console.log(`  ✅ tauri.conf.json → ${version}`);
} else {
  console.log(`  ⏭  tauri.conf.json 已是最新`);
}

// 同步 Cargo.toml
const cargoPath = resolve(root, 'src-tauri/Cargo.toml');
let cargo = readFileSync(cargoPath, 'utf-8');
const cargoMatch = cargo.match(/^version\s*=\s*"(.+?)"/m);
if (cargoMatch && cargoMatch[1] !== version) {
  cargo = cargo.replace(/^version\s*=\s*".*?"/m, `version = "${version}"`);
  writeFileSync(cargoPath, cargo, 'utf-8');
  console.log(`  ✅ Cargo.toml → ${version}`);
} else {
  console.log(`  ⏭  Cargo.toml 已是最新`);
}

console.log('🎉 版本号同步完成');
