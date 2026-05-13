import { useState } from 'react';
import { Tags } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import AiTaggerTab from '../components/AiTaggerTab';
import LlmTaggerTab from '../components/LlmTaggerTab';

export default function TaggerPage() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState('ai');

  const tabs = [
    { id: 'ai', label: t('tagger.aiTab') },
    { id: 'llm', label: t('tagger.llmTab') },
  ];

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Tags style={{ width: 28, height: 28, color: '#f59e0b' }} />
          <h1 className="page-title">{t('tagger.title')}</h1>
        </div>
        <p className="page-subtitle">{t('tagger.subtitle')}</p>
      </div>

      {/* Tab Bar */}
      <div style={{
        display: 'flex', gap: 2, marginBottom: 'var(--space-4)',
        background: 'var(--color-bg-card)', borderRadius: 'var(--radius-lg)',
        padding: 3, border: '1px solid var(--color-border)',
        width: 'fit-content',
      }}>
        {tabs.map(tab => (
          <button key={tab.id} onClick={() => setActiveTab(tab.id)} style={{
            padding: '8px 20px', borderRadius: 'var(--radius-md)', border: 'none',
            cursor: 'pointer', fontSize: 'var(--font-size-sm)', fontWeight: 600,
            transition: 'all 0.2s', fontFamily: 'inherit',
            background: activeTab === tab.id ? 'var(--color-accent-primary)' : 'transparent',
            color: activeTab === tab.id ? '#fff' : 'var(--color-text-tertiary)',
          }}>{tab.label}</button>
        ))}
      </div>

      {activeTab === 'ai' ? <AiTaggerTab /> : <LlmTaggerTab />}
    </div>
  );
}
