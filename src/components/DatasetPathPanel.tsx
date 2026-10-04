import { useTranslation } from 'react-i18next';
import PathInput from './ui/PathInput';
import RecursiveScanToggle from './RecursiveScanToggle';

/** 打标各子页的数据集路径：文件夹或单张图片，标题栏带递归扫描开关 */
export default function DatasetPathPanel({ value, onChange, recursive, onRecursive }: {
  value: string;
  onChange: (path: string) => void;
  recursive: boolean;
  onRecursive: (recursive: boolean) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="tool-panel">
      <div className="tool-panel-header">
        <span className="tool-panel-title">{t('tagger.datasetPath')}</span>
        <RecursiveScanToggle checked={recursive} onChange={onRecursive} />
      </div>
      <PathInput value={value} onChange={onChange} pick="folderOrImage" placeholder={t('tagger.selectFolder')}
        aria-label={t('tagger.datasetPath')} />
    </div>
  );
}
