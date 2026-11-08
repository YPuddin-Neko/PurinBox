import { useLocation } from 'react-router-dom';
import GbcRefineTab from '../components/GbcRefineTab';
import { useEffect, useState } from 'react';
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
  const location = useLocation();
  const [activeTab, setActiveTab] = useState(location.state?.gbcImage ? 'gbc' : 'ai');
  useEffect(() => { if (location.state?.gbcImage) setActiveTab('gbc'); }, [location.key, location.state]);

  const tabs = [
    { id: 'ai', label: t('tagger.aiTab') },
    { id: 'llm', label: t('tagger.llmTab') },
    { id: 'gbc', label: t('gbc.refineTab') },
    ...(hybridTaggerEnabled ? [{ id: 'hybrid', label: t('tagger.hybridTab') }] : []),
  ];

  const tab = !hybridTaggerEnabled && activeTab === 'hybrid' ? 'ai' : activeTab;

  return (
    <div className="page">
      <PageHeader icon={Tags} color="#f59e0b" title={t('tagger.title')} subtitle={t('tagger.subtitle')} />

      <SegmentedTabs tabs={tabs} value={tab} onChange={setActiveTab} style={{ marginBottom: 'var(--space-4)' }} />
      <div style={{ display: tab === 'ai' ? 'block' : 'none' }}><AiTaggerTab /></div>
      <div style={{ display: tab === 'llm' ? 'block' : 'none' }}><LlmTaggerTab /></div>
      <div style={{ display: tab === 'gbc' ? 'block' : 'none' }}><GbcRefineTab /></div>
      <div style={{ display: tab === 'hybrid' ? 'block' : 'none' }}><HybridTaggerTab /></div>
    </div>
  );
}
