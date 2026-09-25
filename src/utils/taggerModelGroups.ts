/** 打标模型的系列分组与版本排序（普通打标/辅助打标的模型选择共用） */

export interface TaggerModelLike {
  id: string;
  name: string;
  is_builtin: boolean;
}

/** 自定义模型统一归入的系列键 */
export const CUSTOM_FAMILY = 'custom';

/** 内置模型按名称首词分系列（WD / PixAI / CL）；自定义模型不论名称一律归入 custom */
export function modelFamily(name: string, isBuiltin: boolean): string {
  if (!isBuiltin) return CUSTOM_FAMILY;
  return name.trim().split(/\s+/)[0] || name;
}

/** 取名称中最后一个版本样式 token（v3 / v1.0 / v2.01a / 2026）作为版本键；
 *  末尾字母按补丁版处理（v2.01a 比 v2.01 新） */
function versionKey(name: string): number[] | null {
  const re = /^v?(\d+(?:\.\d+)*)([a-z])?$/i;
  let found: number[] | null = null;
  for (const tok of name.trim().split(/\s+/)) {
    const m = re.exec(tok);
    if (m) {
      found = m[1].split('.').map(Number);
      found.push(m[2] ? m[2].toLowerCase().charCodeAt(0) - 96 : 0);
    }
  }
  return found;
}

/** 版本新的在前；无版本信息（未导入的自定义模型等）排最后按名称排序；同版本保持传入顺序 */
export function compareByVersionDesc<T extends TaggerModelLike>(a: T, b: T): number {
  const ka = versionKey(a.name);
  const kb = versionKey(b.name);
  if (ka && kb) {
    const len = Math.max(ka.length, kb.length);
    for (let i = 0; i < len; i++) {
      const d = (kb[i] ?? 0) - (ka[i] ?? 0);
      if (d !== 0) return d;
    }
    return 0;
  }
  if (ka) return -1;
  if (kb) return 1;
  return a.name.localeCompare(b.name);
}

export interface ModelGroup<T extends TaggerModelLike> {
  family: string;
  models: T[];
}

/** 按系列分组：系列间保持首次出现顺序（内置列表的编排顺序），custom 组固定最后；
 *  组内按版本新在前排序 */
export function groupTaggerModels<T extends TaggerModelLike>(models: T[]): ModelGroup<T>[] {
  const groups = new Map<string, T[]>();
  for (const m of models) {
    const f = modelFamily(m.name, m.is_builtin);
    const arr = groups.get(f) ?? [];
    arr.push(m);
    groups.set(f, arr);
  }
  const entries = [...groups.entries()];
  entries.sort(([fa], [fb]) => {
    if (fa === CUSTOM_FAMILY) return 1;
    if (fb === CUSTOM_FAMILY) return -1;
    return 0;
  });
  return entries.map(([family, list]) => ({ family, models: [...list].sort(compareByVersionDesc) }));
}
