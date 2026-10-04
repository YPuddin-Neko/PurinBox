import { useState, type CSSProperties, type ReactNode } from 'react';
import { useTranslation } from 'react-i18next';
import { Bot, Eye, EyeOff, Globe, Key, Loader2, RefreshCw, Save } from 'lucide-react';
import CustomSelect from './CustomSelect';
import type { LlmApiConfigController } from '../hooks/useLlmApiConfig';
import { LLM_PRESETS, chatCompletionsUrl, type LlmPresetId } from '../utils/llmPresets';

interface LlmApiPanelProps {
  /** useLlmApiConfig() 的返回值 */
  api: LlmApiConfigController;
  /** 不显示“API 端点”“API 地址”两行标签，用于和其他面板并排的窄布局（辅助打标） */
  compact?: boolean;
  /** 合并到面板根节点（.tool-panel）上 */
  style?: CSSProperties;
  /** 渲染在模型选择之后、同一面板内的表单行，如请求间隔、并发数、温度 */
  children?: ReactNode;
}

const LABEL_ICON: CSSProperties = { width: 13, height: 13, color: 'var(--color-text-tertiary)' };
const OK_COLOR = '#4ade80';
const FAIL_COLOR = '#f87171';

/** LLM API 设置面板：预设端点、自定义地址、API Key、模型选择，以及保存配置 */
export default function LlmApiPanel({ api, compact = false, style, children }: LlmApiPanelProps) {
  const { t } = useTranslation();
  const [showKey, setShowKey] = useState(false);
  const { saveResult, fetchResult } = api;

  const presetButtons: { id: LlmPresetId; label: string }[] = [
    ...LLM_PRESETS.map(p => ({ id: p.id, label: p.label })),
    { id: 'custom', label: t('llmApi.customLabel') },
  ];

  return (
    <div className="tool-panel" style={style}>
      <div className="tool-panel-header">
        <span className="tool-panel-title">{t('llmApi.apiSettings')}</span>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          {saveResult && (
            <span style={{ fontSize: 11, color: saveResult.ok ? OK_COLOR : FAIL_COLOR }}>
              {saveResult.ok ? `✓ ${t('llmApi.configSaved')}` : `✗ ${t('llmApi.saveFailed')}: ${saveResult.error}`}
            </span>
          )}
          <button className="btn btn-ghost btn-sm" onClick={() => { void api.saveConfig(); }}
            style={{ padding: '2px 8px', fontSize: 11, display: 'flex', alignItems: 'center', gap: 4 }}>
            <Save style={{ width: 12, height: 12 }} /> {t('llmApi.saveConfig')}
          </button>
        </div>
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 'var(--space-3)' }}>
        <div className="form-group" style={{ marginBottom: 0 }}>
          {!compact && (
            <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
              <Globe style={LABEL_ICON} /> {t('llmApi.apiEndpoint')}
            </label>
          )}
          <div style={{ display: 'flex', gap: 'var(--space-2)', flexWrap: 'wrap' }}>
            {presetButtons.map(p => (
              <button key={p.id} className={`btn btn-sm ${api.preset === p.id ? 'btn-primary' : 'btn-secondary'}`}
                onClick={() => api.setPreset(p.id)} style={{ flex: 1, fontSize: 11 }}>{p.label}</button>
            ))}
          </div>
          {api.preset === 'custom' ? (
            <>
              {!compact && (
                <div style={{ display: 'flex', alignItems: 'center', gap: 4, marginTop: 6 }}>
                  <span style={{ fontSize: 11, color: 'var(--color-text-secondary)' }}>{t('llmApi.apiAddress')}</span>
                  <span title={t('llmApi.openaiOnly')} style={{ cursor: 'help', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: 14, height: 14, borderRadius: '50%', fontSize: 9, fontWeight: 700, color: 'var(--color-text-tertiary)', border: '1px solid var(--color-border)' }}>?</span>
                </div>
              )}
              <input className="form-input" placeholder="https://api.example.com/v1/" value={api.customEndpoint}
                onChange={e => api.setCustomEndpoint(e.target.value)} style={{ marginTop: compact ? 6 : 4 }} />
              {api.customEndpoint && (
                <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 2, fontFamily: 'monospace', wordBreak: 'break-all' }}>
                  → {chatCompletionsUrl(api.customEndpoint)}
                </div>
              )}
            </>
          ) : (
            <div style={{ fontSize: 10, color: 'var(--color-text-tertiary)', marginTop: 4 }}>{api.endpoint}</div>
          )}
        </div>
        <div className="form-group" style={{ marginBottom: 0 }}>
          <label className="form-label" style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <Key style={LABEL_ICON} /> {t('llmApi.apiKey')}
          </label>
          <div style={{ position: 'relative' }}>
            <input className="form-input" type={showKey ? 'text' : 'password'} placeholder="sk-..." value={api.apiKey}
              onChange={e => api.setApiKey(e.target.value)} style={{ paddingRight: 32 }} />
            <button onClick={() => setShowKey(v => !v)}
              style={{ position: 'absolute', right: 6, top: '50%', transform: 'translateY(-50%)', background: 'none', border: 'none', cursor: 'pointer', color: 'var(--color-text-tertiary)', display: 'flex', padding: 2 }}>
              {showKey ? <EyeOff style={{ width: 14, height: 14 }} /> : <Eye style={{ width: 14, height: 14 }} />}
            </button>
          </div>
        </div>
        <div className="form-group" style={{ marginBottom: 0 }}>
          {/* 不用 <label>：label 会把点击转给内部第一个按钮，点“模型”二字就会触发获取模型列表 */}
          <div className="form-label" style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Bot style={LABEL_ICON} /> {t('llmApi.modelLabel')}</span>
            <button className="btn btn-ghost btn-sm" onClick={() => { void api.fetchModels(); }}
              disabled={api.fetchingModels || !api.endpoint} style={{ padding: '2px 8px', fontSize: 11 }}>
              {api.fetchingModels
                ? <Loader2 style={{ width: 12, height: 12, animation: 'spin 1s linear infinite' }} />
                : <RefreshCw style={{ width: 12, height: 12 }} />} {t('llmApi.fetchModels')}
            </button>
          </div>
          {api.modelList.length > 0 ? (
            <CustomSelect value={api.modelName} onChange={api.setModelName}
              options={api.modelList.map(m => ({ value: m, label: m }))} />
          ) : (
            <input className="form-input" placeholder={t('llmApi.modelPlaceholder')} value={api.modelName}
              onChange={e => api.setModelName(e.target.value)} />
          )}
          {fetchResult && (
            <div style={{ fontSize: 11, marginTop: 4, color: fetchResult.ok ? OK_COLOR : FAIL_COLOR }}>
              {fetchResult.ok ? `✓ ${t('llmApi.fetchOk', { n: fetchResult.count })}` : `✗ ${t('llmApi.fetchFail')}: ${fetchResult.error}`}
            </div>
          )}
        </div>
        {children}
      </div>
    </div>
  );
}
