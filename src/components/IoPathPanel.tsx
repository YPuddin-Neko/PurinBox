import type { LucideIcon } from 'lucide-react';
import { FolderOutput } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import PathInput from './ui/PathInput';
import RecursiveScanToggle from './RecursiveScanToggle';

const ICON = { width: 13, height: 13, color: 'var(--color-text-tertiary)' } as const;
const LABEL = { display: 'flex', alignItems: 'center', gap: 6 } as const;

/** 标签细化、标签排序的输入与输出目录；传 onRecursive 时输入行带递归扫描开关 */
export default function IoPathPanel({ inputIcon: InputIcon, inputLabel, inputPlaceholder, input, onInput, pickImage = false,
  outputLabel, outputPlaceholder, output, onOutput, recursive = false, onRecursive }: {
  inputIcon: LucideIcon;
  inputLabel: string;
  inputPlaceholder: string;
  input: string;
  onInput: (path: string) => void;
  /** 输入也可以选单张图片 */
  pickImage?: boolean;
  outputLabel: string;
  outputPlaceholder: string;
  output: string;
  onOutput: (path: string) => void;
  recursive?: boolean;
  onRecursive?: (recursive: boolean) => void;
}) {
  const { t } = useTranslation();
  const inputLabelNode = <label className="form-label" style={LABEL}><InputIcon style={ICON} /> {inputLabel}</label>;
  return (
    <div className="tool-panel">
      <div className="tool-panel-header"><span className="tool-panel-title">{t('pages.pathSettings')}</span></div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
        <div className="form-group" style={{ marginBottom: 0 }}>
          {onRecursive
            ? <div className="form-label-row">{inputLabelNode}<RecursiveScanToggle checked={recursive} onChange={onRecursive} /></div>
            : inputLabelNode}
          <PathInput value={input} onChange={onInput} pick={pickImage ? 'folderOrImage' : 'folder'} placeholder={inputPlaceholder} aria-label={inputLabel} />
        </div>
        <div className="form-group" style={{ marginBottom: 0 }}>
          <label className="form-label" style={LABEL}><FolderOutput style={ICON} /> {outputLabel}</label>
          <PathInput kind="output" value={output} onChange={onOutput} placeholder={outputPlaceholder} aria-label={outputLabel} />
        </div>
      </div>
    </div>
  );
}
