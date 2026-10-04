import { useState } from 'react';
import SegmentedTabs from '../components/ui/SegmentedTabs';
import PageHeader from '../components/ui/PageHeader';
import { Tags } from 'lucide-react';
import { useTranslation } from 'react-i18next';
import AiTaggerTab from '../components/AiTaggerTab';
import LlmTaggerTab from '../components/LlmTaggerTab';
import HybridTaggerTab from '../components/HybridTaggerTab';
import { useAppSettings } from '../components/ThemeProvider';

export default function TaggerPage() {
  const { t } = useTranslation();
  const { hybridTaggerEnabled } = useAppSettings();
  const [activeTab, setActiveTab] = useState('ai');

  const tabs = [
    { id: 'ai', label: t('tagger.aiTab') },
    { id: 'llm', label: t('tagger.llmTab') },
    ...(hybridTaggerEnabled ? [{ id: 'hybrid', label: t('tagger.hybridTab') }] : []),
  ];

  const tab = !hybridTaggerEnabled && activeTab === 'hybrid' ? 'ai' : activeTab;

  return (
    <div className="page">
      <PageHeader icon={Tags} color="#f59e0b" title={t('tagger.title')} subtitle={t('tagger.subtitle')} />

      <SegmentedTabs tabs={tabs} value={tab} onChange={setActiveTab} style={{ marginBottom: 'var(--space-4)' }} />
      <div style={{ display: tab === 'ai' ? 'block' : 'none' }}><AiTaggerTab /></div>
      <div style={{ display: tab === 'llm' ? 'block' : 'none' }}><LlmTaggerTab /></div>
      <div style={{ display: tab === 'hybrid' ? 'block' : 'none' }}><HybridTaggerTab /></div>
    </div>
  );
}
