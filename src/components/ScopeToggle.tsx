import { useTranslation } from 'react-i18next';
import { OptionChips } from './TagEditorLayout';

/** 批量操作的应用范围：全部图片或当前图片 */
export default function ScopeToggle({ value, onChange, hasCurrent, color = '#60a5fa' }: {
  value: 'all' | 'current';
  onChange: (value: 'all' | 'current') => void;
  hasCurrent: boolean;
  /** 选中项的颜色：添加类弹窗为蓝色，删除类弹窗传红色 */
  color?: string;
}) {
  const { t } = useTranslation();
  return (
    <OptionChips legend={t('tagEditor.applyScope')} value={value} onChange={onChange} color={color} options={[
      { value: 'all', label: t('tagEditor.scopeAllImages') },
      { value: 'current', label: t('tagEditor.scopeCurrentImage'), disabled: !hasCurrent },
    ]} />
  );
}
