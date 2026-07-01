import packageJson from '../../package.json';

export const packageAppVersion = packageJson.version || '0.0.0';

export async function getAppVersion() {
  try {
    const { getVersion } = await import('@tauri-apps/api/app');
    return await getVersion();
  } catch {
    return packageAppVersion;
  }
}
