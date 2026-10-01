import packageJson from '../../package.json';

export const packageAppVersion = packageJson.version || '0.0.0';

/** check_for_updates 的返回值中前端用到的字段 */
export interface UpdateCheckResult {
  has_update: boolean;
  latest_version: string;
  release_url: string;
}
