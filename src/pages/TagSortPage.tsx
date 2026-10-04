import { useState } from 'react';
import SegmentedTabs from '../components/ui/SegmentedTabs';
import PageHeader from '../components/ui/PageHeader';
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
      <PageHeader icon={Wand2} color="#a78bfa" title={t('tagOptimize.title')} subtitle={t('tagOptimize.subtitle')} />

      <SegmentedTabs tabs={tabs} value={activeTab} onChange={setActiveTab} style={{ marginBottom: 'var(--space-4)' }} />
      <div style={{ display: activeTab === 'sort' ? 'block' : 'none' }}><TagSortTab /></div>
      <div style={{ display: activeTab === 'refine' ? 'block' : 'none' }}><TagRefineTab /></div>
    </div>
  );
}
