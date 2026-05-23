import { useState } from 'react';
import { Wand2 } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import TagSortTab from '../components/TagSortTab';
import TagRefineTab from '../components/TagRefineTab';

export default function TagSortPage() {
  const { t } = useTranslation();
  const [activeTab, setActiveTab] = useState('sort');

  const tabs = [
    { id: 'sort', label: t('tagOptimize.sortTab') },
    { id: 'refine', label: t('tagOptimize.refineTab') },
  ];

  return (
    <div className="page">
      <div className="page-header">
        <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 4 }}>
          <Wand2 style={{ width: 28, height: 28, color: '#a78bfa' }} />
          <h1 className="page-title">{t('tagOptimize.title')}</h1>
        </div>
        <p className="page-subtitle">{t('tagOptimize.subtitle')}</p>
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

      {activeTab === 'sort' ? <TagSortTab /> : <TagRefineTab />}
    </div>
  );
}
