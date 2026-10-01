import { useState } from 'react';
import SegmentedTabs from '../components/ui/SegmentedTabs';
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

      <SegmentedTabs tabs={tabs} value={activeTab} onChange={setActiveTab} style={{ marginBottom: 'var(--space-4)' }} />
      <div style={{ display: activeTab === 'sort' ? 'block' : 'none' }}><TagSortTab /></div>
      <div style={{ display: activeTab === 'refine' ? 'block' : 'none' }}><TagRefineTab /></div>
    </div>
  );
}
