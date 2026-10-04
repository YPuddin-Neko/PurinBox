import { useId } from 'react';
import { MessageSquare } from 'lucide-react';
import { useTranslation } from 'react-i18next';

/** LLM 提示词面板：标题栏带「恢复默认」，下面是可编辑的提示词 */
export default function PromptPanel({ title, value, onChange, onReset }: {
  title: string;
  value: string;
  onChange: (value: string) => void;
  onReset: () => void;
}) {
  const { t } = useTranslation();
  const id = useId();
  return (
    <div className="tool-panel">
      <div className="tool-panel-header">
        <span className="tool-panel-title">{title}</span>
        <button className="btn btn-ghost btn-sm" style={{ fontSize: 10 }} onClick={onReset}>{t('llmApi.resetDefault')}</button>
      </div>
      <div className="form-group" style={{ marginBottom: 0 }}>
        <label className="form-label" htmlFor={id} style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <MessageSquare style={{ width: 13, height: 13, color: 'var(--color-text-tertiary)' }} /> Prompt
        </label>
        <textarea id={id} className="form-input" rows={8} value={value} onChange={e => onChange(e.target.value)}
          style={{ resize: 'vertical', fontFamily: 'monospace', fontSize: 12 }} />
      </div>
    </div>
  );
}
